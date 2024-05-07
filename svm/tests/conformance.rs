use std::env;
use std::fs::File;
use std::io::Read;
use prost::Message;
use solana_sdk::pubkey::Pubkey;

mod proto {
    include!(concat!(env!("OUT_DIR"), "/org.solana.sealevel.v1.rs"));
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
    dir.push("bpf-loader");
    dir.push("0c9471f50baa2b03.bin");

    let mut file = File::open(dir.clone()).expect("file not found");
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).expect("Failed to read file");

    let fixture = proto::InstrFixture::decode(buffer.as_slice()).unwrap();

    let program_id = fixture.input.as_ref().unwrap().program_id.clone();
    std::println!("program id: {:?}", Pubkey::new_from_array(program_id.try_into().unwrap()));

    for item in &fixture.input.as_ref().unwrap().accounts {
        std::println!("Acct: {:?}", Pubkey::new_from_array(item.address.clone().try_into().unwrap()));
    }

    for item in &fixture.input.as_ref().unwrap().instr_accounts {
        std::println!("idx: {}, writable: {}, signer: {}", item.index, item.is_writable, item.is_signer);
    }

    std::println!("Has txn context: {:?}", fixture.input.as_ref().unwrap().txn_context.is_some());
    std::println!("Has slot context: {:?}", fixture.input.as_ref().unwrap().slot_context.is_some());
    std::println!("Has epoch context: {:?}", fixture.input.as_ref().unwrap().epoch_context.is_some());
}