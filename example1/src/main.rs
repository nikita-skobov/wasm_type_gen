//! This example shows how to use wasm_type_gen in its most essential form.
//! We do a few things:
//! - we define the host program (this file)
//! - we define a guest program (willbewasm.rs)
//! - the host program compiles the guest program at runtime into a wasm module
//! - then the host program loads it, and passes the data to it.
//! 
//! Instructions:
//! ```
//! cd ./example1/
//! cargo run -- ./src/willbewasm.rs
//! ```


use wasm_type_gen::*;
use wasm_type_gen::WasmTypeGen;

// This is interesting! We mod willbewasm even though it's not strictly necessary.
// The purpose of this is that while editing willbewasm.rs in our editor, we will get intellisense and compiler
// warnings/suggestions as we code, even though this file will eventually be compiled to wasm.
// At compile time we remove the strict linkage and replace with dynamic ser/deserialization (see compile_and_run_wasm for details)
mod willbewasm;

// The WasmTypeGen derives serialization and deserialization code that is used:
// - to serialize Thing into a binary Vec<u8> that gets passed to the guest wasm module
// - the deserialization code is actually embedded into the wasm such that the wasm module can load
//   this type from Vec<u8> -> Thing
#[derive(WasmTypeGen, Debug)]
pub struct Thing {
    pub s: String,
    pub q: u32,
    pub opt: Option<u32>,
}


fn main() {
    let path_to_file = std::env::args().nth(1).expect("Must provide path to .rs file to be compiled to wasm");

    let pass_this = Thing {
        s: "hellofromrusttowasm!".into(),
        q: 101,
        opt: Some(2),
    };
    let out_data = compile_and_run_wasm(&path_to_file, &pass_this).unwrap();
    println!("{:?}", out_data);
}
