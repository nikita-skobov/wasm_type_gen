use super::*;

// it's technically dead code because this never gets called from main!
// but instead it gets called dynamically when we run it as wasm.
#[allow(dead_code)]
pub fn wasm_main(mything: &mut Thing) {
    mything.s = "message from wasm!".into();
}
