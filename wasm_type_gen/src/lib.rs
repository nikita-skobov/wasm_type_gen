use std::{path::PathBuf, process::{Command, Stdio}, io::{Write, Read}};

use wasm_type_gen_derive::{generate_parsing_traits};
pub use wasm_type_gen_derive::WasmTypeGen;
pub use wasm_type_gen_derive::{output_and_stringify, output_and_stringify_basic};
use wasmtime::*;

generate_parsing_traits!();

pub fn compile_file_to_wasm(s: &str, add_to_code: Option<String>) -> Result<String, String> {
    let path = PathBuf::from(s);
    let file_data = std::fs::read_to_string(&path).map_err(|e| format!("Failed to read {:?} file\n{:?}", path, e))?;

    let file_stem = path.file_stem().ok_or("Failed to get .rs file name")?.to_string_lossy().to_string();
    compile_string_to_wasm(&file_stem, &file_data, add_to_code)
}

pub fn compile_string_to_wasm(wasm_out_name: &str, file_data: &str, add_to_code: Option<String>) -> Result<String, String> {
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

    let reader = std::io::BufReader::new(file_data.as_bytes());
    let hash = adler32::adler32(reader).unwrap_or(0);

    let wasm_last_name = if wasm_out_name.is_empty() {
        "last.wasm".to_string()
    } else {
        format!("{wasm_out_name}.last.wasm")
    };
    let wasm_out_name = if wasm_out_name.is_empty() {
        format!("{hash}.wasm")
    } else {
        format!("{wasm_out_name}.{hash}.wasm")
    };

    let wasm_output_base = std::env::var("CARGO_MANIFEST_DIR").unwrap_or(".".into());
    let wasm_out_dir = format!("{}/wasmout", wasm_output_base);
    // skip compilation if file already exists
    let module_path = format!("{}/{}", wasm_out_dir, wasm_out_name);
    let last_module_destination = format!("{}/{}", wasm_out_dir, wasm_last_name);
    // if we have a previously compiled module, store it so we can return this if the current compilation fails
    let last_module_path = if std::fs::File::open(&last_module_destination).is_ok() {
        Some(last_module_destination.clone())
    } else {
        None
    };

    if std::fs::File::open(&module_path).is_ok() {
        // if we are re-using an already compiled wasm file, then
        // we should set this to be the last.wasm for the next compilation
        let _ = std::fs::copy(&module_path, &last_module_destination);
        return Ok(module_path)
    }

    // if this is being compiled by rust analyzer, its for a keystroke, and
    // not something we usually want to fully compile. When the user saves the file, this
    // env var is (hopefully!) not present, and then we will run a normal compile.
    // otherwise, if we detect this, AND we have a last.wasm file, then just return that
    if std::env::var("RUST_ANALYZER_INTERNALS_DO_NOT_USE").is_ok() {
        if let Some(last) = last_module_path {
            return Ok(last);
        }
    }

    let _ = std::fs::create_dir(wasm_out_dir);
    // for debugging:
    // let mut f = std::fs::File::options().create(true).append(true).open("./compilationlog.txt").unwrap();
    // let pkg_name = std::env::var("CARGO_PKG_NAME").unwrap_or("UNKNOWN".into());
    // let mut s = format!("COMPILING {pkg_name} {hash}\n---------\n{file_data}\n\n");
    // for (key, val) in std::env::vars() {
    //     s.push_str(&format!("{key} = {val}\n"));
    // }
    // let _ = f.write_all(s.as_bytes());
    let cmd_resp = Command::new("rustc")
        // .arg(s) // can compile by pointing to a file. but for out purposes we want to use stdin
        .arg("--target").arg("wasm32-unknown-unknown")
        .arg("--crate-type=cdylib")
        .arg("-C").arg("debuginfo=0")
        .arg("-C").arg("opt-level=3")
        .arg("-C").arg("debug-assertions=off")
        .arg("-C").arg("codegen-units=1")
        .arg("-C").arg("linker-plugin-lto=yes")
        .arg("-C").arg("strip=symbols")
        .arg("-C").arg("lto=yes")
        .arg("-o").arg(module_path.as_str())
        .arg("-")
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn();
    let mut cmd = match cmd_resp {
        Ok(c) => c,
        Err(e) => {
            if let Some(last) = last_module_path {
                return Ok(last)
            }
            return Err(format!("Failed to invoke rustc {:?}", e));
        }
    };
    if let Some(mut stdin) = cmd.stdin.take() {
        if let Err(e) = stdin.write_all(file_data.as_bytes()) {
            if let Some(last) = last_module_path {
                return Ok(last)
            }
            return Err(format!("Failed to write stdin for rustc invocation\n{:?}", e));
        }
    }

    let output = match cmd.wait() {
        Ok(o) => o,
        Err(e) => {
            if let Some(last) = last_module_path {
                return Ok(last)
            }
            return Err(format!("Failed to compile to wasm\n{:?}", e));
        }
    };
    if !output.success() {
        let mut err = String::new();
        if let Some(mut out) = cmd.stderr.take() {
            if let Err(e) = out.read_to_string(&mut err) {
                if let Some(last) = last_module_path {
                    return Ok(last)
                }
                return Err(format!("Failed to get stderr after failure to compile wasm\n{:?}", e));
            }
        }
        if let Some(last) = last_module_path {
            return Ok(last)
        }
        return Err(format!("Failed to compile wasm module\n{}", err));
    }

    // copy successful path to the last path
    let _ = std::fs::copy(&module_path, &last_module_destination);

    Ok(module_path)
}

