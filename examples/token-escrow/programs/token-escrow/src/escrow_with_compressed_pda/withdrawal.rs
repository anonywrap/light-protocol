use anchor_lang::prelude::*;
use light_compressed_pda::{
    invoke::processor::CompressedProof,
    sdk::{
        compressed_account::{
            CompressedAccount, CompressedAccountData, PackedCompressedAccountWithMerkleContext,
        },
        CompressedCpiContext,
    },
    InstructionDataInvokeCpi,
};
use light_compressed_token::{
    CompressedTokenInstructionDataTransfer, InputTokenDataWithContext, TokenTransferOutputData,
};
use light_hasher::{DataHasher, Poseidon};

use crate::{
    create_change_output_compressed_token_account, EscrowCompressedTokensWithCompressedPda,
    EscrowError, EscrowTimeLock, PackedInputCompressedPda,
};

pub fn process_withdraw_compressed_tokens_with_compressed_pda<'info>(
    ctx: Context<'_, '_, '_, 'info, EscrowCompressedTokensWithCompressedPda<'info>>,
    withdrawal_amount: u64,
    proof: CompressedProof,
    root_indices: Vec<u16>,
    mint: Pubkey,
    signer_is_delegate: bool,
    input_token_data_with_context: Vec<InputTokenDataWithContext>,
    output_state_merkle_tree_account_indices: Vec<u8>,
    cpi_context: CompressedCpiContext,
    input_compressed_pda: PackedInputCompressedPda,
    bump: u8,
) -> Result<()> {
    let current_slot = Clock::get()?.slot;
    if current_slot < input_compressed_pda.old_lock_up_time {
        return err!(EscrowError::EscrowLocked);
    }
    let (old_state, new_state) = create_compressed_pda_data_based_on_diff(&input_compressed_pda)?;
    let withdrawal_token_data = TokenTransferOutputData {
        amount: withdrawal_amount,
        owner: ctx.accounts.signer.key(),
        lamports: None,
    };
    let escrow_change_token_data = create_change_output_compressed_token_account(
        &input_token_data_with_context,
        &[withdrawal_token_data],
        &ctx.accounts.token_owner_pda.key(),
    );
    let output_compressed_accounts = vec![withdrawal_token_data, escrow_change_token_data];
    cpi_compressed_token_withdrawal(
        &ctx,
        mint,
        signer_is_delegate,
        input_token_data_with_context,
        output_compressed_accounts,
        output_state_merkle_tree_account_indices,
        vec![root_indices[1]],
        proof.clone(),
        bump,
        cpi_context,
    )?;

    cpi_compressed_pda_withdrawal(
        &ctx,
        proof,
        old_state,
        new_state,
        cpi_context,
        vec![root_indices[0]],
        bump,
    )?;
    Ok(())
}

fn create_compressed_pda_data_based_on_diff(
    input_compressed_pda: &PackedInputCompressedPda,
) -> Result<(PackedCompressedAccountWithMerkleContext, CompressedAccount)> {
    let current_slot = Clock::get()?.slot;

    let old_timelock_compressed_pda = EscrowTimeLock {
        slot: input_compressed_pda.old_lock_up_time,
    };
    let old_compressed_account_data = CompressedAccountData {
        discriminator: 1u64.to_le_bytes(),
        data: old_timelock_compressed_pda.try_to_vec().unwrap(),
        data_hash: old_timelock_compressed_pda
            .hash::<Poseidon>()
            .map_err(ProgramError::from)?,
    };
    let old_compressed_account = CompressedAccount {
        owner: crate::ID,
        lamports: 0,
        address: Some(input_compressed_pda.address),
        data: Some(old_compressed_account_data),
    };
    let old_compressed_account_with_context = PackedCompressedAccountWithMerkleContext {
        compressed_account: old_compressed_account,
        merkle_context: input_compressed_pda.merkle_context,
    };
    let new_timelock_compressed_pda = EscrowTimeLock {
        slot: current_slot
            .checked_add(input_compressed_pda.new_lock_up_time)
            .unwrap(),
    };
    let new_compressed_account_data = CompressedAccountData {
        discriminator: 1u64.to_le_bytes(),
        data: new_timelock_compressed_pda.try_to_vec().unwrap(),
        data_hash: new_timelock_compressed_pda
            .hash::<Poseidon>()
            .map_err(ProgramError::from)?,
    };
    let new_state = CompressedAccount {
        owner: crate::ID,
        lamports: 0,
        address: Some(input_compressed_pda.address),
        data: Some(new_compressed_account_data),
    };
    Ok((old_compressed_account_with_context, new_state))
}

