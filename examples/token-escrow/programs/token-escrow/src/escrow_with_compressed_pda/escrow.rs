use crate::{create_change_output_compressed_token_account, program::TokenEscrow, EscrowTimeLock};
use account_compression::{program::AccountCompression, RegisteredProgram};
use anchor_lang::prelude::*;
use anchor_spl::{token_2022::TransferChecked, token_interface::TokenInterface};
use light_compressed_token::{
    process_transfer::{
        CompressedTokenInstructionDataTransfer, InputTokenDataWithContext,
        PackedTokenTransferOutputData,
    },accounts::MintToInstruction,
    program::LightCompressedToken,
};
use anchor_lang::solana_program::program_pack::Pack;
use light_hasher::{errors::HasherError, DataHasher, Hasher, Poseidon};
use light_sdk::{
    light_accounts, utils::create_cpi_inputs_for_new_address, verify::verify, LightTraits,
};
use light_system_program::{
    invoke::processor::CompressedProof,
    invoke_cpi::account::CpiContextAccount,
    program::LightSystemProgram,
    sdk::{
        address::derive_address,
        compressed_account::{CompressedAccount, CompressedAccountData, PackedMerkleContext},
        CompressedCpiContext,
    },
    NewAddressParamsPacked, OutputCompressedAccountWithPackedContext,
};

use light_sdk::traits::*;

#[light_accounts]
#[derive(Accounts, LightTraits)]
pub struct EscrowCompressedTokensWithCompressedPda<'info> {
    #[account(mut)]
    #[fee_payer]
    pub signer: Signer<'info>,
    /// CHECK:
    #[authority]
    #[account(seeds = [b"escrow".as_slice(), signer.key.to_bytes().as_slice()], bump)]
    pub token_owner_pda: AccountInfo<'info>,
    pub compressed_token_program: Program<'info, LightCompressedToken>,
    pub compressed_token_cpi_authority_pda: AccountInfo<'info>,
    #[self_program]
    pub self_program: Program<'info, TokenEscrow>,
    /// CHECK:
    #[cpi_context]
    #[account(mut)]
    pub cpi_context_account: Account<'info, CpiContextAccount>,

    pub unwrapped_mint: Box<InterfaceAccount<'info, anchor_spl::token_interface::Mint>>,
    #[account(mut)]
    pub unwrapped_token_account: Box<InterfaceAccount<'info, anchor_spl::token_interface::TokenAccount>>,
    /// CHECK: 
    #[account(init_if_needed, payer = signer, space = 8 + 32,
        seeds = [b"backpointa", unwrapped_mint.key().as_ref(), token_program.key().as_ref()],
        bump
    )]
    pub wrapped_mint_backpointer: Account<'info, Backpointer>,
    /// CHECK: 
    pub token_program: Interface<'info, TokenInterface>,
    #[account(init_if_needed, payer = signer, associated_token::authority = wrapped_mint_backpointer, associated_token::mint = unwrapped_mint)]
    pub escrow: Box<InterfaceAccount<'info, anchor_spl::token_interface::TokenAccount>>,
    /// CHECK:
    pub associated_token_program: UncheckedAccount<'info>
}

#[account]
pub struct Backpointer {
    pub wrapped_mint: Pubkey,
}
#[derive(Debug, Clone, AnchorSerialize, AnchorDeserialize)]
pub struct PackedInputCompressedPda {
    pub old_lock_up_time: u64,
    pub new_lock_up_time: u64,
    pub address: [u8; 32],
    pub merkle_context: PackedMerkleContext,
    pub root_index: u16,
}

