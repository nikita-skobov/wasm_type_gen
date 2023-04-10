// super::my_gen_stuff_macro!();

// use super::wasmtypegen_wasm;



// user writes:
use super::*;
// wasmtypegen_wasm!();

pub fn wasm_main(mything: Thing) -> u32 {
    // mything.o.abc

    if mything.opt.is_some() {
        1
    } else {
        100
    }
}
// end of user writes

// #[no_mangle]
// pub extern fn entrypoint() -> u32 {
//     let mything = unsafe {
//         let len = get_entrypoint_alloc_size() as usize;
//         let mut data: Vec<u8> = Vec::with_capacity(len);
//         data.set_len(len);
//         let ptr = data.as_ptr();
//         let len = data.len();
//         get_entrypoint_data(ptr, len as _);
//         Thing::from_binary_slice(data).unwrap()
//     };

//     wasm_main(mything)
// }

// #[repr(C)]
// pub struct Thing {
//     pub s: String,
//     pub q: u32,
// }

// impl Thing {
//     pub fn iter_fields<F: FnMut(bool, usize, *const u8, usize) -> usize>(&mut self, mut cb: F) {
//         let z: *const u8 = &0;
//         let size = cb(true, 0, z, 0);
//         self.s = " ".repeat(size);

//         let s = &mut self.s;
//         let s_bytes = s.as_bytes();
//         cb(false, 0, s_bytes.as_ptr(), s_bytes.len());

//         let q = &mut self.q;
//         let q_bytes = q.to_be_bytes();
//         cb(false, 1, q_bytes.as_ptr(), q_bytes.len());
//     }
// }



// pub trait ToBinarySlice {
//     fn add_to_slice(&self, data: &mut Vec<u8>);
// }

// pub trait FromBinarySlice {
//     fn get_from_slice(data: &[u8]) -> Option<Self> where Self: Sized;
// }

// impl ToBinarySlice for String {
//     fn add_to_slice(&self, data: &mut Vec<u8>) {
//         let self_bytes = self.as_bytes();
//         let len_u32 = self_bytes.len() as u32;
//         let len_be_bytes = len_u32.to_be_bytes();
//         data.extend(len_be_bytes);
//         data.extend(self_bytes);
//     }
// }

// impl ToBinarySlice for u32 {
//     fn add_to_slice(&self, data: &mut Vec<u8>) {
//         let self_bytes = self.to_be_bytes();
//         let len_u32 = self_bytes.len() as u32;
//         let len_be_bytes = len_u32.to_be_bytes();
//         data.extend(len_be_bytes);
//         data.extend(self_bytes);
//     }
// }

// // // #[derive(MyThing)]
// // impl ToBinarySlice for Thing {
// //     fn add_to_slice(&self, data: &mut Vec<u8>) {
// //         let mut self_data = vec![];
// //         self.s.add_to_slice(&mut self_data);
// //         self.q.add_to_slice(&mut self_data);
// //         let self_data_len = self_data.len() as u32;
// //         let self_data_bytes = self_data_len.to_be_bytes();
// //         data.extend(self_data_bytes);
// //         data.extend(self_data);
// //     }
// // }

// impl FromBinarySlice for u32 {
//     fn get_from_slice(data: &[u8]) -> Option<Self> {
//         Some(u32::from_be_bytes([data[0], data[1], data[2], data[3]]))
//     }
// }

// impl FromBinarySlice for String {
//     fn get_from_slice(data: &[u8]) -> Option<Self> {
//         Some(String::from_utf8_lossy(data).to_string())
//     }
// }

// // impl FromBinarySlice for Thing {
// //     #[allow(unused_assignments)]
// //     fn get_from_slice(data: &[u8]) -> Option<Self> {
// //         let mut index = 0;

// //         let first_4 = data.get(index..index + 4)?;
// //         index += 4;
// //         let first_4_u32_bytes = [first_4[0], first_4[1], first_4[2], first_4[3]];
// //         let len = u32::from_be_bytes(first_4_u32_bytes) as usize;
// //         let next_data = data.get(index..index + len)?;
// //         index += len;
// //         let s: String = <_>::get_from_slice(next_data)?;
        
// //         let first_4 = data.get(index..index + 4)?;
// //         index += 4;
// //         let first_4_u32_bytes = [first_4[0], first_4[1], first_4[2], first_4[3]];
// //         let len = u32::from_be_bytes(first_4_u32_bytes) as usize;
// //         let next_data = data.get(index..index + len)?;
// //         index += len;
// //         let q: u32 = <_>::get_from_slice(next_data)?;
// //         Some(Self {
// //             s, q
// //         })
// //     }
// // }

// // impl Thing {
// //     pub fn to_binary_slice(&self) -> Vec<u8> {
// //         let mut out = vec![];
// //         self.add_to_slice(&mut out);
// //         out
// //     }
// //     #[allow(unused_assignments)]
// //     pub fn from_binary_slice(data: Vec<u8>) -> Option<Self> {
// //         let mut index = 0;
// //         let first_4 = data.get(index..index + 4)?;
// //         index += 4;
// //         let first_4_u32_bytes = [first_4[0], first_4[1], first_4[2], first_4[3]];
// //         let len = u32::from_be_bytes(first_4_u32_bytes) as usize;
// //         let next_data = data.get(index..index + len)?;
// //         index += len;
// //         let out: Self = <_>::get_from_slice(next_data)?;
// //         Some(out)
// //     }
// // }




// extern "C" {
//     // fn get_field(field_index: u32, ptr: *const u8, len: u32);
//     // fn get_field_size(field_index: u32) -> u32;

//     fn get_entrypoint_alloc_size() -> u32;
//     fn get_entrypoint_data(ptr: *const u8, len: u32);

//     // fn do_something_with_struct(index: u32);
//     // fn get_data() -> u64;
//     // fn println(ptr: *const u8, len: usize);
//     // fn give_string() -> *const u8;
//     // fn set_func_name(index: u32, );
// }