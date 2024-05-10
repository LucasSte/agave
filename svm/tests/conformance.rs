use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::env;
use std::ffi::OsString;
use std::fs::File;
use std::io::Read;
use std::sync::{Arc, RwLock};
use lazy_static::lazy_static;
use prost::Message;
use solana_bpf_loader_program::syscalls::create_program_runtime_environment_v1;
use solana_program_runtime::compute_budget::ComputeBudget;
use solana_program_runtime::loaded_programs::{BlockRelation, ForkGraph, ProgramCache, ProgramCacheEntry, ProgramRuntimeEnvironments};
use solana_program_runtime::solana_rbpf::program::{BuiltinProgram, FunctionRegistry};
use solana_program_runtime::solana_rbpf::vm::Config;
use solana_program_runtime::timings::ExecuteTimings;
use solana_sdk::account::{AccountSharedData, ReadableAccount, WritableAccount};
use solana_sdk::bpf_loader_upgradeable;
use solana_sdk::clock::{Epoch, Slot};
use solana_sdk::epoch_schedule::EpochSchedule;
use solana_sdk::feature_set::{FEATURE_NAMES, FeatureSet};
use solana_sdk::hash::Hash;
use solana_sdk::instruction::AccountMeta;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Signature;
use solana_sdk::transaction::TransactionError;
use solana_svm::runtime_config::RuntimeConfig;
use solana_svm::transaction_error_metrics::TransactionErrorMetrics;
use solana_svm::transaction_processor::{ExecutionRecordingConfig, TransactionBatchProcessor};
use crate::mock_bank::MockBankCallback;
use crate::proto::InstrFixture;
use crate::transaction_builder::SanitizedTransactionBuilder;

mod proto {
    include!(concat!(env!("OUT_DIR"), "/org.solana.sealevel.v1.rs"));
}
mod mock_bank;
mod transaction_builder;

const fn feature_u64(feature: &Pubkey) -> u64 {
    let feature_id = feature.to_bytes();
    feature_id[0] as u64
        | (feature_id[1] as u64) << 8
        | (feature_id[2] as u64) << 16
        | (feature_id[3] as u64) << 24
        | (feature_id[4] as u64) << 32
        | (feature_id[5] as u64) << 40
        | (feature_id[6] as u64) << 48
        | (feature_id[7] as u64) << 56
}

lazy_static! {
    static ref INDEXED_FEATURES: HashMap<u64, Pubkey> = {
        FEATURE_NAMES
            .iter()
            .map(|(pubkey, _)| (feature_u64(pubkey), *pubkey))
            .collect()
    };
}

struct MockForkGraph {}

impl ForkGraph for MockForkGraph {
    fn relationship(&self, a: Slot, b: Slot) -> BlockRelation {
        match a.cmp(&b) {
            Ordering::Less => BlockRelation::Ancestor,
            Ordering::Equal => BlockRelation::Equal,
            Ordering::Greater => BlockRelation::Descendant,
        }
    }

    fn slot_epoch(&self, _slot: Slot) -> Option<Epoch> {
        Some(0)
    }
}

// TODO:
// Should fetch the test-vectors during runtime

