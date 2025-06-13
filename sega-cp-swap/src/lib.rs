pub mod amm_config;
pub mod calculator;
pub mod constant_product;
pub mod fees;
mod math;
pub mod pool;
pub mod swap;

pub use amm_config::*;
pub use calculator::*;
pub use constant_product::*;
pub use fees::*;
pub use math::*;
pub use pool::*;
pub use swap::*;

use anchor_lang::prelude::*;

pub const AUTH_SEED: &str = "vault_and_lp_mint_auth_seed";

declare_id!("SegaXNnoXYTZiqUt9Xn2XqGcL56b25yzXLuJSpadcMu");
