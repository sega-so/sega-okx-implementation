use anchor_lang::AccountDeserialize;
use anyhow::Context;
use async_trait::async_trait;
use log::error;
use solana_client::{
    rpc_client::RpcClient,
    rpc_config::{RpcAccountInfoConfig, RpcProgramAccountsConfig},
    rpc_filter::RpcFilterType,
};
use solana_account_decoder::UiAccountEncoding;
use solana_sdk::{pubkey::Pubkey, commitment_config::CommitmentConfig, clock::Clock, sysvar::Sysvar};
use std::{collections::HashMap, error::Error, str::FromStr};
use spl_token_2022::extension::{StateWithExtensionsOwned, transfer_fee::TransferFeeConfig, BaseStateWithExtensions};
use spl_token_2022::state::{Mint, Account};
use lazy_static::lazy_static;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::Sender;

use super::{get_extra, Dex, PoolMetadata, PoolMetadataValue};

use sega_cp_swap::{AmmConfig, CurveCalculator, PoolState, PoolStatusBitIndex};

pub struct SegaCPMM;

lazy_static::lazy_static! {
    static ref TOKEN_MINT_MAP: Arc<Mutex<HashMap<String, StateWithExtensionsOwned<Mint>>>> = Arc::new(Mutex::new(HashMap::new()));
}

#[async_trait]
impl Dex for SegaCPMM {
    fn dex_name(&self) -> String {
        "Sega".to_string()
    }

    fn dex_program_id(&self) -> Pubkey {
        sega_cp_swap::ID
    }

    fn quote(&self, amount_in: f64, metadata: &PoolMetadata) -> f64 {
        if amount_in <= 0.0 {
            return 0.0;
        }
        let is_trading = get_extra!(metadata, "is_trading", PoolMetadataValue::Bool)
            .unwrap_or(false);
        let epoch = Clock::get().unwrap().unix_timestamp;
        let open_time = get_extra!(metadata, "open_time", PoolMetadataValue::Number)
            .unwrap_or(0.0) as i64;
        if !is_trading || epoch < open_time {
            return 0.0;
        }
        let token_0_transfer_fee = {
            let token_mint_map = TOKEN_MINT_MAP.lock().unwrap();
            let mint = token_mint_map.get(&metadata.base_mint).unwrap();
            if let Some(transfer_fee_config) = mint.get_extension::<TransferFeeConfig>().ok() {
                transfer_fee_config
                    .calculate_epoch_fee(epoch as u64, amount_in as u64)
                    .context("Fee 0 calculation failure").unwrap_or(0)
            } else {
                0
            }
        };
        let actual_amount_in = (amount_in as u64).saturating_sub(token_0_transfer_fee);
        if actual_amount_in == 0 {
            return 0.0;
        }
        let trade_fee_rate = get_extra!(metadata, "trade_fee_rate", PoolMetadataValue::Number)
            .unwrap_or(0.0) as u64;
        let protocol_fee_rate = get_extra!(metadata, "protocol_fee_rate", PoolMetadataValue::Number)
            .unwrap_or(0.0) as u64;
        let fund_fee_rate = get_extra!(metadata, "fund_fee_rate", PoolMetadataValue::Number)
            .unwrap_or(0.0) as u64;
        let total_token_0_amount = metadata.base_reserve.unwrap_or(0.0) as u128;
        let total_token_1_amount = metadata.quote_reserve.unwrap_or(0.0) as u128;
        let swap_result = CurveCalculator::swap_base_input(
            u128::from(actual_amount_in),
            total_token_0_amount,
            total_token_1_amount,
            trade_fee_rate,
            protocol_fee_rate,
            fund_fee_rate,
        );
        if swap_result.is_none() {
            return 0.0;
        }
        let swap_result = swap_result.unwrap();
        let amount_out = swap_result.destination_amount_swapped as u64;
        let token_1_transfer_fee = {
            let token_mint_map = TOKEN_MINT_MAP.lock().unwrap();
            let mint = token_mint_map.get(&metadata.quote_mint).unwrap();
            if let Some(transfer_fee_config) = mint.get_extension::<TransferFeeConfig>().ok() {
                transfer_fee_config
                    .calculate_epoch_fee(epoch as u64, amount_out as u64)
                    .context("Fee 1 calculation failure").unwrap_or(0)
            } else {
                0
            }
        };
        let actual_amount_out = amount_out.saturating_sub(token_1_transfer_fee);
        actual_amount_out as f64
    }

    fn fetch_pool_addresses(&self, client: &RpcClient) -> Vec<String> {
        let pool_len = sega_cp_swap::PoolState::LEN as u64;
        let filters = Some(vec![RpcFilterType::DataSize(pool_len)]);
        let account_config = RpcAccountInfoConfig {
            encoding: Some(UiAccountEncoding::Base64Zstd),
            ..RpcAccountInfoConfig::default()
        };
        let accounts = match client.get_program_accounts_with_config(
            &self.dex_program_id().to_bytes().into(),
            RpcProgramAccountsConfig {
                filters,
                account_config,
                ..RpcProgramAccountsConfig::default()
            }
        ) {
            Ok(accs) => accs,
            Err(e) => {
                error!("Failed to fetch {} pool addresses: {}", self.dex_name(), e);
                return Vec::new();
            }
        };
        accounts
            .into_iter()
            .map(|(pk, _acct)| pk.to_string())
            .collect()
    }