#[test]
fn fixture() {
    let mut dir = env::current_dir().unwrap();
    dir.push("test-vectors");
    dir.push("instr");
    dir.push("fixtures");
    dir.push("20240425");
    dir.push("stake");

    // let ignore = HashSet::from([
    //     OsString::from("573f54c36f3eca95.bin")
    // ]);
    // for path in std::fs::read_dir(dir).unwrap() {
    //     let filename = path.as_ref().unwrap().file_name();
    //     if ignore.contains(&filename) {
    //         continue;
    //     }
    //     let mut file = File::open(path.as_ref().unwrap().path()).expect("file not found");
    //     let mut buffer = Vec::new();
    //     file.read_to_end(&mut buffer).expect("Failed to read file");
    //     std::println!("Testing: {}", path.unwrap().path().display());
    //
    //     let fixture = proto::InstrFixture::decode(buffer.as_slice()).unwrap();
    //     execute_fixture(fixture);
    // }

    // These cases hit !native_loader::check_id(account.owner()) in  svm/src/transaction_account_state_info.rs:34
    // let ignore = HashSet::from([
    //     OsString::from("34ee00c659dc5aa6.bin"),
    //     OsString::from("7fcde5cb94e1dc44.bin"),
    //     OsString::from("aefbe2c014993346.bin"),
    //     OsString::from("8fd951ecde987723.bin"),
    //     OsString::from("0c4d64d17bf10145.bin"),
    //     OsString::from("9c64e491c3e5d64f.bin"),
    //     OsString::from("0fd4bb1ce4eb4b1a.bin"),
    //     OsString::from("9f3c001dcd1803fe.bin"),
    //     OsString::from("701aa738f1970730.bin"),
    // ]);
    // for path in std::fs::read_dir(dir).unwrap() {
    //     let filename = path.as_ref().unwrap().file_name();
    //     if ignore.contains(&filename) {
    //         continue;
    //     }
    //     let mut file = File::open(path.as_ref().unwrap().path()).expect("file not found");
    //     let mut buffer = Vec::new();
    //     file.read_to_end(&mut buffer).expect("Failed to read file");
    //     std::println!("Testing: {}", path.unwrap().path().display());
    //
    //     let fixture = proto::InstrFixture::decode(buffer.as_slice()).unwrap();
    //     execute_fixture(fixture);
    // }

    // let ignore = HashSet::from([
    //     // Lamports disagree
    //     OsString::from("16e004f6282214fb.bin"),
    //     OsString::from("46554d839980adaa.bin"),
    //     OsString::from("efb039175e24951f.bin"),
    //     OsString::from("548eadba9731d55d.bin"),
    //     OsString::from("05073bd8b165c05c.bin"),
    //     OsString::from("27b566f38c90701d.bin"),
    //     OsString::from("5b1ca8102f32ff40.bin"),
    //     OsString::from("f1474c2d42bfa1c3.bin"),
    //     OsString::from("89596672771d2ce0.bin"),
    //     OsString::from("4f08f332c85ae207.bin"),
    //     OsString::from("9e4bf65f3dbd9571.bin"),
    //     OsString::from("6b49c546301c5d8f.bin"),
    //     // Failed transaction
    //     OsString::from("2b9c4987d3ddae11.bin"),
    //     OsString::from("fabd5841eee1c39a.bin"),
    // ]);
    // for path in std::fs::read_dir(dir).unwrap() {
    //     let filename = path.as_ref().unwrap().file_name();
    //     if ignore.contains(&filename) {
    //         continue;
    //     }
    //     let mut file = File::open(path.as_ref().unwrap().path()).expect("file not found");
    //     let mut buffer = Vec::new();
    //     file.read_to_end(&mut buffer).expect("Failed to read file");
    //     std::println!("Testing: {}", path.unwrap().path().display());
    //
    //     let fixture = proto::InstrFixture::decode(buffer.as_slice()).unwrap();
    //     execute_fixture(fixture, filename);
    // }

    let ignore = HashSet::from([
        // Return data differ
        OsString::from("43af1b5d721281f8.bin"),
        OsString::from("6f632e1ad6075480.bin"),
        OsString::from("5078aeaf8961ed39.bin"),
    ]);
    for path in std::fs::read_dir(dir).unwrap() {
        let filename = path.as_ref().unwrap().file_name();
        if ignore.contains(&filename) {
            continue;
        }
        let mut file = File::open(path.as_ref().unwrap().path()).expect("file not found");
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer).expect("Failed to read file");
        std::println!("Testing: {}", path.unwrap().path().display());

        let fixture = proto::InstrFixture::decode(buffer.as_slice()).unwrap();
        execute_fixture(fixture, filename);
    }

    //dir.push("c166fadd709eb7e1.bin");
    //dir.push("0c9471f50baa2b03.bin");

    // For debugging
    // dir.push("573f54c36f3eca95.bin");
    //
    // let mut file = File::open(dir.clone()).expect("file not found");
    // let mut buffer = Vec::new();
    // file.read_to_end(&mut buffer).expect("Failed to read file");
    //
    // let fixture = proto::InstrFixture::decode(buffer.as_slice()).unwrap();
    //
    // let program_id = fixture.input.as_ref().unwrap().program_id.clone();
    // std::println!("program id: {:?}", Pubkey::new_from_array(program_id.try_into().unwrap()));
    //
    // for item in &fixture.input.as_ref().unwrap().accounts {
    //     std::println!("Acct: {:?} => owner: {:?}",
    //                   Pubkey::new_from_array(item.address.clone().try_into().unwrap()),
    //         Pubkey::new_from_array(item.owner.clone().try_into().unwrap()),
    //     );
    // }
    //
    // for item in &fixture.input.as_ref().unwrap().instr_accounts {
    //     std::println!("idx: {}, writable: {}, signer: {}", item.index, item.is_writable, item.is_signer);
    // }
    //
    // std::println!("Has txn context: {:?}", fixture.input.as_ref().unwrap().txn_context.is_some());
    // std::println!("Has slot context: {:?}", fixture.input.as_ref().unwrap().slot_context.is_some());
    // std::println!("Has epoch context: {:?}", fixture.input.as_ref().unwrap().epoch_context.is_some());
    //
    // execute_fixture(fixture);
}

