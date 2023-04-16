use example2_derive::{wasm_meta, wasm_modules};

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

fn main() {
    let thing = Something {a: 6};
}
