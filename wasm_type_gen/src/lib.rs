use std::{path::PathBuf, process::{Command, Stdio}, io::{Write, Read}, collections::{HashSet}, format};

use wasm_type_gen_derive::{generate_parsing_traits};
pub use wasm_type_gen_derive::WasmTypeGen;
pub use wasm_type_gen_derive::{output_and_stringify, output_and_stringify_basic, output_and_stringify_basic_const};
use wasmtime::*;

generate_parsing_traits!();

pub fn compile_file_to_wasm(s: &str, add_to_code: Option<String>) -> Result<String, String> {
    let path = PathBuf::from(s);
    let file_data = std::fs::read_to_string(&path).map_err(|e| format!("Failed to read {:?} file\n{:?}", path, e))?;

    let file_stem = path.file_stem().ok_or("Failed to get .rs file name")?.to_string_lossy().to_string();
    compile_string_to_wasm(&file_stem, &file_data, add_to_code, None)
}

pub fn format_file_contents(data: &str) -> Result<String, String> {
    let mut cmd = Command::new("rustfmt")
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn().map_err(|e| format!("Failed to invoke rustfmt {:?}", e))?;
    if let Some(mut stdin) = cmd.stdin.take() {
        stdin.write_all(data.as_bytes()).map_err(|e| format!("Failed to write stdin for rustc invocation\n{:?}", e))?;
    }

    let output = cmd.wait().map_err(|e| format!("Failed to run rustfmt\n{:?}", e))?;
    if !output.success() {
        let mut err = String::new();
        if let Some(mut out) = cmd.stderr.take() {
            out.read_to_string(&mut err).map_err(|e| format!("Failed to get stderr after failure to run rustfmt\n{:?}", e))?;
        }
        return Err(format!("Failed to run rustfmt\n{}", err));
    }
    let mut out = String::new();
    if let Some(mut stdout) = cmd.stdout.take() {
        stdout.read_to_string(&mut out).map_err(|e| format!("Failed to get stdout after running rusftmt\n{:?}", e))?;
    }
    Ok(out)
}

pub fn try_find_existing_extern_crate_file(output_dir: &str, dep_name: &str) -> Result<(String, String), String> {
    let expected_file = format!("{output_dir}/externloc_{dep_name}.txt");
    if let Ok(contents) = std::fs::read_to_string(&expected_file) {
        let actual_path = contents.trim().to_string();
        // ensure it still exists:
        if std::fs::File::open(&actual_path).is_ok() {
            return Ok((expected_file, actual_path));
        }
    }
    Err(expected_file)
}

/// given the name of a cargo dependency, use cargo rustc
/// to compile that to a wasm .rlib.
/// returns final path to wasm_deps_path + compiled rlib name
pub fn compile_extern_crate(
    output_dir: &str,
    wasm_deps_path: &str,
    target_dir: &str,
    dep_name: &str,
    force_extern_compile: bool,
) -> Result<String, String> {
    // check if existing file already made by reading the location from cached file.
    let cache_file_info = match try_find_existing_extern_crate_file(output_dir, dep_name) {
        Ok((e, o)) => {
            if force_extern_compile {
                let _ = std::fs::remove_file(&e);
                e
            } else {
                return Ok(o);
            }
        },
        Err(e) => e,
    };
    // sigh.. when testing i found that rustc isnt able to emit the file name and also compile it for some reason.
    // so this needs to be 2 steps :/
    // we first get the file name.
    let cmd_resp = Command::new("cargo")
        .args(&[
            "-q", "rustc", "--lib", "--package", dep_name, "--target", "wasm32-unknown-unknown",
            "--target-dir", target_dir,
            "--",
            "--emit=link", "--crate-type=rlib", "--print=file-names"
        ])
        .output().map_err(|e| format!("Failed to compile dependency {}\n{:?}", dep_name, e))?;
    if !cmd_resp.status.success() {
        let err_str = String::from_utf8_lossy(&cmd_resp.stderr).to_string();
        return Err(format!("Failed to compile dependency {}\n{}", dep_name, err_str));
    }
    let file_name = String::from_utf8_lossy(&cmd_resp.stdout).to_string();
    // and then we actually compile it
    let cmd_resp = Command::new("cargo")
        .args(&[
            "-q", "rustc", "--lib", "--package", dep_name, "--target", "wasm32-unknown-unknown",
            "--target-dir", target_dir,
            "--",
            "--emit=link", "--crate-type=rlib",
        ])
        .output().map_err(|e| format!("Failed to compile dependency {}\n{:?}", dep_name, e))?;
    if !cmd_resp.status.success() {
        let err_str = String::from_utf8_lossy(&cmd_resp.stderr).to_string();
        return Err(format!("Failed to compile dependency {}\n{}", dep_name, err_str));
    }

    // TODO: we are caching the location infinitely.
    // we have no way to recover if the dependency has
    // changed since the last time we built it...
    let location = format!("{}/{}", wasm_deps_path, file_name.trim());
    // best effort
    let _ = std::fs::write(cache_file_info, &location);
    Ok(location)
}

