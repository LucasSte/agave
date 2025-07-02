#![allow(unused)]

use std::cell::RefCell;
use std::sync::Arc;
use solana_sbpf::ebpf::{MM_ACCOUNTS_AREA, MM_RETURN_DATA_AREA, MM_TX_AREA, MM_TX_INSTRUCTION_AREA, MM_TX_INSTRUCTION_DATA_AREA};
use solana_sbpf::memory_region::MemoryRegion;
use {
    solana_account::ReadableAccount, solana_pubkey::Pubkey, solana_sbpf::ebpf::MM_REGION_SIZE,
    solana_svm_feature_set::SVMFeatureSet,
    std::slice,
};
use solana_svm_transaction::svm_message::SVMMessage;
use solana_transaction_context::TransactionAccount;
use crate::guest_instruction::{create_ix_array, GuestInstruction, GuestInstructionAccount};
use crate::guest_slice::GuestSliceReference;

/// The Return data scratchpad
#[repr(C)]
struct ReturnDataScratchpad {
    /// The key of the last program to write in the scratchpad
    pubkey: Pubkey,
    /// Reference to the slice
    slice: GuestSliceReference,
}

/// `GuestTransactionAccount` is how a transaction appears to programs in the virtual machine
#[repr(C)]
struct GuestTransactionAccount {
    pubkey: Pubkey,
    owner: Pubkey,
    lamports: u64,
    data: GuestSliceReference,
}

#[repr(C)]
struct GuestTransactionContext {
    return_data_scratchpad: ReturnDataScratchpad,
    cpi_scratchpad: GuestSliceReference,
    /// The index of the current executing instruction
    instruction_idx: u64,
    /// The number of instructions in the transaction
    instruction_num: u64,
    /// The number of accounts in the transaction
    accounts_no: u64,
}

/// `RuntimeGuestTransaction` contains both the `GuestTransactionContext` and an array of
/// `GuestTransactionAccount`. It is the memory region in ABIv2 that contains the transaction
/// information.
pub struct RuntimeGuestTransaction {
    tx_raw_metadata: Box<[u8]>,
    ix_metadata: Vec<GuestInstruction>,
    ix_accounts: Vec<GuestInstructionAccount>,
    account_data: Vec<Arc<Vec<u8>>>,
    payloads: Vec<MemoryRegion>,
}


impl RuntimeGuestTransaction {
    pub fn new_with_feature_set(
        transaction_accounts: &[TransactionAccount],
        message: &impl SVMMessage,
        feature_set: &SVMFeatureSet,
    ) -> Option<RuntimeGuestTransaction> {
        if feature_set.enable_abi_v2_programs {
            return Some(Self::new(transaction_accounts, message));
        }

        None
    }

    // TODO: This is supposed to become the new Transaction Context
    pub(crate) fn new(
        transaction_accounts: &[TransactionAccount],
        message: &impl SVMMessage,
    ) -> RuntimeGuestTransaction {
        let size = size_of::<GuestTransactionContext>().saturating_add(
            transaction_accounts.len()
                .saturating_mul(size_of::<GuestTransactionAccount>()),
        );
        let mut memory_vec: Vec<u8> = Vec::with_capacity(size);
        let memory = memory_vec.spare_capacity_mut();

        // SAFETY: The memory region is large enough to contain a GuestTransactionContext
        let guest_transaction =
            unsafe { &mut *(memory.as_mut_ptr() as *mut GuestTransactionContext) };

        guest_transaction.return_data_scratchpad = ReturnDataScratchpad {
            pubkey: Pubkey::new_from_array([0u8; 32]),
            slice: GuestSliceReference {
                pointer: MM_RETURN_DATA_AREA,
                length: 0,
            },
        };

        guest_transaction.cpi_scratchpad = GuestSliceReference {
            pointer: MM_TX_INSTRUCTION_DATA_AREA + MM_REGION_SIZE * message.num_instructions() as u64,
            length: 0,
        };

        guest_transaction.instruction_idx = 0;
        guest_transaction.instruction_num = message.num_instructions() as u64;
        guest_transaction.accounts_no = transaction_accounts.len() as u64;

        // SAFETY: The memory region is large enough to contain a GuestTransactionContext and an
        // array of `GuestTransactionAccount`
        let guest_transaction_accounts = unsafe {
            let ptr = memory
                .as_mut_ptr()
                .add(size_of::<GuestTransactionContext>());
            slice::from_raw_parts_mut(
                ptr as *mut GuestTransactionAccount,
                guest_transaction.accounts_no as usize,
            )
        };

        for (idx, tx_account) in transaction_accounts.iter().enumerate() {
            let account_ref = guest_transaction_accounts
                .get_mut(idx)
                .unwrap();
            account_ref.pubkey = tx_account.0;
            account_ref.owner = *tx_account.1.owner();
            account_ref.lamports = tx_account.1.lamports();
            let vm_data_addr = MM_ACCOUNTS_AREA
                .saturating_add(MM_REGION_SIZE.saturating_mul(idx as u64));
            account_ref.data = GuestSliceReference {
                pointer: vm_data_addr,
                length: tx_account.1.data().len() as u64,
            };
        }

        // SAFETY: The vector has been allocated with at least `size` bytes.
        unsafe {
            memory_vec.set_len(size);
        }
        
        let payloads = message.instructions_iter().enumerate().map(|(idx, item)|
            MemoryRegion::new_readonly(
                item.data,
                MM_TX_INSTRUCTION_DATA_AREA + MM_REGION_SIZE * idx as u64,
            )
        ).collect();
        
        let (ix_metadata, ix_accounts) = create_ix_array(message);
        RuntimeGuestTransaction {
            tx_raw_metadata: memory_vec.into_boxed_slice(),
            ix_metadata,
            ix_accounts,
            account_data: transaction_accounts.iter().map(|item| item.1.data_clone()).collect(),
            payloads,
        }
    }
    