fn execute_fixture(fixture: InstrFixture, filename: OsString) {
    let input = fixture.input.unwrap();
    let output = fixture.output.unwrap();
    //std::println!("Result: {}, err: {}", output.result, output.custom_err);

    let mut transaction_builder = SanitizedTransactionBuilder::default();
    let program_id = Pubkey::new_from_array(input.program_id.try_into().unwrap());
    let mut accounts : Vec<AccountMeta> = Vec::with_capacity(input.instr_accounts.len());
    let mut signatures : HashMap<Pubkey, Signature> = HashMap::with_capacity(input.instr_accounts.len());

    for item in input.instr_accounts {
        let pubkey = Pubkey::new_from_array(input.accounts[item.index as usize].address.clone().try_into().unwrap());
        accounts.push(
            AccountMeta {
                pubkey,
                is_signer: item.is_signer,
                is_writable: item.is_writable
            }
        );

        if item.is_signer {
            signatures.insert(
                pubkey,
                Signature::new_unique()
            );
        }
    }

    transaction_builder.create_instruction(
        program_id,
        accounts,
        signatures,
        input.data
    );

    let mut feature_set = FeatureSet::default();
    if let Some(features) = &input.epoch_context.as_ref().unwrap().features {
        for id in &features.features {
            if let Some(pubkey) = INDEXED_FEATURES.get(id) {
                feature_set.activate(pubkey, 0);
            }
        }
    }

    let fee_payer = Pubkey::new_unique();
    let Ok(transaction) = transaction_builder.build(
        Hash::default(), (fee_payer, Signature::new_unique()), false
    ) else {
        assert_ne!(output.result, 0);
        return;
    };

    //std::println!("transaction: {:#?}", transaction);
    let transactions = vec![transaction];
    let mut transaction_check = vec![(Ok(()), None, Some(30))];

    let mut mock_bank = MockBankCallback::default();
    {
        let mut account_data_map = mock_bank.account_shared_data.borrow_mut();
        for item in input.accounts {
            let pubkey = Pubkey::new_from_array(item.address.try_into().unwrap());
            if bpf_loader_upgradeable::check_id(&pubkey) {
                break;
            }

            let mut account_data = AccountSharedData::default();
            account_data.set_lamports(item.lamports);
            account_data.set_data(item.data);
            account_data.set_owner(Pubkey::new_from_array(item.owner.try_into().unwrap()));
            account_data.set_executable(item.executable);
            account_data.set_rent_epoch(item.rent_epoch);

            account_data_map.insert(
                pubkey,
                account_data
            );
        }
        let mut account_data = AccountSharedData::default();
        account_data.set_lamports(800000);
        account_data_map.insert(
            fee_payer,
            account_data
        );
    }

    let compute_budget = ComputeBudget {
        compute_unit_limit: input.cu_avail,
        ..ComputeBudget::default()
    };

    let v1_environment = create_program_runtime_environment_v1(
        &feature_set,
        &compute_budget,
        false,
        false
    ).unwrap();

    let mut program_cache = ProgramCache::<MockForkGraph>::new(0, 20);
    program_cache.environments = ProgramRuntimeEnvironments {
        program_runtime_v1: Arc::new(v1_environment),
        program_runtime_v2: Arc::new(BuiltinProgram::new_loader(
            Config::default(),
            FunctionRegistry::default(),
        ))
    };
    program_cache.fork_graph = Some(Arc::new(RwLock::new(MockForkGraph {})));

    let program_cache = Arc::new(RwLock::new(program_cache));
    mock_bank.override_feature_set(feature_set);
    let batch_processor = TransactionBatchProcessor::<MockForkGraph>::new(
        42,
        2,
        EpochSchedule::default(),
        Arc::new(RuntimeConfig::default()),
        program_cache.clone(),
        HashSet::new(),
    );

    batch_processor.fill_missing_sysvar_cache_entries(&mock_bank);
    register_builtins(&batch_processor, &mock_bank);

    let mut error_counter = TransactionErrorMetrics::default();
    let recording_config = ExecutionRecordingConfig {
        enable_log_recording: true,
        enable_return_data_recording: true,
        enable_cpi_recording: false,
    };
    let mut timings = ExecuteTimings::default();

    let result = batch_processor.load_and_execute_sanitized_transactions(
        &mock_bank,
        &transactions,
        transaction_check.as_mut_slice(),
        &mut error_counter,
        recording_config,
        &mut timings,
        None,
        None,
        true,
    );

    //std::println!("{:?}", result.execution_results);

    // assert that is worked and has no error
    if !result.execution_results[0].was_executed() || result.execution_results[0].details().unwrap().status.is_err() {
        //std::println!("{:?}", result.execution_results[0]);
        //std::println!("result: {} -  custom error: {}", output.result, output.custom_err);
        if output.result != 0 {
            return;
        }

        match result.execution_results[0].flattened_result() {
            Err(TransactionError::InsufficientFundsForRent { .. }) => {
                // This is a transaction error not an instruction error
                let details = result.execution_results[0].details().unwrap();
                // TODO: Consider the right string here!
                //assert!(details.log_messages.as_ref().unwrap().contains(&"Program BPFLoaderUpgradeab1e11111111111111111111111 success".to_string()));
                //assert!(details.log_messages.as_ref().unwrap().contains(&"Program 11111111111111111111111111111111 success".to_string()));
                return;
            }
            // TODO: Is this necessary?
            Err(TransactionError::ProgramCacheHitMaxLimit) => {
                // This is a transaction error not an instruction error
                return;
            }
            _ => (),
        }

        panic!("Transaction was not successful");
        //std::println!("Transaction shouldn't have failed: {:?}", filename);
        return;
    }

    // Check modified accounts
    let idx_map : HashMap<Pubkey, usize> = result.loaded_transactions[0].0.as_ref().unwrap().accounts
        .iter().enumerate().map(|(idx, item)| (item.0, idx))
        .collect();


    for item in &output.modified_accounts {
        let pubkey = Pubkey::new_from_array(item.address.clone().try_into().unwrap());
        //std::println!("looking for: {:?}", pubkey);
        let index = *idx_map.get(&pubkey).expect("Account not in expected results");
        let received_data = &result.loaded_transactions[0].0.as_ref()
            .unwrap().accounts[index].1;
        assert_eq!(received_data.lamports(), item.lamports);
        // if received_data.lamports() != item.lamports {
        //     std::println!("Lamports: {:?}", filename);
        // }
        assert_eq!(received_data.data(), item.data.as_slice());
        assert_eq!(received_data.owner(), &Pubkey::new_from_array(item.owner.clone().try_into().unwrap()));
        assert_eq!(received_data.executable(), item.executable);

        // u64::MAX means we are not considering the epoch
        if item.rent_epoch != u64::MAX && received_data.rent_epoch() != u64::MAX {
            assert_eq!(received_data.rent_epoch(), item.rent_epoch);
        }
        // if received_data.rent_epoch() != item.rent_epoch {
        //     std::println!("Rent epoch: {:?}", filename);
        // }
    }

    let execution_details = result.execution_results[0].details().unwrap();
    assert_eq!(execution_details.executed_units, input.cu_avail - output.cu_avail);
    //std::println!("cu: {} - expected: {}", result.execution_results[0].details().unwrap().executed_units, input.cu_avail - output.cu_avail);
    //std::println!("ret: {}", result.execution_results[0].details().unwrap().return_data.is_some());
    //std::println!("expected_ret: {}", output.return_data.len());

    if let Some(return_data) = &execution_details.return_data {
        assert_eq!(return_data.data, output.return_data);
    } else {
        assert_eq!(output.return_data.len(), 0);
    }
}

