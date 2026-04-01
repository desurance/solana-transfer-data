use anchor_lang::prelude::*;

declare_id!("Emw5eLRyfwQrKnqt3P1UA2WwBrGgBRqaUjEv2Pu3YKGc");

#[program]
#[allow(unused_variables)]
pub mod solana_data_transfer {
    use super::*;

    /// Upload the first block of a data transfer.
    ///
    /// Protocol layout (stored in transaction instruction data):
    ///   version(0001) | is_first(1) | total_size(32bit) | sha256_hash(256bit) | data
    ///
    /// The version and is_first flag are encoded implicitly by the instruction
    /// discriminator. All data is retrievable by knowing the last transaction
    /// signature and traversing the linked list backwards.
    pub fn upload_first(
        _ctx: Context<Upload>,
        total_size: u32,
        sha256_hash: [u8; 32],
        data: Vec<u8>,
    ) -> Result<()> {
        require!(total_size > 0, DataTransferError::InvalidDataSize);
        require!(!data.is_empty(), DataTransferError::EmptyData);
        require!(
            data.len() <= total_size as usize,
            DataTransferError::DataExceedsTotalSize
        );
        Ok(())
    }

    /// Upload a continuation block of a data transfer.
    ///
    /// Protocol layout (stored in transaction instruction data):
    ///   version(0001) | is_first(0) | prev_tx_signature(512bit) | data
    ///
    /// Each continuation block references the previous transaction signature,
    /// forming a reverse linked list back to the first block.
    pub fn upload_continuation(
        _ctx: Context<Upload>,
        prev_tx_sig: [u8; 64],
        data: Vec<u8>,
    ) -> Result<()> {
        require!(!data.is_empty(), DataTransferError::EmptyData);
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Upload<'info> {
    #[account(mut)]
    pub uploader: Signer<'info>,
}

#[error_code]
pub enum DataTransferError {
    #[msg("Data chunk cannot be empty")]
    EmptyData,
    #[msg("Total data size must be greater than zero")]
    InvalidDataSize,
    #[msg("Data chunk exceeds total data size")]
    DataExceedsTotalSize,
}
