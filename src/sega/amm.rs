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

use crate::okx::{get_extra, Dex, PoolMetadata, PoolMetadataValue};

pub struct SegaCPMM;

// #[async_trait]
// impl Dex for SegaCPMM {
//     fn dex_name(&self) -> String {
//         "sega".to_string()
//     }

//     fn dex_program_id(&self) -> Pubkey {
//         Pubkey::from_str("segaswap").unwrap()
//     }
    

// }