use async_trait::async_trait;
use log::error;
use solana_client::{
    rpc_client::RpcClient,
    rpc_config::{RpcAccountInfoConfig, RpcProgramAccountsConfig},
    rpc_filter::{Memcmp, RpcFilterType},
};
use solana_sdk::pubkey::Pubkey;
use std::{collections::HashMap, error::Error, str::FromStr};
use tokio::sync::mpsc::Sender;

use super::{get_extra, Dex, PoolMetadata, PoolMetadataValue};

use sega_cp_swap::{PoolState, SegaSwap};

pub struct SegaCPMM;

#[async_trait]
impl Dex for SegaCPMM {
    fn dex_name(&self) -> String {
        "sega".to_string()
    }

    fn dex_program_id(&self) -> Pubkey {
        sega_cp_swap::ID
    }

    fn quote(&self, amount_in: f64, metadata: &PoolMetadata) -> f64 {
        todo!()
    }

    fn fetch_pool_addresses(&self, client: &RpcClient) -> Vec<String> {
        todo!()
    }

    async fn listen_new_pool_addresses(
        &self,
        _client: &RpcClient,
        _address_tx: Sender<String>,
    ) -> Result<(), Box<dyn Error>> {
        Ok(())
    }

    fn fetch_pool_metadata(&self, client: &RpcClient, pool_address: &str) -> Option<PoolMetadata> {
        todo!()
    }
}