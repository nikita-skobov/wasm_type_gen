///! Instructions:
///! ```
///! cd example2/
///! mkdir -p wasm_modules
///! cp ./src/modtest.rs ./wasm_modules/
///! cargo build
///! ```
///! Edit this file, and try to change obj.apples to anything but 2.
///! Then look at src/modtest.rs to see why it works.
///! Every time you change modtest.rs you must copy it over to ./wasm_modules/

use example2_derive::{wasm_meta, wasm_modules};

// This macro expects to find a mymod.rs file in ./wasm_modules/
wasm_modules!("mymod.rs");

mod modtest;

// 'mymod' must match the filename of the module
// imported with wasm_modules!()
// Notice how we have full type hints, and doc comments
// for a type that was dynamically inlined from the wasm module
#[wasm_meta(|obj: &mut mymod::MyStruct| {
    obj.apples = 2;
})]
pub struct Something {
    pub a: u64,
}