pub fn compile_and_run_wasm<T: FromBinarySlice + ToBinarySlice + WasmIncludeString>(
    path_to_rs_wasm_file: &str,
    data_to_pass: &T,
) -> Result<T, String> {
    // code generation / compilation
    let mut add_to_code = T::include_in_rs_wasm();
    add_to_code.push_str(T::gen_entrypoint());
    // this got generated by generate_parsing_traits!()
    add_to_code.push_str(WASM_PARSING_TRAIT_STR);
    let wasm_path = compile_file_to_wasm(path_to_rs_wasm_file, Some(add_to_code))?;
    let mut wasm_f = std::fs::File::open(wasm_path).map_err(|e| format!("Failed to open wasm file {:?}", e))?;
    let mut wasm_data = vec![];
    wasm_f.read_to_end(&mut wasm_data).map_err(|e| format!("Failed to read wasm file {:?}", e))?;

    let mut serialized_data = vec![];
    data_to_pass.add_to_slice(&mut serialized_data);

    let serialized_data = run_wasm(&wasm_data, serialized_data)?;
    let mut index = 0;
    let out = T::get_from_slice(&mut index, &serialized_data);
    match out {
        Some(o) => Ok(o),
        None => Err(format!("Failed to deserialize output from wasm guest")),
    }
}