/// create compressed pda data
/// transfer tokens
/// execute complete transaction
pub fn process_escrow_compressed_tokens_with_compressed_pda<'info>(
    ctx: Context<'_, '_, '_, 'info, EscrowCompressedTokensWithCompressedPda<'info>>,
    lock_up_time: u64,
    escrow_amount: u64,
    proof: CompressedProof,
    mint: Pubkey,
    signer_is_delegate: bool,
    input_token_data_with_context: Vec<InputTokenDataWithContext>,
    output_state_merkle_tree_account_indices: Vec<u8>,
    new_address_params: NewAddressParamsPacked,
    cpi_context: CompressedCpiContext,
    bump: u8,
) -> Result<()> {
    
    let compressed_pda = create_compressed_pda_data(lock_up_time, &ctx, &new_address_params)?;
    let escrow_token_data = PackedTokenTransferOutputData {
        amount: escrow_amount,
        owner: ctx.accounts.token_owner_pda.key(),
        lamports: None,
        merkle_tree_index: output_state_merkle_tree_account_indices[0],
    };
    let change_token_data = create_change_output_compressed_token_account(
        &input_token_data_with_context,
        &[escrow_token_data],
        &ctx.accounts.signer.key(),
        output_state_merkle_tree_account_indices[1],
    );
    let output_compressed_accounts = vec![escrow_token_data, change_token_data];
    let backpointer = &mut ctx.accounts.wrapped_mint_backpointer;
    backpointer.wrapped_mint = mint;
    let ai = ctx.accounts.unwrapped_mint.to_account_info();
    let unwrapped_mint_data = &ai.try_borrow_data()?;
    let unwrapped_mint = spl_token_2022::state::Mint::unpack(&unwrapped_mint_data)?;

    // Now, you can access the `decimals` field of the mint
    let decimals = unwrapped_mint.decimals;
    let seeds = &[b"backpointa", ctx.accounts.unwrapped_mint.to_account_info().key.as_ref(), ctx.accounts.token_program.to_account_info().key.as_ref(), &[ctx.bumps.wrapped_mint_backpointer]];
    let signer = &[&seeds[..]];

    // Proceed with the wrapping logic, ensuring the amounts are scaled correctly according to the decimals
    // Example: Transfer tokens from the user's account to the escrow
    let transfer_to_escrow_cpi_accounts = TransferChecked {
        from: ctx.accounts.unwrapped_token_account.to_account_info(),
        to: ctx.accounts.escrow.to_account_info(),
        authority: ctx.accounts.signer.to_account_info(),
        mint: ctx.accounts.unwrapped_mint.to_account_info(),
    };
    let transfer_to_escrow_cpi_context = CpiContext::new_with_signer(
        ctx.accounts.token_program.to_account_info(),
        transfer_to_escrow_cpi_accounts,
        signer,
    );
    anchor_spl::token_interface::transfer_checked(transfer_to_escrow_cpi_context, escrow_amount, decimals)?;

    cpi_compressed_token_transfer_pda(
        &ctx,
        mint,
        signer_is_delegate,
        input_token_data_with_context,
        output_compressed_accounts,
        proof.clone(),
        cpi_context,
        escrow_amount
    )?;
    cpi_compressed_pda_transfer(
        ctx,
        proof,
        new_address_params,
        compressed_pda,
        cpi_context,
        bump,
    )?;
    Ok(())
}

fn cpi_compressed_pda_transfer<'info>(
    ctx: Context<'_, '_, '_, 'info, EscrowCompressedTokensWithCompressedPda<'info>>,
    proof: CompressedProof,
    new_address_params: NewAddressParamsPacked,
    compressed_pda: OutputCompressedAccountWithPackedContext,
    mut cpi_context: CompressedCpiContext,
    bump: u8,
) -> Result<()> {
    // Create CPI signer seed
    let bump_seed = &[bump];
    let signer_key_bytes = ctx.accounts.signer.key.to_bytes();
    let signer_seeds = [&b"escrow"[..], &signer_key_bytes[..], bump_seed];
    cpi_context.first_set_context = false;
    // Create inputs struct
    let inputs_struct = create_cpi_inputs_for_new_address(
        proof,
        new_address_params,
        compressed_pda,
        &signer_seeds,
        Some(cpi_context),
    );

    verify(ctx, &inputs_struct, &[&signer_seeds])?;

    Ok(())
}

fn create_compressed_pda_data(
    lock_up_time: u64,
    ctx: &Context<'_, '_, '_, '_, EscrowCompressedTokensWithCompressedPda<'_>>,
    new_address_params: &NewAddressParamsPacked,
) -> Result<OutputCompressedAccountWithPackedContext> {
    let current_slot = Clock::get()?.slot;
    let timelock_compressed_pda = EscrowTimeLock {
        slot: current_slot.checked_add(lock_up_time).unwrap(),
    };
    let compressed_account_data = CompressedAccountData {
        discriminator: 1u64.to_le_bytes(),
        data: timelock_compressed_pda.try_to_vec().unwrap(),
        data_hash: timelock_compressed_pda
            .hash::<Poseidon>()
            .map_err(ProgramError::from)?,
    };
    let derive_address = derive_address(
        &ctx.remaining_accounts[new_address_params.address_merkle_tree_account_index as usize]
            .key(),
        &new_address_params.seed,
    )
    .map_err(|_| ProgramError::InvalidArgument)?;
    Ok(OutputCompressedAccountWithPackedContext {
        compressed_account: CompressedAccount {
            owner: crate::ID,
            lamports: 0,
            address: Some(derive_address),
            data: Some(compressed_account_data),
        },
        merkle_tree_index: 0,
    })
}

