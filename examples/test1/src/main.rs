use std::{path::PathBuf, process::{Command, Stdio}, io::{Write, Read}};

use wasm_type_gen::{MyThing, generate_wasm_entrypoint, generate_parsing_traits};
use wasmtime::*;

mod willbewasm;

fn compile_to_wasm(s: &str, add_to_code: Option<String>) -> Result<String, String> {
    let path = PathBuf::from(s);
    let file_data = std::fs::read_to_string(&path).map_err(|e| format!("Failed to read {:?} file\n{:?}", path, e))?;
    
    let mut file_data = file_data.replace("use super::*;", "");
    if let Some(add) = add_to_code {
        file_data.push('\n');
        file_data.push_str(&add);
    }

    let file_stem = path.file_stem().ok_or("Failed to get .rs file name")?.to_string_lossy().to_string();
    let module_path = format!("./wasmout/{}.wasm", file_stem);
    let _ = std::fs::create_dir("./wasmout/");
    let mut cmd = Command::new("rustc")
        // .arg(s) // can compile by pointing to a file. but for out purposes we want to use stdin
        .arg("--target").arg("wasm32-unknown-unknown")
        .arg("--crate-type=cdylib")
        .arg("-C").arg("debuginfo=0")
        .arg("-C").arg("opt-level=3")
        .arg("-C").arg("debug-assertions=off")
        .arg("-C").arg("lto=yes")
        .arg("-o").arg(module_path.as_str())
        .arg("-")
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn().map_err(|e| format!("Failed to invoke rustc\n{:?}", e))?;
    if let Some(mut stdin) = cmd.stdin.take() {
        stdin.write_all(file_data.as_bytes())
            .map_err(|e| format!("Failed to write to stdin for rustc invocation\n{:?}", e))?;
    }

    let output = cmd.wait().map_err(|e| format!("Failed to run rustc\n{:?}", e))?;
    if !output.success() {
        let mut err = String::new();
        if let Some(mut out) = cmd.stderr.take() {
            out.read_to_string(&mut err).map_err(|e| format!("Failed to read stderr {:?}", e))?;
        }
        return Err(format!("Failed to compile wasm module\n{}", err));
    }
    Ok(module_path)
}

