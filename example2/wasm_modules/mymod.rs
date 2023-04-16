/// doc comments work!
pub struct MyStruct {
    /// this value can only be 2!
    /// this is a simple example, but it shows
    /// how you can enforce value checks dynamically
    pub apples: u32,
}

pub type ExportType = MyStruct;

pub fn wasm_entrypoint(obj: &mut LibraryObj, cb: fn(&mut MyStruct)) {
    let mut mystuff = MyStruct {
        apples: 2,
    };
    cb(&mut mystuff);
    if mystuff.apples != 2 {
        obj.compile_error("apples must be 2");
    }
}
