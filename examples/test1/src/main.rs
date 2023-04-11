use std::{path::PathBuf, process::{Command, Stdio}, io::{Write, Read}};

use wasm_type_gen::{WasmTypeGen, generate_wasm_entrypoint, generate_parsing_traits};
use wasmtime::*;

mod willbewasm;

fn compile_to_wasm(s: &str, add_to_code: Option<String>) -> Result<String, String> {
    let path = PathBuf::from(s);
    let file_data = std::fs::read_to_string(&path).map_err(|e| format!("Failed to read {:?} file\n{:?}", path, e))?;

    // to get IDE hints in our editor, our .rs file that will be turned into a .wasm file
    // must import the types that it references.
    // however, we wish to compile only a single file, and thus have no way of handling imports / linking.
    // so our hack is thus:
    // we remove `use super::*` and we add in the referenced code as text directly to the code before compiling.
    // this is why we compile via stdin rather than from a file: because we can modify the code in memory
    // rather than needing to modify the user's actual code on disk.
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
    let path_to_file = std::env::args().nth(1).expect("Must provide path to .rs file to be compiled to wasm");

    // code generation / compilation
    let mut add_to_code = Thing::include_in_rs_wasm();
    add_to_code.push_str(generate_wasm_entrypoint!(Thing));
    add_to_code.push_str(PARSING_TRAIT_STR);
    let wasm_path = match compile_to_wasm(&path_to_file, Some(add_to_code)) {
        Err(e) => panic!("Failed to compile to wasm {e}"),
        Ok(o) => o,
    };

    // linking (giving wasm guest access to host functions)
    let engine = Engine::default();
    let module = Module::from_file(&engine, wasm_path).expect("failed to read wasm file");
    let mut linker: Linker<_> = Linker::new(&engine);
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

    // instantiation, setting our main data entrypoint, calling wasm entry
    let pass_this = Thing {
        s: "hellofromrusttowasm!".into(),
        q: 101,
        opt: Some(2),
    };
    let mut store: Store<_> = Store::new(&engine, pass_this.to_binary_slice());
    let instance = linker.instantiate(&mut store, &module).unwrap();
    let func = instance.get_typed_func::<(), u32>(&mut store, "wasm_entrypoint").unwrap();
    let res = func.call(&mut store, ());
    println!("{:?}", res);
}


generate_parsing_traits!();

#[derive(WasmTypeGen)]
pub struct Thing {
    pub s: String,
    pub q: u32,
    pub opt: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ser_deser_works() {
        #[derive(WasmTypeGen)]
        pub struct Abc {
            pub u1: u32,
            pub s: String,
            pub u2: u32,
        }
        let item = Abc {
            s: "abcd".into(),
            // o: Other { abc: 2 },
            u1: 23,
            u2: 42,
        };
        // does ser work?
        let data = item.to_binary_slice();
        assert!(data.len() > 0);

        // now deser:
        let item2 = Abc::from_binary_slice(data).expect("Expected deser to work");
        assert_eq!(item2.u1, item2.u1);
        assert_eq!(item2.s, item2.s);
        assert_eq!(item2.u2, item2.u2);
    }

    #[test]
    fn ser_deser_works_other_types() {
        #[derive(WasmTypeGen, PartialEq, Debug)]
        pub struct Abc {
            pub child: Abc2,
        }

        #[derive(WasmTypeGen, PartialEq, Debug)]
        pub struct Abc2 {
            pub u: u32,
        }
        let item = Abc {
            child: Abc2 { u: 1 },
        };
        // does ser work?
        let data = item.to_binary_slice();
        assert!(data.len() > 0);

        // now deser:
        let item2 = Abc::from_binary_slice(data).expect("Expected deser to work");
        assert_eq!(item2, item2);
    }

