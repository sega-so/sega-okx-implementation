use anchor_lang::prelude::{AccountMeta, Pubkey, ToAccountMetas};

#[derive(Copy, Clone, Debug)]
pub struct SegaSwap {
    pub program: Pubkey,
    pub payer: Pubkey,
    pub authority: Pubkey,
    pub amm_config: Pubkey,
    pub pool_state: Pubkey,
    pub input_token_account: Pubkey,
    pub output_token_account: Pubkey,
    pub input_vault: Pubkey,
    pub output_vault: Pubkey,
    pub input_token_program: Pubkey,
    pub output_token_program: Pubkey,
    pub input_token_mint: Pubkey,
    pub output_token_mint: Pubkey,
    pub observation_state: Pubkey,
}

impl ToAccountMetas for SegaSwap {
    fn to_account_metas(&self, _is_signer: Option<bool>) -> Vec<AccountMeta> {
        vec![
            AccountMeta::new(self.program, false),
            AccountMeta::new(self.payer, false),
            AccountMeta::new_readonly(self.authority, false),
            AccountMeta::new_readonly(self.amm_config, false),
            AccountMeta::new(self.pool_state, false),
            AccountMeta::new(self.input_token_account, false),
            AccountMeta::new(self.output_token_account, false),
            AccountMeta::new(self.input_vault, false),
            AccountMeta::new(self.output_vault, false),
            AccountMeta::new_readonly(self.input_token_program, false),
            AccountMeta::new_readonly(self.output_token_program, false),
            AccountMeta::new_readonly(self.input_token_mint, false),
            AccountMeta::new_readonly(self.output_token_mint, false),
            AccountMeta::new(self.observation_state, false),
        ]
    }
}
