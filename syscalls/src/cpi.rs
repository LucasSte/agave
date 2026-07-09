use {
    super::*,
    solana_instruction::Instruction,
    solana_program_runtime::cpi::{
<<<<<<< HEAD
        SolInstruction, SyscallInvokeSigned, TranslatedAccount, cpi_common, translate_accounts_c,
        translate_accounts_rust, translate_accounts_v2, translate_instruction_c,
        translate_instruction_rust, translate_instruction_v2,
=======
        SyscallInvokeSigned, TranslatedAccount, cpi_common, translate_accounts_c,
        translate_accounts_rust, translate_instruction_c, translate_instruction_rust,
        translate_signers,
>>>>>>> 305570cac3 (ABIv2 => ABIv2 CPI (#10))
    },
};

/// Cross-program invocation called from Rust
pub struct SyscallInvokeSignedRust {}
impl BuiltinFunctionDefinition<InvokeContext<'_, '_>> for SyscallInvokeSignedRust {
    type Error = Error;
    fn rust(
        invoke_context: &mut InvokeContext<'_, '_>,
        instruction_addr: u64,
        account_infos_addr: u64,
        account_infos_len: u64,
        signers_seeds_addr: u64,
        signers_seeds_len: u64,
    ) -> Result<u64, Error> {
        cpi_common::<Self>(
            invoke_context,
            instruction_addr,
            account_infos_addr,
            account_infos_len,
            signers_seeds_addr,
            signers_seeds_len,
        )
    }
}

impl SyscallInvokeSigned for SyscallInvokeSignedRust {
    fn translate_instruction(
        addr: u64,
        invoke_context: &InvokeContext,
    ) -> Result<Instruction, Error> {
        translate_instruction_rust(addr, invoke_context)
    }

    fn translate_accounts<'a>(
        account_infos_addr: u64,
        account_infos_len: u64,
        invoke_context: &InvokeContext,
    ) -> Result<Vec<TranslatedAccount<'a>>, Error> {
        translate_accounts_rust(account_infos_addr, account_infos_len, invoke_context)
    }
}

/// Cross-program invocation called from C
pub struct SyscallInvokeSignedC {}
impl BuiltinFunctionDefinition<InvokeContext<'_, '_>> for SyscallInvokeSignedC {
    type Error = Error;
    fn rust(
        invoke_context: &mut InvokeContext<'_, '_>,
        instruction_addr: u64,
        account_infos_addr: u64,
        account_infos_len: u64,
        signers_seeds_addr: u64,
        signers_seeds_len: u64,
    ) -> Result<u64, Error> {
        cpi_common::<Self>(
            invoke_context,
            instruction_addr,
            account_infos_addr,
            account_infos_len,
            signers_seeds_addr,
            signers_seeds_len,
        )
    }
}

impl SyscallInvokeSigned for SyscallInvokeSignedC {
    fn translate_instruction(
        addr: u64,
        invoke_context: &InvokeContext,
    ) -> Result<Instruction, Error> {
        translate_instruction_c(addr, invoke_context)
    }

    fn translate_accounts<'a>(
        account_infos_addr: u64,
        account_infos_len: u64,
        invoke_context: &InvokeContext,
    ) -> Result<Vec<TranslatedAccount<'a>>, Error> {
        translate_accounts_c(account_infos_addr, account_infos_len, invoke_context)
    }
}

/// Cross-program invocation called from `sol_invoke_signed_v2`
pub struct SyscallInvokeSignedV2 {}
impl BuiltinFunctionDefinition<InvokeContext<'_, '_>> for SyscallInvokeSignedV2 {
    type Error = Error;
    fn rust(
        invoke_context: &mut InvokeContext<'_, '_>,
        instruction_addr: u64,
        signers_seeds_addr: u64,
        signers_seeds_len: u64,
        _arg4: u64,
        _arg5: u64,
    ) -> Result<u64, Error> {
        let check_aligned = invoke_context.get_check_aligned();

        // `invoke_signed_v2` requires check_aligned to be `true` (ABIv1), so return an early
        // error if it is not.
        if !check_aligned {
            return Err(MemoryTranslationError::UnalignedPointer.into());
        }

        let memory_mapping = invoke_context.memory_contexts.memory_mapping()?;
        let ix_c =
            translate_type::<SolInstruction>(memory_mapping, instruction_addr, check_aligned)?;

        cpi_common::<Self>(
            invoke_context,
            instruction_addr,
            ix_c.accounts_addr,
            ix_c.accounts_len,
            signers_seeds_addr,
            signers_seeds_len,
        )
    }
}

impl SyscallInvokeSigned for SyscallInvokeSignedV2 {
    fn translate_instruction(
        addr: u64,
        invoke_context: &InvokeContext,
    ) -> Result<Instruction, Error> {
        translate_instruction_v2(addr, invoke_context)
    }

    fn translate_accounts<'a>(
        account_metas_addr: u64,
        account_metas_len: u64,
        invoke_context: &InvokeContext,
    ) -> Result<Vec<TranslatedAccount<'a>>, Error> {
        translate_accounts_v2(account_metas_addr, account_metas_len, invoke_context)
    }
}

/// Cross-program invocation called from `sol_invoke_signed_v2`
pub struct SyscallInvokeSignedABIV2 {}

impl BuiltinFunctionDefinition<InvokeContext<'_, '_>> for SyscallInvokeSignedV2 {
    type Error = Error;
    fn rust(
        invoke_context: &mut InvokeContext<'_, '_>,
        program_idx_in_tx: u64,
        signers_seeds_addr: u64,
        signers_seeds_len: u64,
        _arg4: u64,
        _arg5: u64,
    ) -> Result<u64, Error> {
        // Deduct cost
        let compute_cost = invoke_context.get_execution_cost();
        invoke_context.transaction_context.compute_meter.consume_checked(compute_cost.abi_v2_cpi_base)?;

        // Configure instruction frame
        let callee_program_index_in_tx = u16::try_from(program_idx_in_tx).map_err(|_| InstructionError::MissingAccount)?;
        invoke_context.transaction_context.build_abi_v2_frame(callee_program_index_in_tx)?;

        // This check also verifies that the program account is in the transaction
        let caller_program_id = invoke_context.transaction_context.get_current_instruction_context()?.get_program_key()?;

        // Convert seeds
        let signers = translate_signers(caller_program_id, signers_seeds_addr, signers_seeds_len, invoke_context)?;

        // Invoke program
        invoke_context.internal_native_invoke(&signers)?;
        Ok(0)
    }
}