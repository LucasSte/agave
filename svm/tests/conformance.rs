use std::env;
use std::fs::File;
use std::io::Read;

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

}