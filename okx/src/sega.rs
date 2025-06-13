use anchor_lang::AccountDeserialize;
use anyhow::Context;
use async_trait::async_trait;
use lazy_static::lazy_static;
use log::{error, info};
use solana_account_decoder::UiAccountEncoding;
use solana_client::{
    rpc_client::RpcClient,
    rpc_config::{RpcAccountInfoConfig, RpcProgramAccountsConfig},
    rpc_filter::RpcFilterType,
};
use solana_sdk::{
    clock::Clock, commitment_config::CommitmentConfig, pubkey::Pubkey, signature::Keypair,
    signature::Signature, sysvar::Sysvar,
};
use spl_token_2022::extension::{
    transfer_fee::TransferFeeConfig, BaseStateWithExtensions, StateWithExtensionsOwned,
};
use spl_token_2022::state::{Account, Mint};
use std::sync::{Arc, Mutex};
use std::{collections::HashMap, error::Error, str::FromStr};
use tokio::sync::mpsc::Sender;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};

use super::{get_extra, Dex, PoolMetadata, PoolMetadataValue};

use sega_cp_swap::{AmmConfig, CurveCalculator, PoolState, PoolStatusBitIndex};

pub struct SegaCPMM;

lazy_static::lazy_static! {
    static ref TOKEN_MINT_MAP: Arc<Mutex<HashMap<String, StateWithExtensionsOwned<Mint>>>> = Arc::new(Mutex::new(HashMap::new()));
    static ref POOL_ADDRESS_MAP: Arc<Mutex<HashMap<String, PoolState>>> = Arc::new(Mutex::new(HashMap::new()));
}

impl SegaCPMM {
    fn find_pool_address_from_account(&self, account_address: &str) -> String {
        let pool_address_map = POOL_ADDRESS_MAP.lock().unwrap();
        let pool_state = pool_address_map.get(account_address);
        if pool_state.is_some() {
            account_address.to_string()
        } else {
            String::new()
        }
    }

    fn is_valid_pool_address(&self, client: &RpcClient, pool_address: &str) -> bool {
        let pool_state = self.derive_accounts_from_pool_address(client, pool_address);
        pool_state.is_some()
    }