fn cpi_compressed_pda_withdrawal<'info>(
    ctx: &Context<'_, '_, '_, 'info, EscrowCompressedTokensWithCompressedPda<'info>>,
    proof: CompressedProof,
    old_state: PackedCompressedAccountWithMerkleContext,
    compressed_pda: CompressedAccount,
    cpi_context: CompressedCpiContext,
    root_indices: Vec<u16>,
    bump: u8,
) -> Result<()> {
    let bump = &[bump];
    let signer_bytes = ctx.accounts.signer.key.to_bytes();
    let seeds: [&[u8]; 3] = [b"escrow".as_slice(), signer_bytes.as_slice(), bump];
    let inputs_struct = InstructionDataInvokeCpi {
        relay_fee: None,
        input_compressed_accounts_with_merkle_context: vec![old_state],
        output_compressed_accounts: vec![compressed_pda],
        input_root_indices: root_indices,
        output_state_merkle_tree_account_indices: vec![0],
        proof: Some(proof),
        new_address_params: Vec::new(),
        compression_lamports: None,
        is_compress: false,
        signer_seeds: seeds.iter().map(|seed| seed.to_vec()).collect(),
        cpi_context: Some(cpi_context),
    };

    let mut inputs = Vec::new();
    InstructionDataInvokeCpi::serialize(&inputs_struct, &mut inputs).unwrap();

    let cpi_context_account = match Some(cpi_context) {
        Some(cpi_context) => Some(
            ctx.remaining_accounts
                .get(cpi_context.cpi_context_account_index as usize)
                .unwrap()
                .to_account_info(),
        ),
        None => return err!(EscrowError::CpiContextAccountIndexNotFound),
    };

    let cpi_accounts = light_compressed_pda::cpi::accounts::InvokeCpiInstruction {
        fee_payer: ctx.accounts.signer.to_account_info(),
        authority: ctx.accounts.token_owner_pda.to_account_info(),
        registered_program_pda: ctx.accounts.registered_program_pda.to_account_info(),
        noop_program: ctx.accounts.noop_program.to_account_info(),
        account_compression_authority: ctx.accounts.account_compression_authority.to_account_info(),
        account_compression_program: ctx.accounts.account_compression_program.to_account_info(),
        invoking_program: ctx.accounts.self_program.to_account_info(),
        compressed_sol_pda: None,
        compression_recipient: None,
        system_program: ctx.accounts.system_program.to_account_info(),
        cpi_context_account,
    };
    let signer_seeds: [&[&[u8]]; 1] = [&seeds[..]];
    let mut cpi_ctx = CpiContext::new_with_signer(
        ctx.accounts.compressed_pda_program.to_account_info(),
        cpi_accounts,
        &signer_seeds,
    );
    cpi_ctx.remaining_accounts = ctx.remaining_accounts.to_vec();

    light_compressed_pda::cpi::invoke_cpi(cpi_ctx, inputs)?;
    Ok(())
}

#[inline(never)]
pub fn cpi_compressed_token_withdrawal<'info>(
    ctx: &Context<'_, '_, '_, 'info, EscrowCompressedTokensWithCompressedPda<'info>>,
    mint: Pubkey,
    signer_is_delegate: bool,
    input_token_data_with_context: Vec<InputTokenDataWithContext>,
    output_compressed_accounts: Vec<TokenTransferOutputData>,
    output_state_merkle_tree_account_indices: Vec<u8>,
    root_indices: Vec<u16>,
    proof: CompressedProof,
    bump: u8,
    mut cpi_context: CompressedCpiContext,
) -> Result<()> {
    let bump = &[bump];
    let signer_bytes = ctx.accounts.signer.key.to_bytes();
    let seeds: [&[u8]; 3] = [b"escrow".as_slice(), signer_bytes.as_slice(), bump];
    cpi_context.set_context = true;

    let inputs_struct = CompressedTokenInstructionDataTransfer {
        proof: Some(proof),
        root_indices,
        mint,
        signer_is_delegate,
        input_token_data_with_context,
        output_compressed_accounts,
        output_state_merkle_tree_account_indices,
        is_compress: false,
        compression_amount: None,
        cpi_context: Some(cpi_context),
    };

    let mut inputs = Vec::new();
    CompressedTokenInstructionDataTransfer::serialize(&inputs_struct, &mut inputs).unwrap();

    let cpi_accounts = light_compressed_token::cpi::accounts::TransferInstruction {
        fee_payer: ctx.accounts.signer.to_account_info(),
        authority: ctx.accounts.token_owner_pda.to_account_info(),
        registered_program_pda: ctx.accounts.registered_program_pda.to_account_info(),
        noop_program: ctx.accounts.noop_program.to_account_info(),
        account_compression_authority: ctx.accounts.account_compression_authority.to_account_info(),
        account_compression_program: ctx.accounts.account_compression_program.to_account_info(),
        self_program: ctx.accounts.compressed_token_program.to_account_info(),
        cpi_authority_pda: ctx
            .accounts
            .compressed_token_cpi_authority_pda
            .to_account_info(),
        compressed_pda_program: ctx.accounts.compressed_pda_program.to_account_info(),
        token_pool_pda: None,
        decompress_token_account: None,
        token_program: None,
        system_program: ctx.accounts.system_program.to_account_info(),
    };
    let signer_seeds: [&[&[u8]]; 1] = [&seeds[..]];

    let mut cpi_ctx = CpiContext::new_with_signer(
        ctx.accounts.compressed_token_program.to_account_info(),
        cpi_accounts,
        &signer_seeds,
    );

    cpi_ctx.remaining_accounts = ctx.remaining_accounts.to_vec();
    light_compressed_token::cpi::transfer(cpi_ctx, inputs)?;
    Ok(())
}