    pub fn retrieve_instruction(&self) -> &GuestInstruction {
        let context = unsafe { &*(self.tx_raw_metadata.as_ptr() as *const GuestTransactionContext) };
        let ix_idx = context.instruction_idx;
        &self.ix_metadata[ix_idx as usize]
    }
    
    pub fn prepare_regions(&self) -> Vec<MemoryRegion> {
        let instr = self.retrieve_instruction();
        let mut regions: Vec<MemoryRegion> = Vec::with_capacity(
            3+self.ix_metadata.len()+self.payloads.len() as usize
        );
        
        // TX Area
        regions.push(
            MemoryRegion::new_readonly(
                self.as_slice(),
                MM_TX_AREA
            )
        );
        
        // IX Area
        let ix_metadata_slice = unsafe {
            let length = self.ix_metadata.len() * size_of::<GuestInstruction>();
            slice::from_raw_parts(self.ix_metadata.as_ptr() as *const u8, length)
        };
        regions.push(
            MemoryRegion::new_readonly(
                ix_metadata_slice,
                MM_TX_INSTRUCTION_AREA
            )
        );
        
        // IX accounts metadata area
        let ix_account_slice = unsafe {
            let length = self.ix_accounts.len() * size_of::<GuestInstructionAccount>();
            slice::from_raw_parts(self.ix_accounts.as_ptr() as *const u8, length)
        };
        regions.push(
            MemoryRegion::new_readonly(
                ix_account_slice,
                MM_TX_INSTRUCTION_DATA_AREA
            )
        );
        
        // tx accounts payload area
        let starting_index = ((instr.ix_accounts.pointer - MM_ACCOUNTS_AREA)
            / size_of::<GuestInstructionAccount>() as u64) as usize;
        let length = instr.ix_accounts.length as usize;
        for i in starting_index..(starting_index+length) {
            let ix_account_metadata = self.ix_accounts.get(i).unwrap();
            
            let data = self.account_data.get(ix_account_metadata.tx_acc_idx as usize).unwrap();
            let addr = MM_ACCOUNTS_AREA + MM_REGION_SIZE * ix_account_metadata.tx_acc_idx as u64;
            // The writable check isn't as simple as the flag, and this part must be integrated into
            // TransactionContext.
            let region = if (ix_account_metadata.flags >> 1) == 1 {
                // This is a hack and must be removed in the refactor.
                #[allow(mutable_transmutes)]
                let slice = unsafe {
                    std::mem::transmute::<&[u8], &mut [u8]>(data.as_slice())
                };
                MemoryRegion::new_writable(
                    slice,
                    addr
                )
            } else {
                MemoryRegion::new_readonly(
                    data.as_slice(),
                    addr
                )
            };
        }
        
        // The payloads region
        regions.extend(self.payloads.clone());
        regions
    }
    
    pub fn as_slice(&self) -> &[u8] {
        &self.tx_raw_metadata
    }

    pub fn set_instruction_index(&mut self, index: usize) {
        // SAFETY: We assume the transaction was created using `RuntimeGuestTransaction::new`, which
        // guarantees the safety of size constraints and contents.
        let context = unsafe { &mut *(self.tx_raw_metadata.as_mut_ptr() as *mut GuestTransactionContext) };

        context.instruction_idx = index as u64;
    }
}

