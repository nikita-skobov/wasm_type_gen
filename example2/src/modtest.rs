///! This file shows an example of how to write wasm_modules.
///! The convenience here is we get type hints by providing `#[wasm_meta]` at the top of this file.
///! This just inlines the types that wasm_module writers care about such as LibraryObj, and UserData.
///! After writing this file w/ type hints, you would copy it over to your `wasm_modules/` directory
///! in order to use it as a wasm module.
///! Note: we use #[allow(dead_code)] everywhere below! This is because the code is unused from the main program, but
///! rather is compiled separately. This is not necessary, but it avoids warnings when editing which is nice
use example2_derive::wasm_meta;
#[wasm_meta]
const _: () = ();
// everything below this line can be copied into wasm_modules:

/// doc comments work!
#[allow(dead_code)]
pub struct MyStruct {
    /// this value can only be 2!
    /// this is a simple example, but it shows
    /// how you can enforce value checks dynamically
    pub apples: u32,
}

#[allow(dead_code)]
pub type ExportType = MyStruct;

#[allow(dead_code)]
pub fn wasm_entrypoint(obj: &mut LibraryObj, cb: fn(&mut MyStruct)) {
    let mut mystuff = MyStruct {
        apples: 2,
    };
    cb(&mut mystuff);
    if mystuff.apples != 2 {
        obj.compile_error("apples must be 2");
    }
}