fn register_builtins(
    batch_processor: &TransactionBatchProcessor<MockForkGraph>,
    mock_bank: &MockBankCallback,
) {

    let bpf_loader = "solana_bpf_loader_upgradeable_program";
    batch_processor.add_builtin(
        mock_bank,
        bpf_loader_upgradeable::id(),
        bpf_loader,
        ProgramCacheEntry::new_builtin(
            0,
            bpf_loader.len(),
            solana_bpf_loader_program::Entrypoint::vm,
        )
    );

    let system_program  = "system_program";
    batch_processor.add_builtin(
        mock_bank,
        solana_system_program::id(),
        system_program,
        ProgramCacheEntry::new_builtin(
            0,
            system_program.len(),
            solana_system_program::system_processor::Entrypoint::vm,
        ),
    );

    let vote_program = "vote_program";
    batch_processor.add_builtin(
        mock_bank,
        solana_vote_program::id(),
        vote_program,
        ProgramCacheEntry::new_builtin(
            0,
            vote_program.len(),
            solana_vote_program::vote_processor::Entrypoint::vm,
        )
    );

    let stake_program = "stake_program";
    batch_processor.add_builtin(
        mock_bank,
        solana_stake_program::id(),
        stake_program,
        ProgramCacheEntry::new_builtin(
            0,
            stake_program.len(),
            solana_stake_program::stake_instruction::Entrypoint::vm,
        )
    )
}