    #[test]
    fn ser_deser_works_options() {
        #[derive(WasmTypeGen, PartialEq, Debug)]
        pub struct Abc {
            pub a: u32,
            pub b: Option<u32>,
            pub child: Option<Abc2>,
        }

        #[derive(WasmTypeGen, PartialEq, Debug)]
        pub struct Abc2 {
            pub u: Option<u32>,
        }
        let item = Abc {
            a: 0,
            b: None,
            child: Some(Abc2 { u: Some(1) }),
        };
        // does ser work?
        let data = item.to_binary_slice();
        assert!(data.len() > 0);

        // now deser:
        let item2 = Abc::from_binary_slice(data).expect("Expected deser to work");
        assert_eq!(item2, item2);
    }

    #[test]
    fn works_for_enums() {
        #[derive(WasmTypeGen, PartialEq, Debug)]
        pub enum Abc {
            Unit,
            Named { x: u32, y: u32 },
            NonNamed(u32),
        }

        let item = Abc::Unit;
        // does ser work?
        let data = item.to_binary_slice();
        assert!(data.len() > 0);
        // now deser:
        let item2 = Abc::from_binary_slice(data).expect("Expected deser to work");
        assert_eq!(item2, item2);

        let item = Abc::Named { x: 2, y: 30 };
        // does ser work?
        let data = item.to_binary_slice();
        assert!(data.len() > 0);
        // now deser:
        let item2 = Abc::from_binary_slice(data).expect("Expected deser to work");
        assert_eq!(item2, item2);

        let item = Abc::NonNamed(100);
        // does ser work?
        let data = item.to_binary_slice();
        assert!(data.len() > 0);
        // now deser:
        let item2 = Abc::from_binary_slice(data).expect("Expected deser to work");
        assert_eq!(item2, item2);
    }

    #[test]
    fn works_for_option_of_enum() {
        #[derive(WasmTypeGen, PartialEq, Debug)]
        pub struct Abc {
            pub child: Option<Abc2>,
        }

        #[derive(WasmTypeGen, PartialEq, Debug)]
        pub enum Abc2 {
            Unit,
            Named { x: u32, y: u32 },
            NonNamed(u32),
        }

        let item = Abc {
            child: Some(Abc2::NonNamed(1)),
        };
        // does ser work?
        let data = item.to_binary_slice();
        assert!(data.len() > 0);
        // now deser:
        let item2 = Abc::from_binary_slice(data).expect("Expected deser to work");
        assert_eq!(item2, item2);
    }

    #[test]
    fn works_for_advanced_struct() {
        #[derive(WasmTypeGen, PartialEq, Debug)]
        pub struct Abc {
            pub child: Option<Abc2>,
            pub e_num: Abc3,
        }

        #[derive(WasmTypeGen, PartialEq, Debug)]
        pub struct Abc2 {
            pub a: u32,
        }

        #[derive(WasmTypeGen, PartialEq, Debug)]
        pub enum Abc3 {
            Child(Abc2),
            Nothing,
        }

        let item = Abc {
            child: Some(Abc2 { a: 2 }),
            e_num: Abc3::Child(Abc2 { a: 3 }),
        };
        // does ser work?
        let data = item.to_binary_slice();
        assert!(data.len() > 0);
        // now deser:
        let item2 = Abc::from_binary_slice(data).expect("Expected deser to work");
        assert_eq!(item2, item2);
    }

    #[test]
    fn works_for_vec() {
        #[derive(WasmTypeGen, PartialEq, Debug)]
        pub struct Abc {
            pub data: Vec<Abc2>,
        }
        #[derive(WasmTypeGen, PartialEq, Debug)]
        pub enum Abc2 {
            Child(u32),
            Nothing,
        }

        let item = Abc {
            data: vec![
                Abc2::Child(1),
                Abc2::Nothing,
                Abc2::Child(2),
            ],
        };
        // does ser work?
        let data = item.to_binary_slice();
        assert!(data.len() > 0);
        // now deser:
        let item2 = Abc::from_binary_slice(data).expect("Expected deser to work");
        assert_eq!(item2, item2);
        // ensure that generated code for wasm includes type def of Abc2
        assert!(Abc::include_in_rs_wasm().contains("pub enum Abc2"));
    }
}