#[cfg(test)]
mod test {
    use solana_hash::Hash;
    use solana_message::compiled_instruction::CompiledInstruction;
    use solana_sbpf::ebpf::{MM_ACCOUNTS_AREA, MM_RETURN_DATA_AREA, MM_TX_INSTRUCTION_DATA_AREA};
    use solana_transaction::sanitized::SanitizedTransaction;
    use {
        crate::guest_transaction::{
            GuestTransactionAccount, GuestTransactionContext, RuntimeGuestTransaction,
        },
        solana_account::{Account, AccountSharedData, ReadableAccount},
        solana_pubkey::Pubkey,
        solana_sbpf::ebpf::MM_REGION_SIZE,
        solana_sdk_ids::bpf_loader,
        std::slice,
    };
    use solana_svm_transaction::instruction::SVMInstruction;
    use solana_svm_transaction::message_address_table_lookup::SVMMessageAddressTableLookup;
    use solana_svm_transaction::svm_message::SVMMessage;
    use crate::guest_instruction::{create_ix_array, GuestInstructionAccount};

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
    fn test_creation() {
        let transaction_accounts = vec![
            (
                solana_pubkey::new_rand(),
                AccountSharedData::from(Account {
                    lamports: 0,
                    data: vec![],
                    owner: bpf_loader::id(),
                    executable: true,
                    rent_epoch: 0,
                }),
            ),
            (
                solana_pubkey::new_rand(),
                AccountSharedData::from(Account {
                    lamports: 1,
                    data: vec![1u8, 2, 3, 4, 5],
                    owner: bpf_loader::id(),
                    executable: false,
                    rent_epoch: 100,
                }),
            ),
            (
                solana_pubkey::new_rand(),
                AccountSharedData::from(Account {
                    lamports: 2,
                    data: vec![11u8, 12, 13, 14, 15, 16, 17, 18, 19],
                    owner: bpf_loader::id(),
                    executable: true,
                    rent_epoch: 200,
                }),
            ),
            (
                solana_pubkey::new_rand(),
                AccountSharedData::from(Account {
                    lamports: 3,
                    data: vec![],
                    owner: bpf_loader::id(),
                    executable: false,
                    rent_epoch: 300,
                }),
            ),
            (
                solana_pubkey::new_rand(),
                AccountSharedData::from(Account {
                    lamports: 4,
                    data: vec![1u8, 2, 3, 4, 5],
                    owner: bpf_loader::id(),
                    executable: false,
                    rent_epoch: 100,
                }),
            ),
            (
                solana_pubkey::new_rand(),
                AccountSharedData::from(Account {
                    lamports: 5,
                    data: vec![11u8, 12, 13, 14, 15, 16, 17, 18, 19],
                    owner: bpf_loader::id(),
                    executable: true,
                    rent_epoch: 200,
                }),
            ),
        ];

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

        let mut runtime_transaction = RuntimeGuestTransaction::new(&transaction_accounts, &svm_mes);

        let guest_transaction = unsafe {
            &*(runtime_transaction.as_slice().as_ptr() as *const GuestTransactionContext)
        };

        assert_eq!(
            guest_transaction.return_data_scratchpad.pubkey,
            Pubkey::new_from_array([0u8; 32]),
        );
        assert_eq!(
            guest_transaction.return_data_scratchpad.slice.pointer,
            MM_RETURN_DATA_AREA
        );
        assert_eq!(guest_transaction.return_data_scratchpad.slice.length, 0);

        assert_eq!(
            guest_transaction.cpi_scratchpad.pointer,
            MM_TX_INSTRUCTION_DATA_AREA + MM_REGION_SIZE * svm_mes.num_instructions() as u64,
        );
        assert_eq!(guest_transaction.cpi_scratchpad.length, 0);

        assert_eq!(guest_transaction.instruction_idx, 0);
        assert_eq!(guest_transaction.instruction_num, svm_mes.num_instructions() as u64);
        runtime_transaction.set_instruction_index(80);
        assert_eq!(guest_transaction.instruction_idx, 80);

        assert_eq!(guest_transaction.accounts_no, 6);

        let guest_accounts = unsafe {
            let ptr = runtime_transaction
                .as_slice()
                .as_ptr()
                .add(size_of::<GuestTransactionContext>());
            slice::from_raw_parts(
                ptr as *const GuestTransactionAccount,
                guest_transaction.accounts_no as usize,
            )
        };

        for (idx, tx_account) in transaction_accounts.iter().enumerate() {
            let guest_account = guest_accounts.get(idx).unwrap();

            assert_eq!(
                tx_account.0,
                guest_account.pubkey
            );
            assert_eq!(*tx_account.1.owner(), guest_account.owner);
            assert_eq!(tx_account.1.lamports(), guest_account.lamports);
            let addr = MM_ACCOUNTS_AREA + MM_REGION_SIZE * idx as u64;
            assert_eq!(addr, guest_account.data.pointer);
            assert_eq!(tx_account.1.data().len() as u64, guest_account.data.length);
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
                MM_TX_INSTRUCTION_DATA_AREA + idx as u64 * MM_REGION_SIZE,
                g_instr.ix_data.pointer
            );
            assert_eq!(c_instr.data.len() as u64, g_instr.ix_data.length);
            assert_eq!(c_instr.accounts.len() as u64, g_instr.ix_accounts.length);

            let starting_index = (g_instr.ix_accounts.pointer - MM_ACCOUNTS_AREA)
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