    fn derive_accounts_from_pool_address(
        &self,
        client: &RpcClient,
        pool_address: &str,
    ) -> Option<PoolState> {
        let pool_address = Pubkey::from_str(pool_address).unwrap();
        if let Some(account) = client
            .get_account_with_commitment(&pool_address, CommitmentConfig::processed())
            .ok()?
            .value
        {
            let mut data: &[u8] = &account.data;
            let ret = PoolState::try_deserialize(&mut data).unwrap();
            Some(ret)
        } else {
            None
        }
    }
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
        let is_trading =
            get_extra!(metadata, "is_trading", PoolMetadataValue::Bool).unwrap_or(false);
        let epoch = Clock::get().unwrap().unix_timestamp;
        let open_time =
            get_extra!(metadata, "open_time", PoolMetadataValue::Number).unwrap_or(0.0) as i64;
        if !is_trading || epoch < open_time {
            return 0.0;
        }
        let token_0_transfer_fee = {
            let token_mint_map = TOKEN_MINT_MAP.lock().unwrap();
            let mint = token_mint_map.get(&metadata.base_mint).unwrap();
            if let Some(transfer_fee_config) = mint.get_extension::<TransferFeeConfig>().ok() {
                transfer_fee_config
                    .calculate_epoch_fee(epoch as u64, amount_in as u64)
                    .context("Fee 0 calculation failure")
                    .unwrap_or(0)
            } else {
                0
            }
        };
        let actual_amount_in = (amount_in as u64).saturating_sub(token_0_transfer_fee);
        if actual_amount_in == 0 {
            return 0.0;
        }
        let trade_fee_rate =
            get_extra!(metadata, "trade_fee_rate", PoolMetadataValue::Number).unwrap_or(0.0) as u64;
        let protocol_fee_rate = get_extra!(metadata, "protocol_fee_rate", PoolMetadataValue::Number)
            .unwrap_or(0.0) as u64;
        let fund_fee_rate =
            get_extra!(metadata, "fund_fee_rate", PoolMetadataValue::Number).unwrap_or(0.0) as u64;
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
                    .context("Fee 1 calculation failure")
                    .unwrap_or(0)
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
            },
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
        client: &RpcClient,
        address_tx: Sender<String>,
    ) -> Result<(), Box<dyn Error>> {
        // let program_id = self.dex_program_id();
        // let ws_url = "wss://api.mainnet-beta.solana.com";
        // let (mut ws_stream, _) = connect_async(ws_url).await?;
        // let subscribe_msg = format!(
        //     r#"{"jsonrpc":"2.0","id":1,"method":"logsSubscribe","params":["mentions","{}"]}"#,
        //     program_id
        // );
        // ws_stream.send(Message::Text(subscribe_msg)).await?;

        // while let Some(msg) = ws_stream.next().await {
        //     let msg = msg?;
        //     if let Message::Text(text) = msg {
        //         let log: serde_json::Value = serde_json::from_str(&text)?;
        //         if log.get("result").is_some() {
        //             continue;
        //         }

        //         let params = log.get("params").and_then(|p| p.get("result")).ok_or("No params")?;
        //         let tx_sig = params.get("signature").and_then(|s| s.as_str()).ok_or("No signature")?;
        //         let logs = params.get("logs").and_then(|l| l.as_array()).ok_or("No logs")?;

        //         let tx = client.get_transaction(
        //             &Signature::from_str(tx_sig)?,
        //             CommitmentConfig::confirmed(),
        //         )?;
        //         if tx.meta.is_some() && tx.meta.unwrap().err.is_some() {
        //             continue;
        //         }

        //         let log_str = logs.iter().filter_map(|l| l.as_str()).collect::<Vec<&str>>().join(" ");
        //         let account_keys = tx.transaction.message.account_keys;

        //         for (i, key) in account_keys.iter().enumerate() {
        //             if tx.transaction.message.is_writable(i) {
        //                 let pool_address = self.find_pool_address_from_account(&key.to_string());
        //                 if pool_address.is_empty() {
        //                     if let Some(pool_state) = self.derive_accounts_from_pool_address(client, &key.to_string()) {
        //                         POOL_ADDRESS_MAP.lock().unwrap().insert(key.to_string(), pool_state);
        //                         info!("Detected new {} pool address: {}", self.dex_name(), key);
        //                         address_tx.send(key.to_string()).await?;
        //                     }
        //                 } else if !pool_address.is_empty() {
        //                     info!("Detected writable account affecting pool: {}, Pool: {}", tx_sig, pool_address);
        //                     address_tx.send(pool_address).await?;
        //                 }
        //             }
        //         }

        //         if log_str.contains("initialize") {
        //             if let Some(pool_address) = self.extract_new_pool_address(client, tx_sig) {
        //                 if self.is_valid_pool_address(client, &pool_address) {
        //                     if let Some(accounts) = self.derive_accounts_from_pool_address(client, &pool_address) {
        //                         POOL_ADDRESS_MAP.lock().unwrap().insert(pool_address.clone(), accounts);
        //                         info!("Detected new {} pool address: {}", self.dex_name(), pool_address);
        //                         address_tx.send(pool_address).await?;
        //                     }
        //                 }
        //             }
        //         }
        //     }
        // }
        Ok(())
    }

    fn fetch_pool_metadata(&self, client: &RpcClient, pool_address: &str) -> Option<PoolMetadata> {
        let pool_address = Pubkey::from_str(pool_address).unwrap();
        let pool_state: Option<PoolState> = if let Some(account) = client
            .get_account_with_commitment(&pool_address, CommitmentConfig::processed())
            .ok()?
            .value
        {
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
        POOL_ADDRESS_MAP
            .lock()
            .unwrap()
            .insert(pool_address.to_string(), pool_state.clone());
        let amm_config: Option<AmmConfig> = if let Some(account) = client
            .get_account_with_commitment(&pool_state.amm_config, CommitmentConfig::processed())
            .ok()?
            .value
        {
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
            .get_account_with_commitment(&pool_state.token_0_mint, CommitmentConfig::processed())
            .ok()?
            .value
        {
            let data: &[u8] = &account.data;
            let ret =
                StateWithExtensionsOwned::<spl_token_2022::state::Mint>::unpack(data.to_vec())
                    .ok()?;
            Some(ret)
        } else {
            None
        };
        if token_0_mint.is_none() {
            return None;
        }
        let token_0_mint = token_0_mint.unwrap();
        let token_1_mint: Option<StateWithExtensionsOwned<Mint>> = if let Some(account) = client
            .get_account_with_commitment(&pool_state.token_1_mint, CommitmentConfig::processed())
            .ok()?
            .value
        {
            let data: &[u8] = &account.data;
            let ret =
                StateWithExtensionsOwned::<spl_token_2022::state::Mint>::unpack(data.to_vec())
                    .ok()?;
            Some(ret)
        } else {
            None
        };
        if token_1_mint.is_none() {
            return None;
        }
        let token_1_mint = token_1_mint.unwrap();

        TOKEN_MINT_MAP
            .lock()
            .unwrap()
            .insert(pool_state.token_0_mint.to_string(), token_0_mint);
        TOKEN_MINT_MAP
            .lock()
            .unwrap()
            .insert(pool_state.token_1_mint.to_string(), token_1_mint);

        let token_0_vault: Option<StateWithExtensionsOwned<Account>> = if let Some(account) = client
            .get_account_with_commitment(&pool_state.token_0_vault, CommitmentConfig::processed())
            .ok()?
            .value
        {
            let data: &[u8] = &account.data;
            let ret =
                StateWithExtensionsOwned::<spl_token_2022::state::Account>::unpack(data.to_vec())
                    .ok()?;
            Some(ret)
        } else {
            None
        };
        if token_0_vault.is_none() {
            return None;
        }
        let token_0_vault = token_0_vault.unwrap();
        let token_1_vault: Option<StateWithExtensionsOwned<Account>> = if let Some(account) = client
            .get_account_with_commitment(&pool_state.token_1_vault, CommitmentConfig::processed())
            .ok()?
            .value
        {
            let data: &[u8] = &account.data;
            let ret =
                StateWithExtensionsOwned::<spl_token_2022::state::Account>::unpack(data.to_vec())
                    .ok()?;
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
            .context("Vault 0 missing or frozen")
            .ok()?
            .checked_sub(pool_state.protocol_fees_token_0 + pool_state.fund_fees_token_0);
        let quote_reserve: Option<u64> = vault_1_amount
            .context("Vault 1 missing or frozen")
            .ok()?
            .checked_sub(pool_state.protocol_fees_token_1 + pool_state.fund_fees_token_1);

        let mut extra = HashMap::new();
        extra.insert(
            "is_trading".to_string(),
            PoolMetadataValue::Bool(pool_state.get_status_by_bit(PoolStatusBitIndex::Swap)),
        );
        extra.insert(
            "open_time".to_string(),
            PoolMetadataValue::Number(pool_state.open_time as f64),
        );
        extra.insert(
            "trade_fee_rate".to_string(),
            PoolMetadataValue::Number(amm_config.trade_fee_rate as f64),
        );
        extra.insert(
            "protocol_fee_rate".to_string(),
            PoolMetadataValue::Number(amm_config.protocol_fee_rate as f64),
        );
        extra.insert(
            "fund_fee_rate".to_string(),
            PoolMetadataValue::Number(amm_config.fund_fee_rate as f64),
        );
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
