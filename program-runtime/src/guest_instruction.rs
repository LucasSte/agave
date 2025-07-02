#![allow(unused)]

use solana_sbpf::ebpf::{MM_ACCOUNTS_AREA, MM_TX_INSTRUCTION_DATA_AREA};
use {
    solana_sbpf::ebpf::MM_REGION_SIZE,
    solana_svm_transaction::svm_message::SVMMessage,
};
use crate::guest_slice::GuestSliceReference;

#[repr(C)]
pub struct GuestInstruction {
    pub(crate) program_id_idx: u64,
    pub(crate) cpi_nesting_level: u16,
    pub(crate) parent_ix_idx: u16,
    pub(crate) ix_accounts: GuestSliceReference,
    pub(crate) ix_data: GuestSliceReference,
}

#[repr(C)]
pub struct GuestInstructionAccount {
    pub(crate) tx_acc_idx: u16,
    pub(crate) flags: u16,
}

pub(crate) fn create_ix_array(
    tx: &impl SVMMessage,
) -> (Vec<GuestInstruction>, Vec<GuestInstructionAccount>) {
    let mut final_vec: Vec<GuestInstruction> = Vec::with_capacity(tx.num_instructions());
    let mut instr_acc: Vec<GuestInstructionAccount> =
        Vec::with_capacity(tx.num_instructions().saturating_mul(3usize));

    let mut accounts_addr: u64 = MM_ACCOUNTS_AREA;
    for (ix_idx, instr) in tx.instructions_iter().enumerate() {
        for acc in instr.accounts {
            instr_acc.push(GuestInstructionAccount {
                tx_acc_idx: *acc as u16,
                flags: (tx.is_signer(*acc as usize) as u16)
                    | ((tx.is_writable(*acc as usize) as u16) << 1),
            });
        }

        final_vec.push(GuestInstruction {
            program_id_idx: instr.program_id_index as u64,
            cpi_nesting_level: 0,
            parent_ix_idx: u16::MAX,
            ix_accounts: GuestSliceReference {
                pointer: accounts_addr,
                length: instr.accounts.len() as u64,
            },
            ix_data: GuestSliceReference {
                pointer: MM_TX_INSTRUCTION_DATA_AREA
                    .saturating_add((ix_idx as u64).saturating_mul(MM_REGION_SIZE)),
                length: instr.data.len() as u64,
            },
        });
        accounts_addr = accounts_addr.saturating_add(
            instr
                .accounts
                .len()
                .saturating_mul(size_of::<GuestInstructionAccount>()) as u64,
        );
    }

    (final_vec, instr_acc)
}