/// using `cargo metadata` we can get the output target directory
pub fn get_target_dir() -> Result<String, String> {
    // cargo -q metadata --format-version=1
    let cmd_resp = Command::new("cargo")
        .args(&["-q", "metadata", "--format-version=1"])
        .output().map_err(|e| format!("Failed to get cargo metadata\n{:?}", e))?;
    if !cmd_resp.status.success() {
        let err_str = String::from_utf8_lossy(&cmd_resp.stderr).to_string();
        return Err(format!("Failed to get cargo metadata\n{}", err_str));
    }
    let path = String::from_utf8_lossy(&cmd_resp.stdout).to_string();
    Ok(path.trim().to_string())
}

pub fn print_debug<S: AsRef<str>>(out_f: &str, contents: S) {
    let mut out_f = if let Ok(f) = std::fs::File::options().create(true).append(true).open(out_f) {
        f
    } else {
        return
    };
    // best effort
    let _ = out_f.write_all(contents.as_ref().as_bytes());
}

/// Same as `compile_strings_to_wasm`, but optionally specify a list
/// of names of crates that you want to be compiled as dependencies
/// of your wasm code.
pub fn compile_strings_to_wasm_with_extern_crates(
    data: &[(String, String)],
    extern_crate_names: &[String],
    output_dir: &str,
    custom_codegen_options: Option<Vec<&str>>,
    logfile: Option<&str>,
    force_extern_compile: bool,
) -> Result<String, String> {
    let mut delete_prefixes = HashSet::new();
    let mut delete_exclusions = vec![];
    let len = data.len();
    if len == 0 {
        return Err("Must provide at least 1 file to compile".to_string());
    }
    let last_index = len - 1;
    let mut return_string = "".to_string();

    let mut dependency_has_changed = false;

    // if this is being compiled by rust analyzer, its for a keystroke, and
    // not something we usually want to fully compile. When the user saves the file, this
    // env var is (hopefully!) not present, and then we will run a normal compile.
    // otherwise, if we detect this, AND we have a last.wasm file, then just return that
    if std::env::var("RUST_ANALYZER_INTERNALS_DO_NOT_USE").is_ok() {
        // try to see if we have the final .wasm file, if so, just return that without
        // compiling. if not, then continue and compile.
        if let Some((wasm_name, _)) = data.last() {
            let output_path = format!("{}/{}.wasm", output_dir, wasm_name);
            if std::fs::File::open(&output_path).is_ok() {
                return Ok(output_path);
            }
        }
    }

    // we add extra link args to each rustc command for each string we compile.
    // the link args allow us to include depndencies that the wasm code wants to depend on.
    let mut extra_link_args: Vec<String> = vec![];
    if !extern_crate_names.is_empty() {
        let target_dir = format!("{}/target", output_dir);
        let wasm_deps_dir = format!("{}/wasm32-unknown-unknown/debug/deps", target_dir);
        // we need this as well in case any dependency uses a proc macro. even though we compile for wasm,
        // proc macros are compiled as shared objects into the normal debug/deps directory.
        let deps_dir = format!("{}/debug/deps", target_dir);
        extra_link_args.push("-L".to_string());
        extra_link_args.push(wasm_deps_dir.clone());
        extra_link_args.push("-L".to_string());
        extra_link_args.push(deps_dir);
        for extern_crate in extern_crate_names {
            let now = std::time::Instant::now();
            let compiled_file = compile_extern_crate(output_dir, &wasm_deps_dir, &target_dir, &extern_crate, force_extern_compile)?;
            let elapsed = now.elapsed().as_millis();
            if let Some(logf) = logfile {
                print_debug(logf, format!("Compiled extern crate {} -> {} dur={}ms\n", extern_crate, compiled_file, elapsed));
            }
            extra_link_args.push("--extern".to_string());
            // this is a hack. for crate names that have dashes in them,
            // to compile them with cargo, you must provide the name with the dash.
            // but when compiling with rustc, and passing externs, it expects it
            // to be converted to underscore. our hack is to try to see if the file name
            // doesnt contain the crate name (ie: - != _)
            // and if so, then replace it with underscroe
            if !compiled_file.contains(extern_crate) {
                extra_link_args.push(format!("{}={}", extern_crate.replace("-", "_"), compiled_file));
            } else {
                extra_link_args.push(format!("{}={}", extern_crate, compiled_file));
            }
        }
    }

    for (i, (name, contents)) in data.iter().enumerate() {
        // let contents = format_file_contents(&contents)?;
        let reader = std::io::BufReader::new(contents.as_bytes());
        let hash = adler32::adler32(reader).unwrap_or(0);
        let (out_prefix, crate_type, ext) = if i == last_index {
            ("", "--crate-type=cdylib", "wasm")
        } else {
            ("lib", "--crate-type=rlib", "rlib")
        };

        let output_name = format!("{out_prefix}{name}.{ext}");
        let hash_file = format!("{output_name}.{hash}.txt");
        let output_path = format!("{}/{}", output_dir, output_name);
        let hash_file_path = format!("{}/{}", output_dir, hash_file);
        // let incremental_dir = format!("incremental={}/incremental", output_dir);
        // check if this file already exists. if so: skip its compilation.
        // if it doesnt exist: we do 2 things:
        // 1. compile it
        // 2. after compilation, look for all files of {out_prefix}{name}.*{hash}.txt
        //    and delete them (except the current one)
        if !dependency_has_changed {
            match std::fs::File::open(&hash_file_path) {
                // it exists, skip compilation. if its a wasm, we should return the path.
                Ok(_) => {
                    if ext == "wasm" { return Ok(output_path) }
                    continue;
                }
                Err(_) => {
                    // if any of the files changed, then all subsequent files depend on them
                    // so it makes sense to re-compile those.
                    dependency_has_changed = true;
                }
            }
        }
        if dependency_has_changed {
            delete_prefixes.insert(format!("{out_prefix}{name}"));
            delete_exclusions.push(output_path.clone());
            delete_exclusions.push(hash_file_path.clone());
        }

        let mut args = vec![
            &crate_type,
            "--target", "wasm32-unknown-unknown",
        ];
        if let Some(codegen_opts) = &custom_codegen_options {
            args.extend(codegen_opts);
        } else {
            // use defaults. good for cargo since its fast, although bloated.
            args.extend([
                "-C", "debuginfo=0",
                "-C", "debug-assertions=off",
                "-C", "codegen-units=16",
                "-C", "embed-bitcode=no",
                "-C", "strip=symbols",
                "-C", "lto=no",
            ]);
        }
        args.extend(["--crate-name", name, "-L", "./"]);

        for extra in &extra_link_args {
            args.push(extra);
        }
        // these should always be at the end:
        args.push(&"-o");
        args.push(&output_name);
        args.push(&"-");

        compile_single_file(&args, output_dir, &contents)?;
        // save a hash file so we can avoid compilation the next time:
        let _ = std::fs::write(hash_file_path, "");
        return_string = output_path;
    }

    // try to delete all past compiled files:
    delete_old_artifacts(output_dir, delete_prefixes, delete_exclusions);

    Ok(return_string)
}

