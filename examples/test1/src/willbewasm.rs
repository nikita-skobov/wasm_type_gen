use super::*;

#[allow(dead_code)]
pub fn wasm_main(mything: &mut Thing) {
    mything.s = "message from wasm!".into();
}