fn main() {
    println!("Hello, world!");
    let path_to_file = std::env::args().nth(1).expect("Must provide path to .rs file to be compiled to wasm");

    let mut add_to_code = Thing::include_in_rs_wasm();
    add_to_code.push_str(generate_wasm_entrypoint!(Thing));
    add_to_code.push_str(PARSING_TRAIT_STR);
    // println!("ENTRYPOINT!\n{entrypoint}");
    let wasm_path = match compile_to_wasm(&path_to_file, Some(add_to_code)) {
        Err(e) => panic!("Failed to compile to wasm {e}"),
        Ok(o) => o,
    };

    
    // unsafe {
        
    //     const SIZE: usize = std::mem::size_of::<Thing>();
    //     let mut mything_bytes = std::mem::transmute::<Thing, [u8; SIZE]>(mything);
    //     for b in mything_bytes.iter_mut() {
    //         if *b == 2 {
    //             *b = 3;
    //         }
    //     }
    //     let other_thing = std::mem::transmute::<[u8; SIZE], Thing>(mything_bytes);
    //     println!("OTHER THING {}", other_thing.q);
    // }

    let engine = Engine::default();
    let module = Module::from_file(&engine, wasm_path).expect("failed to read wasm file");
    let mut linker: Linker<_> = Linker::new(&engine);
    // linker.func_wrap("env", "get_field_size", |mut caller: Caller<'_, _>, field_index: u32| -> u32 {
    //     let mything = Thing { s: "Dsa".into(), q: 2 };
    //     let field_index = field_index as usize;
    //     mything.get_field_size(field_index) as _
    // }).unwrap();
    linker.func_wrap("env", "get_entrypoint_alloc_size", |mut caller: Caller<'_, _>| -> u32 {
        let data: &Vec<u8> = caller.data();
        data.len() as u32
    }).unwrap();
    linker.func_wrap("env", "get_entrypoint_data", |mut caller: Caller<'_, _>, ptr: u32, len: u32| {
        let ptr = ptr as usize;
        let len = len as usize;
        let host_data: &Vec<u8> = caller.data();
        if host_data.len() != len {
            return;
        }
        let host_data = host_data.clone();
        if let Some(Extern::Memory(mem)) = caller.get_export("memory") {
            let mem_data = mem.data_mut(&mut caller);
            if let Some(data) = mem_data.get_mut(ptr..ptr+len) {
                data.copy_from_slice(&host_data);
            }
        }
    }).unwrap();
    // linker.func_wrap("env", "get_field", |mut caller: Caller<'_, _>, field_index: u32, ptr: u32, len: u32| {
    //     if let Some(Extern::Memory(mem)) = caller.get_export("memory") {
    //         let field_index = field_index as usize;
    //         let ptr = ptr as usize;
    //         let len = len as usize;

    //         let mything = Thing { s: "Dsa".into(), q: 2 };


    //         // const SIZE: usize = std::mem::size_of::<Thing>();
    //         // if len != SIZE {
    //         //     println!("In wasm Thing is {}", len);
    //         //     println!("In rust Thing is {}", SIZE);
    //         //     panic!("REEEE. len is not size");
    //         // }
    //         let mem_data = mem.data_mut(&mut caller);
    //         if let Some(data) = mem_data.get_mut(ptr..ptr+len) {
    //             mything.set_data(field_index, data);
    //             // println!("{:?}", data);
    //             // let x = Thing {
    //             //     s: "dsadsa".into(),
    //             //     q: 3,
    //             // };

    //             // unsafe {
                    
    //             //     let mything_bytes = std::mem::transmute::<Thing, [u8; SIZE]>(x);
    //             //     for (index, b) in data.iter_mut().enumerate() {
    //             //         *b = mything_bytes[index];
    //             //     }
    //             // }
    //             // println!("{}", String::from_utf8_lossy(data).to_string());
    //         }
    //     }
    // }).unwrap();


    let pass_this = Thing {
        s: "hellofromrusttowasm!".into(),
        q: 101,
        opt: Some(2),
        // o: Other { abc: 2 },
    };


    // TODO: add host funcs via linker
    let mut store: Store<_> = Store::new(&engine, pass_this.to_binary_slice());
    let instance = linker.instantiate(&mut store, &module).unwrap();
    let func = instance.get_typed_func::<(), u32>(&mut store, "wasm_entrypoint").unwrap();
    let res = func.call(&mut store, ());
    println!("{:?}", res);


    // let x = Thing {
    //     s: "dsa".into(),
    //     q: 2,
    // };

    // let out_to_wasm = Thing::include_in_rs_wasm();
    // println!("I should transclude this into wasm:\n{out_to_wasm}");

    // let engine = Engine::default();
    // let module = Module::from_file(&engine, "./test1/something2.wasm").unwrap();
}


generate_parsing_traits!();

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

// // #[derive(MyThing)]
// impl ToBinarySlice for Thing {
//     fn add_to_slice(&self, data: &mut Vec<u8>) {
//         let mut self_data = vec![];
//         self.s.add_to_slice(&mut self_data);
//         self.q.add_to_slice(&mut self_data);
//         let self_data_len = self_data.len() as u32;
//         let self_data_bytes = self_data_len.to_be_bytes();
//         data.extend(self_data_bytes);
//         data.extend(self_data);
//     }
// }


// impl FromBinarySlice for Thing {
//     #[allow(unused_assignments)]
//     fn get_from_slice(data: &[u8]) -> Option<Self> {
//         let mut index = 0;

//         let first_4 = data.get(index..index + 4)?;
//         index += 4;
//         let first_4_u32_bytes = [first_4[0], first_4[1], first_4[2], first_4[3]];
//         let len = u32::from_be_bytes(first_4_u32_bytes) as usize;
//         let next_data = data.get(index..index + len)?;
//         index += len;
//         let s: String = <_>::get_from_slice(next_data)?;
        
