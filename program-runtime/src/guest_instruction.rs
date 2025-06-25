#![allow(unused)]

use {
    crate::guest_transaction::GuestSliceReference, solana_sbpf::ebpf::MM_REGION_SIZE,
    solana_svm_transaction::svm_message::SVMMessage,
};

#[repr(C)]
pub struct GuestInstruction {
    program_id_idx: u64,
    cpi_nesting_level: u16,
    parent_ix_idx: u16,
    ix_accounts: GuestSliceReference,
    ix_data: GuestSliceReference,
}

#[repr(C)]
pub struct GuestInstructionAccount {
    tx_acc_idx: u16,
    flags: u16,
}

const IX_ACCOUNTS_REGION: u64 = MM_REGION_SIZE * 7;
const IX_PAYLOAD_REGION: u64 = MM_REGION_SIZE * 256;

pub(crate) fn create_ix_array(
    tx: &impl SVMMessage,
) -> (Vec<GuestInstruction>, Vec<GuestInstructionAccount>) {
    let mut final_vec: Vec<GuestInstruction> = Vec::with_capacity(tx.num_instructions());
    let mut instr_acc: Vec<GuestInstructionAccount> =
        Vec::with_capacity(tx.num_instructions().saturating_mul(3usize));

    let mut accounts_addr: u64 = IX_ACCOUNTS_REGION;
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
                pointer: IX_PAYLOAD_REGION
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

#[cfg(test)]
mod test {
    use {
        crate::guest_instruction::{
            create_ix_array, GuestInstructionAccount, IX_ACCOUNTS_REGION, IX_PAYLOAD_REGION,
        },
        solana_hash::Hash,
        solana_message::compiled_instruction::CompiledInstruction,
        solana_pubkey::Pubkey,
        solana_sbpf::ebpf::MM_REGION_SIZE,
        solana_svm_transaction::{
            instruction::SVMInstruction,
            message_address_table_lookup::SVMMessageAddressTableLookup, svm_message::SVMMessage,
        },
        solana_transaction::sanitized::SanitizedTransaction,
        std::ops::Range,
    };

    #[derive(Debug)]
    struct DummyTx {
        ix: Vec<CompiledInstruction>,
    }

    impl SVMMessage for DummyTx {
        fn num_transaction_signatures(&self) -> u64 {
            unimplemented!()
        }

        fn num_write_locks(&self) -> u64 {
            unimplemented!()
        }

        fn recent_blockhash(&self) -> &Hash {
            unimplemented!()
        }

        fn num_instructions(&self) -> usize {
            self.ix.len()
        }

        fn instructions_iter(&self) -> impl Iterator<Item = SVMInstruction> {
            self.ix.iter().map(SVMInstruction::from)
        }

        fn program_instructions_iter(
            &self,
        ) -> impl Iterator<Item = (&Pubkey, SVMInstruction)> + Clone {
            unimplemented!();
            let a = unsafe { std::mem::transmute::<&DummyTx, &SanitizedTransaction>(self) };

            SVMMessage::program_instructions_iter(SanitizedTransaction::message(a))
        }

        fn static_account_keys(&self) -> &[Pubkey] {
            unimplemented!()
        }

        fn account_keys(&self) -> solana_message::AccountKeys {
            unimplemented!()
        }

        fn fee_payer(&self) -> &Pubkey {
            unimplemented!()
        }

        fn is_writable(&self, index: usize) -> bool {
            (index % 2) == 1
        }

        fn is_signer(&self, index: usize) -> bool {
            (index % 2) == 0
        }

        fn is_invoked(&self, key_index: usize) -> bool {
            unimplemented!()
        }

        fn num_lookup_tables(&self) -> usize {
            unimplemented!()
        }

        fn message_address_table_lookups(
            &self,
        ) -> impl Iterator<Item = SVMMessageAddressTableLookup> {
            unimplemented!();
            let a = unsafe { std::mem::transmute::<&DummyTx, &SanitizedTransaction>(self) };

            SVMMessage::message_address_table_lookups(SanitizedTransaction::message(a))
        }
    }

    #[test]
    fn test_ix_area() {
        let ix_vec = vec![
            CompiledInstruction {
                program_id_index: 2,
                accounts: vec![1, 2, 3, 4],
                data: vec![0, 9, 8, 5],
            },
            CompiledInstruction {
                program_id_index: 6,
                accounts: vec![9, 0],
                data: vec![0],
            },
            CompiledInstruction {
                program_id_index: 8,
                accounts: vec![8, 8, 8],
                data: vec![1, 2, 3],
            },
        ];

        let svm_mes = DummyTx { ix: ix_vec };

        let (g_instrs, g_accs) = create_ix_array(&svm_mes);

        for (idx, c_instr) in svm_mes.ix.iter().enumerate() {
            let g_instr = g_instrs.get(idx).unwrap();
            assert_eq!(c_instr.program_id_index as u64, g_instr.program_id_idx);
            assert_eq!(0, g_instr.cpi_nesting_level);
            assert_eq!(u16::MAX, g_instr.parent_ix_idx);
            assert_eq!(
                IX_PAYLOAD_REGION + idx as u64 * MM_REGION_SIZE,
                g_instr.ix_data.pointer
            );
            assert_eq!(c_instr.data.len() as u64, g_instr.ix_data.length);
            assert_eq!(c_instr.accounts.len() as u64, g_instr.ix_accounts.length);

            let starting_index = (g_instr.ix_accounts.pointer - IX_ACCOUNTS_REGION)
                / size_of::<GuestInstructionAccount>() as u64;
            for i in starting_index..(starting_index + g_instr.ix_accounts.length) {
                let g_acc = g_accs.get(i as usize).unwrap();
                let c_acc = *c_instr.accounts.get((i - starting_index) as usize).unwrap();
                assert_eq!(c_acc as u16, g_acc.tx_acc_idx);
                assert_eq!(svm_mes.is_writable(c_acc as usize) as u16, g_acc.flags >> 1);
                assert_eq!(svm_mes.is_signer(c_acc as usize) as u16, g_acc.flags & 0x1);
            }
        }
    }
}
