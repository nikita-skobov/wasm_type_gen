use super::*;

#[allow(dead_code)]
pub fn wasm_main(mything: &mut Thing) -> u32 {
    if mything.opt.is_some() {
        1
    } else {
        100
    }
}