    async fn listen_new_pool_addresses(
        &self,
        _client: &RpcClient,
        _address_tx: Sender<String>,
    ) -> Result<(), Box<dyn Error>> {
        Ok(())
    }

    fn fetch_pool_metadata(&self, client: &RpcClient, pool_address: &str) -> Option<PoolMetadata> {
        let pool_address = Pubkey::from_str(pool_address).unwrap();
        let pool_state: Option<PoolState> = if let Some(account) = client
            .get_account_with_commitment(&pool_address, CommitmentConfig::processed()).ok()?
            .value {
                let mut data: &[u8] = &account.data;
                let ret = PoolState::try_deserialize(&mut data).unwrap();
                Some(ret)
            } else {
                None
            };
        if pool_state.is_none() {
            return None;
        }
        let pool_state = pool_state.unwrap();
        let amm_config: Option<AmmConfig> = if let Some(account) = client
            .get_account_with_commitment(&pool_state.amm_config, CommitmentConfig::processed()).ok()?
            .value {
                let mut data: &[u8] = &account.data;
                let ret = AmmConfig::try_deserialize(&mut data).unwrap();
                Some(ret)
            } else {
                None
            };
        if amm_config.is_none() {
            return None;
        }
        let amm_config = amm_config.unwrap();
        let token_0_mint: Option<StateWithExtensionsOwned<Mint>> = if let Some(account) = client
            .get_account_with_commitment(&pool_state.token_0_mint, CommitmentConfig::processed()).ok()?
            .value {
                let data: &[u8] = &account.data;
                let ret = StateWithExtensionsOwned::<spl_token_2022::state::Mint>::unpack(data.to_vec()).ok()?;
                Some(ret)
            } else {
                None
            };
        if token_0_mint.is_none() {
            return None;
        }
        let token_0_mint = token_0_mint.unwrap();
        let token_1_mint: Option<StateWithExtensionsOwned<Mint>> = if let Some(account) = client
            .get_account_with_commitment(&pool_state.token_1_mint, CommitmentConfig::processed()).ok()?
            .value {
                let data: &[u8] = &account.data;
                let ret = StateWithExtensionsOwned::<spl_token_2022::state::Mint>::unpack(data.to_vec()).ok()?;
                Some(ret)
            } else {
                None
            };
        if token_1_mint.is_none() {
            return None;
        }
        let token_1_mint = token_1_mint.unwrap();

        TOKEN_MINT_MAP.lock().unwrap().insert(pool_state.token_0_mint.to_string(), token_0_mint);
        TOKEN_MINT_MAP.lock().unwrap().insert(pool_state.token_1_mint.to_string(), token_1_mint);

        let token_0_vault: Option<StateWithExtensionsOwned<Account>> = if let Some(account) = client
            .get_account_with_commitment(&pool_state.token_0_vault, CommitmentConfig::processed()).ok()?
            .value {
                let data: &[u8] = &account.data;
                let ret = StateWithExtensionsOwned::<spl_token_2022::state::Account>::unpack(data.to_vec()).ok()?;
                Some(ret)
            } else {
                None
            };
        if token_0_vault.is_none() {
            return None;
        }
        let token_0_vault = token_0_vault.unwrap();
        let token_1_vault: Option<StateWithExtensionsOwned<Account>> = if let Some(account) = client
            .get_account_with_commitment(&pool_state.token_1_vault, CommitmentConfig::processed()).ok()?
            .value {
                let data: &[u8] = &account.data;
                let ret = StateWithExtensionsOwned::<spl_token_2022::state::Account>::unpack(data.to_vec()).ok()?;
                Some(ret)
            } else {
                None
            };
        if token_1_vault.is_none() {
            return None;
        }
        let token_1_vault = token_1_vault.unwrap();

        let vault_0_amount = if token_0_vault.base.is_frozen() {
            None
        } else {
            Some(token_0_vault.base.amount)
        };
        let vault_1_amount = if token_1_vault.base.is_frozen() {
            None
        } else {
            Some(token_1_vault.base.amount)
        };

        let base_reserve: Option<u64> = vault_0_amount
            .context("Vault 0 missing or frozen").ok()?
            .checked_sub(pool_state.protocol_fees_token_0 + pool_state.fund_fees_token_0);
        let quote_reserve: Option<u64> = vault_1_amount
            .context("Vault 1 missing or frozen").ok()?
            .checked_sub(pool_state.protocol_fees_token_1 + pool_state.fund_fees_token_1);

        let mut extra = HashMap::new();
        extra.insert("is_trading".to_string(), PoolMetadataValue::Bool(pool_state.get_status_by_bit(PoolStatusBitIndex::Swap)));
        extra.insert("open_time".to_string(), PoolMetadataValue::Number(pool_state.open_time as f64));
        extra.insert("trade_fee_rate".to_string(), PoolMetadataValue::Number(amm_config.trade_fee_rate as f64));
        extra.insert("protocol_fee_rate".to_string(), PoolMetadataValue::Number(amm_config.protocol_fee_rate as f64));
        extra.insert("fund_fee_rate".to_string(), PoolMetadataValue::Number(amm_config.fund_fee_rate as f64));
        Some(PoolMetadata {
            extra,
            base_reserve: base_reserve.map(|v| v as f64),
            quote_reserve: quote_reserve.map(|v| v as f64),
            pool_address: pool_address.to_string(),
            base_mint: pool_state.token_0_mint.to_string(),
            quote_mint: pool_state.token_1_mint.to_string(),
            trade_fee: None,
        })
    }
}