/// new and improved compilation function.
/// Provide a Vec<(S1, S2)> where the tuple has 2 strings:
/// S1 is the name of the module being compiled, and S2 is the contents to compile.
/// This function will compile in order of the vector, and the final element of the vector will be
/// compiled to .wasm, whereas everything prior will be compiled to .rlib.
/// This function skips compiling of each unit if that unit is already compiled.
/// After compilation, this function will remove all other files that are .wasm and
/// contain the name of the final module. For example consider:
/// input: [("apple", "contents..."), ("orange", "contents...")]
/// we compile libapple.{hash}.rlib
/// and then orange.{hash}.wasm. If we detect that orange.{hash}.wasm doesnt exist,
/// then we will iterate over the output_dir and remove anything with orange.*.wasm besides the one we
/// just compiled.
/// If successful, returns full path to the final .wasm file.
pub fn compile_strings_to_wasm(
    data: &[(String, String)],
    output_dir: &str,
) -> Result<String, String> {
    compile_strings_to_wasm_with_extern_crates(data, &[], output_dir, None, None, false)
}


pub fn delete_old_artifacts(
    output_dir: &str,
    delete_prefixes: HashSet<String>,
    delete_exclusions: Vec<String>
) {
    if delete_exclusions.is_empty() {
        return;
    }

    let readdir = match std::fs::read_dir(output_dir) {
        Ok(r) => r,
        Err(_) => return,
    };

    let mut filenames = vec![];
    for entry in readdir {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let ftype = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        let base_name = entry.file_name().to_string_lossy().to_string();
        if ftype.is_file() {
            filenames.push((base_name, entry.path().to_string_lossy().to_string()));
        }
    }

    // now we have a list of files in this directory
    // iterate over the delete prefixes and delete all files that match the prefix, and
    // are not excluded by delete exclusions
    for prefix in delete_prefixes {
        for (base, path) in filenames.iter() {
            if base.starts_with(&prefix) {
                // delete it!
                // but first.. check if its a file that we want to exclude from deletion:
                // println!("I want to delete {base} because it starts with {prefix}");
                if delete_exclusions.contains(path) {
                    // println!("Delete exclusions contains {path} so i wont delete");
                    continue;
                }
                // println!("Delete exclusions DOES NOT CONTAIN {path} so i WILL delete");
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

pub fn compile_single_file(
    args: &[&str],
    output_dir: &str,
    file_data: &str,
) -> Result<(), String> {
    // // for debugging:
    // let mut out_str = "rustc ".to_string();
    // for a in args {
    //     out_str.push_str(&a);
    //     out_str.push(' ');
    // }
    // println!("=> {out_str}");

    let mut cmd = Command::new("rustc")
        .current_dir(output_dir)
        .args(args)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn().map_err(|e| format!("Failed to invoke rustc {:?}", e))?;
    if let Some(mut stdin) = cmd.stdin.take() {
        stdin.write_all(file_data.as_bytes()).map_err(|e| format!("Failed to write stdin for rustc invocation\n{:?}", e))?;
    } else {
        return Err(format!("Failed to get stdin handle when running {:?}", args));
    }

    let output = cmd.wait().map_err(|e| format!("Failed to compile {:?}\n{:?}", args, e))?;
    if !output.success() {
        let mut err = String::new();
        if let Some(mut out) = cmd.stderr.take() {
            out.read_to_string(&mut err).map_err(|e| format!("Failed to get stderr after failure to compile {:?}\n{:?}", args, e))?;
        }
        // println!("\n\nErr {err}");
        return Err(format!("Failed to compile wasm module\n{}", err));
    }
    Ok(())
}

/// If output_dir is provided we output wasm binaries to:
/// CARGO_MANIFEST_DIR/output_dir
/// If output_dir starts with a slash, then we just output directly to:
/// output_dir
pub fn compile_string_to_wasm(
    wasm_out_name: &str,
    file_data: &str,
    add_to_code: Option<String>,
    output_dir: Option<String>,
) -> Result<String, String> {
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

    let wasm_out_dir = match output_dir {
        Some(s) => if s.starts_with('/') {
            s
        } else {
            let wasm_output_base = std::env::var("CARGO_MANIFEST_DIR").unwrap_or(".".into());
            format!("{wasm_output_base}/{s}")
        }
        None => {
            let wasm_output_base = std::env::var("CARGO_MANIFEST_DIR").unwrap_or(".".into());
            format!("{}/wasmout", wasm_output_base)
        }
    };

    let wasm_out_dir_incremental = format!("{}/incremental", wasm_out_dir);
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

    let incremental_arg = format!("incremental={wasm_out_dir_incremental}");
    let _ = std::fs::create_dir(wasm_out_dir);
    let _ = std::fs::create_dir(wasm_out_dir_incremental);
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
        .arg("-C").arg("opt-level=0")
        .arg("-C").arg("debug-assertions=off")
        .arg("-C").arg("codegen-units=16")
        .arg("-C").arg("embed-bitcode=no")
        .arg("-C").arg("strip=symbols")
        .arg("-C").arg("lto=no")
        .arg("-C").arg(&incremental_arg)
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

    #[test]
    fn child_struct_def_only_once() {
        #[derive(WasmTypeGen, PartialEq, Debug, Copy, Clone)]
        pub struct Something {
            pub a: u32,
        }
        #[derive(WasmTypeGen, PartialEq, Debug)]
        pub struct Abc {
            pub a1: Something,
            pub a2: Something,
            pub a3: Something,
        }
        let item = Abc {
            a1: Something { a: 0 },
            a2: Something { a: 1 },
            a3: Something { a: 2 },
        };
        // does ser work?
        let data = item.to_binary_slice();
        assert!(data.len() > 0);
        // now deser:
        let item2 = Abc::from_binary_slice(data).expect("Expected deser to work");
        assert_eq!(item2, item2);
        // ensure that generated code for wasm includes type def of Something
        // and ensure it only appears once!
        assert_eq!(Abc::include_in_rs_wasm().match_indices("pub struct Something").collect::<Vec<_>>().len(), 1);
    }

    #[test]
    fn works_for_results() {
        #[derive(WasmTypeGen, PartialEq, Debug)]
        pub struct Something {
            pub a: u32,
        }
        #[derive(WasmTypeGen, PartialEq, Debug)]
        pub struct Other {
            pub a: u32,
        }
        #[derive(WasmTypeGen, PartialEq, Debug)]
        pub struct Abc {
            b: Result<Something, Other>,
            c: Result<Other, Something>,
        }
        let item = Abc {
            b: Ok(Something { a: 100 }),
            c: Ok(Other { a: 101 }),
        };
        // does ser work?
        let data = item.to_binary_slice();
        assert!(data.len() > 0);
        // now deser:
        let item2 = Abc::from_binary_slice(data).expect("Expected deser to work");
        assert_eq!(item2, item2);
        // ensure that generated code for wasm includes type def of Something, and Other
        // and ensure they only appear once!
        assert_eq!(Abc::include_in_rs_wasm().match_indices("pub struct Other").collect::<Vec<_>>().len(), 1);
        assert_eq!(Abc::include_in_rs_wasm().match_indices("pub struct Something").collect::<Vec<_>>().len(), 1);
    }

    #[test]
    fn works_for_hashmap() {
        #[derive(WasmTypeGen, PartialEq, Debug)]
        pub struct Something {
            pub a: u32,
        }
        #[derive(WasmTypeGen, PartialEq, Debug)]
        pub struct Abc {
            pub a: std::collections::HashMap<String, Result<Something, Something>>,
            pub b: Something,
        }
        let mut map = std::collections::HashMap::new();
        map.insert("hello".to_string(), Ok(Something { a: 1 }));
        map.insert("world".to_string(), Err(Something { a: 2 }));
        let item = Abc {
            a: map,
            b: Something { a: 3 },
        };
        // does ser work?
        let data = item.to_binary_slice();
        assert!(data.len() > 0);
        // now deser:
        let item2 = Abc::from_binary_slice(data).expect("Expected deser to work");
        assert_eq!(item2, item2);
        // ensure that generated code for wasm includes type def of Something
        // and ensure it only appears once!
        assert_eq!(Abc::include_in_rs_wasm().match_indices("pub struct Something").collect::<Vec<_>>().len(), 1);
    }
}