//         let first_4 = data.get(index..index + 4)?;
//         index += 4;
//         let first_4_u32_bytes = [first_4[0], first_4[1], first_4[2], first_4[3]];
//         let len = u32::from_be_bytes(first_4_u32_bytes) as usize;
//         let next_data = data.get(index..index + len)?;
//         index += len;
//         let q: u32 = <_>::get_from_slice(next_data)?;
//         Some(Self {
//             s, q
//         })
//     }
// }

// impl Thing {
//     pub fn to_binary_slice(&self) -> Vec<u8> {
//         let mut out = vec![];
//         self.add_to_slice(&mut out);
//         out
//     }
//     #[allow(unused_assignments)]
//     pub fn from_binary_slice(data: Vec<u8>) -> Option<Self> {
//         let mut index = 0;
//         let first_4 = data.get(index..index + 4)?;
//         index += 4;
//         let first_4_u32_bytes = [first_4[0], first_4[1], first_4[2], first_4[3]];
//         let len = u32::from_be_bytes(first_4_u32_bytes) as usize;
//         let next_data = data.get(index..index + len)?;
//         index += len;
//         let out: Self = <_>::get_from_slice(next_data)?;
//         Some(out)
//     }
// }

#[derive(MyThing)]
pub struct Thing {
    pub s: String,
    pub q: u32,
    // pub o: Other,
    pub opt: Option<u32>,
}

// fn hello() {
//     let opt : Option < u32 > = None;
// }

// #[derive(MyThing)]
// pub struct Other {
//     pub abc: u32,
// }

// impl<T: ToBinarySlice> ToBinarySlice for Option<T> {
//     fn add_to_slice(&self, data: &mut Vec<u8>) {
//         match self {
//             Some(t) => {
//                 t.add_to_slice(data);
//             }
//             None => {
//                 data.extend([255, 255, 255, 255]);
//             }
//         }
//     }
// }

// impl<T: FromBinarySlice> FromBinarySlice for Option<T> {
//     fn get_from_slice(index: &mut usize, data: &[u8]) -> Option<Self>where Self:Sized {
//         let first_4 = data.get(*index..*index + 4)?;
//         let first_4_u32_bytes = [first_4[0], first_4[1], first_4[2], first_4[3]];
//         if first_4_u32_bytes == [255, 255, 255, 255] {
//             *index += 4;
//             return Some(None);
//         }
//         let t_thing = T::get_from_slice(index, data)?;
//         Some(Some(t_thing))
//     }
// }


// impl Thing {
//     pub fn set_data(&self, field_index: usize, data: &mut [u8]) {
//         let thing: Option<String> = None;
//         // let thing_ptr: *const Option<String> = &thing;
//         // let thing_ptr_bytes = thing_ptr.
//         let p = std::ptr::addr_of!(thing);
//         let pp: *const u8 = p.cast();
//         unsafe {
//             let x = pp.read();
//         }
//         match field_index {
//             0 => {
//                 // println!("In wasm you have {:?}. Which is {} bytes", data, data.len());
//                 // println!("In rust I have {:?}, Which is {} bytes", self.s.as_bytes(), self.s.as_bytes().len());
//                 if data.len() == self.s.len() {
//                     data.copy_from_slice(self.s.as_bytes());
//                 }
//             }
//             1 => {
//                 let q_bytes = self.q.to_be_bytes();
//                 if data.len() == q_bytes.len() {
//                     data.copy_from_slice(&q_bytes);
//                 }
//             }
//             _ => {}
//         }
//     }
//     pub fn get_field_size(&self, field_index: usize) -> usize {
//         match field_index {
//             0 => self.s.as_bytes().len(),
//             _ => 0,
//         }
//     }
// }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ser_deser_works() {
        let mything = Thing {
            s: "abcd".into(),
            // o: Other { abc: 2 },
            q: 23,
            opt: None,
        };
        // does ser work?
        let data = mything.to_binary_slice();
        assert!(data.len() > 0);

        // now deser:
        let mything2 = Thing::from_binary_slice(data).expect("Expected deser to work");
        assert_eq!(mything2.s, mything.s);
        assert_eq!(mything2.q, mything.q);
    }
}