impl light_hasher::DataHasher for EscrowTimeLock {
    fn hash<H: Hasher>(&self) -> std::result::Result<[u8; 32], HasherError> {
        H::hash(&self.slot.to_le_bytes())
    }
}

#[inline(never)]
pub fn cpi_compressed_token_transfer_pda<'info>(
    ctx: &Context<'_, '_, '_, 'info, EscrowCompressedTokensWithCompressedPda<'info>>,
    mint: Pubkey,
    _signer_is_delegate: bool,
    input_token_data_with_context: Vec<InputTokenDataWithContext>,
    output_compressed_accounts: Vec<PackedTokenTransferOutputData>,
    proof: CompressedProof,
    mut cpi_context: CompressedCpiContext,
    escrow_amount: u64
) -> Result<()> {
    cpi_context.set_context = true;
 
    let inputs_struct = MintToInstruction {
        fee_payer: *ctx.accounts.signer.to_account_info().key,
        authority: *ctx.accounts.signer.to_account_info().key,
        cpi_authority_pda: *ctx.accounts.compressed_token_cpi_authority_pda.to_account_info().key,
        mint: *ctx.accounts.unwrapped_mint.to_account_info().key,
        token_pool_pda: *ctx.accounts.token_owner_pda.to_account_info().key,
        token_program: *ctx.accounts.token_program.to_account_info().key,
        light_system_program: *ctx.accounts.light_system_program.to_account_info().key,
        registered_program_pda: *ctx.accounts.registered_program_pda.to_account_info().key,
        noop_program: *ctx.accounts.noop_program.to_account_info().key,
        account_compression_authority: *ctx.accounts.account_compression_authority.to_account_info().key,
        account_compression_program: *ctx.accounts.account_compression_program.to_account_info().key,
        merkle_tree: *ctx.remaining_accounts[0].to_account_info().key,
        self_program: *ctx.accounts.compressed_token_program.to_account_info().key,
        system_program: *ctx.accounts.system_program.to_account_info().key,
    };

    let inputs = vec![
        *ctx.accounts.signer.to_account_info().key,
        *ctx.accounts.compressed_token_cpi_authority_pda.to_account_info().key,
        *ctx.accounts.unwrapped_mint.to_account_info().key,
        *ctx.accounts.token_owner_pda.to_account_info().key,
        *ctx.accounts.token_program.to_account_info().key,
        *ctx.accounts.light_system_program.to_account_info().key,
        *ctx.accounts.registered_program_pda.to_account_info().key,
        *ctx.accounts.noop_program.to_account_info().key,
        *ctx.accounts.account_compression_authority.to_account_info().key,
        *ctx.accounts.account_compression_program.to_account_info().key,
        *ctx.remaining_accounts[0].to_account_info().key,
        *ctx.accounts.compressed_token_program.to_account_info().key,
        *ctx.accounts.system_program.to_account_info().key,
    ];

    let cpi_accounts = light_compressed_token::cpi::accounts::MintToInstruction {
        fee_payer: ctx.accounts.signer.to_account_info(),
        authority: ctx.accounts.signer.to_account_info(),
        merkle_tree: ctx.remaining_accounts[0].to_account_info(),
        mint: ctx.accounts.unwrapped_mint.to_account_info(),
        token_program: ctx.accounts.token_program.to_account_info(),
        registered_program_pda: ctx.accounts.registered_program_pda.to_account_info(),
        noop_program: ctx.accounts.noop_program.to_account_info(),
        account_compression_authority: ctx.accounts.account_compression_authority.to_account_info(),
        account_compression_program: ctx.accounts.account_compression_program.to_account_info(),
        self_program: ctx.accounts.compressed_token_program.to_account_info(),
        cpi_authority_pda: ctx
            .accounts
            .compressed_token_cpi_authority_pda
            .to_account_info(),
        light_system_program: ctx.accounts.light_system_program.to_account_info(),
        token_pool_pda: ctx.accounts.token_owner_pda.to_account_info(),
        system_program: ctx.accounts.system_program.to_account_info(),
    };

    let mut cpi_ctx = CpiContext::new(
        ctx.accounts.compressed_token_program.to_account_info(),
        cpi_accounts,
    );

    cpi_ctx.remaining_accounts = ctx.remaining_accounts.to_vec();

    light_compressed_token::cpi::mint_to(cpi_ctx, inputs, vec![escrow_amount])?;
    Ok(())
}