pub fn run_wasm(
    wasm_data: &[u8],
    serialized_data: Vec<u8>,
) -> Result<Vec<u8>, String> {
    // linking (giving wasm guest access to host functions)
    let engine = Engine::default();
    let module = Module::from_binary(&engine, &wasm_data).map_err(|e| format!("failed to load wasm module {:?}", e))?;
    let mut linker: Linker<_> = Linker::new(&engine);
    linker.func_wrap("env", "get_entrypoint_alloc_size", |caller: Caller<'_, _>| -> u32 {
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
    linker.func_wrap("env", "set_entrypoint_data", |mut caller: Caller<'_, _>, ptr: u32, len: u32| {
        let ptr = ptr as usize;
        let len = len as usize;
        let output = if let Some(Extern::Memory(mem)) = caller.get_export("memory") {
            let mem_data = mem.data_mut(&mut caller);
            if let Some(data) = mem_data.get_mut(ptr..ptr+len) {
                Some(data.to_vec())
            } else {
                None
            }
        } else { None };
        if let Some(out) = output {
            let host_data: &mut Vec<u8> = caller.data_mut();
            *host_data = out;
        }
    }).unwrap();

    // instantiation, setting our main data entrypoint, calling wasm entry
    let mut store: Store<_> = Store::new(&engine, serialized_data);
    let instance = linker.instantiate(&mut store, &module).unwrap();
    let func = instance.get_typed_func::<(), u32>(&mut store, "wasm_entrypoint").unwrap();
    let res = func.call(&mut store, ()).unwrap();
    if res != 0 {
        return Err("Failed to deserialize data from host to wasm guest".into());
    }
    let out_data = store.into_data();
    Ok(out_data)
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
    fn ser_deser_works_unnamed_struct() {
        #[derive(WasmTypeGen, PartialEq, Debug)]
        pub struct Abc(u32, Vec<Option<Vec<u32>>>, u32);

        let item = Abc(0, vec![
            None, Some(vec![1, 2, 3]),
            Some(vec![]), Some(vec![4,5,6]),
            None, None, None
        ], 2);
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

    #[test]
    fn works_for_i8() {
        #[derive(WasmTypeGen, PartialEq, Debug)]
        pub struct Abc {
            pub d1: i8,
            pub d2: i8,
        }
        let item = Abc {
            d1: 127,
            d2: -128,
        };
        // does ser work?
        let data = item.to_binary_slice();
        assert!(data.len() > 0);
        // now deser:
        let item2 = Abc::from_binary_slice(data).expect("Expected deser to work");
        assert_eq!(item2, item2);
    }

    #[test]
    fn works_for_u8() {
        #[derive(WasmTypeGen, PartialEq, Debug)]
        pub struct Abc {
            pub d1: u8,
            pub d2: u8,
        }
        let item = Abc {
            d1: 0,
            d2: 255,
        };
        // does ser work?
        let data = item.to_binary_slice();
        assert!(data.len() > 0);
        // now deser:
        let item2 = Abc::from_binary_slice(data).expect("Expected deser to work");
        assert_eq!(item2, item2);
    }

    #[test]
    fn works_for_i16() {
        #[derive(WasmTypeGen, PartialEq, Debug)]
        pub struct Abc {
            pub d1: i16,
            pub d2: i16,
        }
        let item = Abc {
            d1: i16::MAX,
            d2: i16::MIN,
        };
        // does ser work?
        let data = item.to_binary_slice();
        assert!(data.len() > 0);
        // now deser:
        let item2 = Abc::from_binary_slice(data).expect("Expected deser to work");
        assert_eq!(item2, item2);
    }

    #[test]
    fn works_for_u16() {
        #[derive(WasmTypeGen, PartialEq, Debug)]
        pub struct Abc {
            pub d1: u16,
            pub d2: u16,
        }
        let item = Abc {
            d1: u16::MAX,
            d2: u16::MIN,
        };
        // does ser work?
        let data = item.to_binary_slice();
        assert!(data.len() > 0);
        // now deser:
        let item2 = Abc::from_binary_slice(data).expect("Expected deser to work");
        assert_eq!(item2, item2);
    }

    #[test]
    fn works_for_i32() {
        #[derive(WasmTypeGen, PartialEq, Debug)]
        pub struct Abc {
            pub d1: i32,
            pub d2: i32,
        }
        let item = Abc {
            d1: i32::MAX,
            d2: i32::MIN,
        };
        // does ser work?
        let data = item.to_binary_slice();
        assert!(data.len() > 0);
        // now deser:
        let item2 = Abc::from_binary_slice(data).expect("Expected deser to work");
        assert_eq!(item2, item2);
    }

    #[test]
    fn works_for_i64() {
        #[derive(WasmTypeGen, PartialEq, Debug)]
        pub struct Abc {
            pub d1: i64,
            pub d2: i64,
        }
        let item = Abc {
            d1: i64::MAX,
            d2: i64::MIN,
        };
        // does ser work?
        let data = item.to_binary_slice();
        assert!(data.len() > 0);
        // now deser:
        let item2 = Abc::from_binary_slice(data).expect("Expected deser to work");
        assert_eq!(item2, item2);
    }

    #[test]
    fn works_for_u64() {
        #[derive(WasmTypeGen, PartialEq, Debug)]
        pub struct Abc {
            pub d1: u64,
            pub d2: u64,
        }
        let item = Abc {
            d1: u64::MAX,
            d2: u64::MIN,
        };
        // does ser work?
        let data = item.to_binary_slice();
        assert!(data.len() > 0);
        // now deser:
        let item2 = Abc::from_binary_slice(data).expect("Expected deser to work");
        assert_eq!(item2, item2);
    }

    #[test]
    fn works_for_i128() {
        #[derive(WasmTypeGen, PartialEq, Debug)]
        pub struct Abc {
            pub d1: i128,
            pub d2: i128,
        }
        let item = Abc {
            d1: i128::MAX,
            d2: i128::MIN,
        };
        // does ser work?
        let data = item.to_binary_slice();
        assert!(data.len() > 0);
        // now deser:
        let item2 = Abc::from_binary_slice(data).expect("Expected deser to work");
        assert_eq!(item2, item2);
    }

    #[test]
    fn works_for_u128() {
        #[derive(WasmTypeGen, PartialEq, Debug)]
        pub struct Abc {
            pub d1: u128,
            pub d2: u128,
        }
        let item = Abc {
            d1: u128::MAX,
            d2: u128::MIN,
        };
        // does ser work?
        let data = item.to_binary_slice();
        assert!(data.len() > 0);
        // now deser:
        let item2 = Abc::from_binary_slice(data).expect("Expected deser to work");
        assert_eq!(item2, item2);
    }

    #[test]
    fn works_for_isize() {
        #[derive(WasmTypeGen, PartialEq, Debug)]
        pub struct Abc {
            pub d1: isize,
            pub d2: isize,
        }
        let item = Abc {
            d1: isize::MAX,
            d2: isize::MIN,
        };
        // does ser work?
        let data = item.to_binary_slice();
        assert!(data.len() > 0);
        // now deser:
        let item2 = Abc::from_binary_slice(data).expect("Expected deser to work");
        assert_eq!(item2, item2);
    }

    #[test]
    fn works_for_usize() {
        #[derive(WasmTypeGen, PartialEq, Debug)]
        pub struct Abc {
            pub d1: usize,
            pub d2: usize,
        }
        let item = Abc {
            d1: usize::MAX,
            d2: usize::MIN,
        };
        // does ser work?
        let data = item.to_binary_slice();
        assert!(data.len() > 0);
        // now deser:
        let item2 = Abc::from_binary_slice(data).expect("Expected deser to work");
        assert_eq!(item2, item2);
    }

    #[test]
    fn works_for_f32() {
        #[derive(WasmTypeGen, PartialEq, Debug)]
        pub struct Abc {
            pub d1: f32,
            pub d2: f32,
        }
        let item = Abc {
            d1: f32::MAX,
            d2: f32::MIN,
        };
        // does ser work?
        let data = item.to_binary_slice();
        assert!(data.len() > 0);
        // now deser:
        let item2 = Abc::from_binary_slice(data).expect("Expected deser to work");
        assert_eq!(item2, item2);
    }

    #[test]
    fn works_for_f64() {
        #[derive(WasmTypeGen, PartialEq, Debug)]
        pub struct Abc {
            pub d1: f64,
            pub d2: f64,
        }
        let item = Abc {
            d1: f64::MAX,
            d2: f64::MIN,
        };
        // does ser work?
        let data = item.to_binary_slice();
        assert!(data.len() > 0);
        // now deser:
        let item2 = Abc::from_binary_slice(data).expect("Expected deser to work");
        assert_eq!(item2, item2);
    }

    #[test]
    fn works_for_bool() {
        #[derive(WasmTypeGen, PartialEq, Debug)]
        pub struct Abc {
            pub d1: bool,
            pub d2: bool,
        }
        let item = Abc {
            d1: false,
            d2: true,
        };
        // does ser work?
        let data = item.to_binary_slice();
        assert!(data.len() > 0);
        // now deser:
        let item2 = Abc::from_binary_slice(data).expect("Expected deser to work");
        assert_eq!(item2, item2);
    }

    #[test]
    fn works_for_char() {
        #[derive(WasmTypeGen, PartialEq, Debug)]
        pub struct Abc {
            pub d1: char,
            pub d2: char,
        }
        let item = Abc {
            d1: '😻',
            d2: 'a',
        };
        // does ser work?
        let data = item.to_binary_slice();
        assert!(data.len() > 0);
        // now deser:
        let item2 = Abc::from_binary_slice(data).expect("Expected deser to work");
        assert_eq!(item2, item2);
    }

    #[test]
    fn works_for_array() {
        #[derive(WasmTypeGen, PartialEq, Debug, Copy, Clone)]
        pub struct Something {
            pub a: u32,
        }
        #[derive(WasmTypeGen, PartialEq, Debug)]
        pub struct Abc {
            pub d1: [u8; 4],
            pub d2: [Something; 3],
        }
        let item = Abc {
            d1: [1, 2, 3, 4],
            d2: [Something { a: 1}, Something { a: 2}, Something { a: 3}],
        };
        // does ser work?
        let data = item.to_binary_slice();
        assert!(data.len() > 0);
        // now deser:
        let item2 = Abc::from_binary_slice(data).expect("Expected deser to work");
        assert_eq!(item2, item2);
        // ensure that generated code for wasm includes type def of Something
        assert!(Abc::include_in_rs_wasm().contains("pub struct Something"));
    }
}
