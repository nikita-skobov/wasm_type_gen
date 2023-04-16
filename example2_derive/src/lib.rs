use std::path::PathBuf;
use std::str::FromStr;

use proc_macro::TokenStream;
use proc_macro2::Ident;
use syn::{
    Type,
    parse_file,
    ItemFn,
    ItemStruct,
    ItemStatic, ItemConst, ItemMod,
    // Data,
    // Fields,
    // FieldsNamed,
    // DataEnum,
    // FieldsUnnamed,
    // ExprClosure,
    // PathSegment,
    // PatType,
    // PatIdent,
    // Pat,
    // TypeParam,
    // TypeNever
};
use quote::{quote, format_ident, ToTokens};
use wasm_type_gen::*;

fn get_wasm_base_dir() -> String {
    let base_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or(".".into());
    let base_dir = format!("{base_dir}/wasm_modules");
    base_dir
}

#[proc_macro]
pub fn wasm_modules(items: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let mut module_paths = vec![];
    for item in items {
        if let proc_macro::TokenTree::Literal(l) = item {
            let mut path = l.to_string();
            while path.starts_with('"') && path.ends_with('"') {
                path.remove(0);
                path.pop();
            }
            module_paths.push(path);
        }
    }

    let base_dir = get_wasm_base_dir();
    let mut exports = vec![];
    // load every wasm module and export its types into the file the user is editing
    for mut path in module_paths {
        let original_path = path.clone();
        if !path.ends_with(".rs") {
            path.push_str(".rs");
        }
        let path_name = PathBuf::from(&path);
        let path_name = match path_name.file_stem() {
            Some(f) => f.to_string_lossy().to_string(),
            None => panic!("Unable to create module name for '{}'", path),
        };
        let module_name = format_ident!("{}", path_name);
        let path = format!("{base_dir}/{path}");
        let wasm_code = match load_rs_wasm_module(&path) {
            Ok(c) => c,
            Err(_) => {
                let s = format!("Failed to read wasm module '{}'. No file found '{}'", original_path, path);
                let out = quote! {
                    compile_error!(#s);
                };
                return TokenStream::from(out);
            }
        };
        let parsed_wasm_code = match parse_file(&wasm_code) {
            Ok(p) => p,
            Err(e) => {
                panic!("Failed to parse {} as valid rust code. Error:\n{:?}", path, e);
            }
        };
        let exported_type = parsed_wasm_code.items.iter().find_map(|item| match item {
            syn::Item::Type(ty) => if ty.ident.to_string() == "ExportType" {
                match *ty.ty {
                    Type::Path(ref ty) => {
                        match ty.path.segments.last() {
                            Some(seg) => {
                                if ty.path.segments.len() == 1 {
                                    Some(seg.ident.clone())
                                } else {
                                    None
                                }
                            }
                            None => None,
                        }
                    },
                    _ => None,
                }
            } else {
                None
            },
            _ => None,
        });
        let export_type = if let Some(export_type) = exported_type {
            export_type.to_string()
        } else {
            continue;
        };
        // search the file again and export its type inline:
        let export_item = parsed_wasm_code.items.iter().find(|thing| {
            match thing {
                syn::Item::Struct(s) => if s.ident.to_string() == export_type {
                    true
                } else {
                    false
                },
                _ => false,
            }
        });
        if let Some(export) = export_item {
            exports.push(quote! {
                mod #module_name {
                    #export
                }
            });
        }
    }

    let expanded = quote! {
        #(#exports)*
    };

    TokenStream::from(expanded)
}

/// given a module path (a string). open the file and read it to a string.
/// this string will be compiled to a wasm file.
fn load_rs_wasm_module(module_path: &str) -> Result<String, String> {
    Ok(std::fs::read_to_string(module_path)
        .map_err(|e| format!("Failed to read module path '{module_path}'\n{:?}", e))?)
}

enum GlobalVariable {
    Constant(ItemConst),
    Static(ItemStatic),
}

enum InputType {
    Struct(ItemStruct),
    Function(ItemFn),
    GlobalVar(GlobalVariable),
    Module(ItemMod),
}

impl InputType {
    pub fn get_name(&self) -> String {
        match self {
            InputType::Struct(di) => di.ident.to_string(),
            InputType::Function(fi) => fi.sig.ident.to_string(),
            InputType::Module(mi) => mi.ident.to_string(),
            InputType::GlobalVar(gi) => match gi {
                GlobalVariable::Constant(c) => c.ident.to_string(),
                GlobalVariable::Static(c) => c.ident.to_string(),
            }
        }
    }
    pub fn back_to_stream(self) -> proc_macro2::TokenStream {
        match self {
            InputType::Struct(s) => s.into_token_stream(),
            InputType::Function(f) => f.into_token_stream(),
            InputType::GlobalVar(g) => match g {
                GlobalVariable::Constant(c) => c.into_token_stream(),
                GlobalVariable::Static(s) => s.into_token_stream(),
            }
            InputType::Module(m) => m.into_token_stream(),
        }
    }
}

fn get_input_type(item: proc_macro2::TokenStream) -> Option<InputType> {
    let is_struct_input = syn::parse2::<ItemStruct>(item.clone()).ok();
    if let Some(struct_input) = is_struct_input {
        return Some(InputType::Struct(struct_input));
    }
    let is_fn_input = syn::parse2::<ItemFn>(item.clone()).ok();
    if let Some(function_input) = is_fn_input {
        return Some(InputType::Function(function_input));
    }
    
    let is_static_input = syn::parse2::<ItemStatic>(item.clone()).ok();
    if let Some(input) = is_static_input {
        return Some(InputType::GlobalVar(GlobalVariable::Static(input)));
    }
    let is_const_input = syn::parse2::<ItemConst>(item.clone()).ok();
    if let Some(input) = is_const_input {
        return Some(InputType::GlobalVar(GlobalVariable::Constant(input)));
    }
    let is_mod_input = syn::parse2::<ItemMod>(item.clone()).ok();
    if let Some(input) = is_mod_input {
        return Some(InputType::Module(input));
    }
    None
}

fn rename_ident(id: &mut Ident, name: &str) {
    if id.to_string() != name {
        let span = id.span();
        let new_ident = Ident::new(name, span);
        *id = new_ident;
    }
}

#[proc_macro_attribute]
pub fn wasm_meta(attr: proc_macro::TokenStream, item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    // this is the data the end user passed to the macro, and we serialize it
    // and pass it to the wasm module that the user specified
    #[derive(WasmTypeGen, Debug)]
    pub enum UserData {
        Struct { name: String, },
        Function { name: String, },
        Module { name: String, },
        GlobalVariable { name: String, },
        Missing,
    }
    impl Default for UserData {
        fn default() -> Self {
            Self::Missing
        }
    }

    #[derive(WasmTypeGen, Debug, Default)]
    pub struct LibraryObj {
        pub compiler_error_message: String,
        pub user_data: UserData,
    }

    impl From<&InputType> for UserData {
        fn from(value: &InputType) -> Self {
            let name = value.get_name();
            match value {
                InputType::Struct(x) => {
                    Self::Struct { name }
                },
                InputType::Function(x) => {
                    Self::Function { name }
                }
                InputType::GlobalVar(x) => {
                    Self::GlobalVariable { name }
                }
                InputType::Module(x) => {
                    Self::Module { name }
                }
            }
        }
    }

    impl InputType {
        pub fn apply_library_obj_changes(&mut self, lib_obj: LibraryObj) {
            let user_data = lib_obj.user_data;
            match (self, user_data) {
                (InputType::Struct(x), UserData::Struct { name }) => {
                    rename_ident(&mut x.ident, &name);
                }
                (InputType::Function(x), UserData::Function { name }) => {
                    rename_ident(&mut x.sig.ident, &name);
                }
                (InputType::GlobalVar(GlobalVariable::Constant(x)), UserData::GlobalVariable { name }) => {
                    rename_ident(&mut x.ident, &name);
                }
                (InputType::GlobalVar(GlobalVariable::Static(x)), UserData::GlobalVariable { name }) => {
                    rename_ident(&mut x.ident, &name);
                }
                (InputType::Module(x), UserData::Module { name }) => {
                    rename_ident(&mut x.ident, &name);
                }
                _ => {}
            }
        }
    }

    #[output_and_stringify_basic(library_obj_extra_impl)]
    impl LibraryObj {
        fn compile_error(&mut self, err_msg: &str) {
            self.compiler_error_message = err_msg.into();
        }
    }

    // this is a hack to allow people who write wasm_modules easy type hints.
    // if we detect no attributes, then we just output all of the types that
    // wasm module writers depend on, like UserData, and LibraryObj
    if attr.is_empty() {
        let mut include_str = LibraryObj::include_in_rs_wasm();
        include_str.push_str(library_obj_extra_impl);
        let include_tokens = proc_macro2::TokenStream::from_str(&include_str).unwrap_or_default();
        let parsing_tokens = proc_macro2::TokenStream::from_str(WASM_PARSING_TRAIT_STR).unwrap_or_default();
        let out = quote! {
            #parsing_tokens

            #include_tokens
        };
        return TokenStream::from(out);
    }
    
    let mut attr = proc_macro2::TokenStream::from(attr);
    let item = proc_macro2::TokenStream::from(item);
    let item_str = item.to_string();
    let attr_str = attr.to_string();
    let combined = format!("{item_str}{attr_str}");
    let hash = adler32::adler32(combined.as_bytes()).unwrap_or(0);
    let func_name = format_ident!("_a{hash}");
    let err_str = "Failed to parse signature of macro attribute. Expected a closure like |obj: &mut modulename::StructName| {{ ... }}";
    let input_type = get_input_type(item);

    // verify the input is something that we support. currently:
    // - entire functions, signature + body.
    // - derive input, ie: struct defs, enums.
    let mut input_type = if let Some(input) = input_type {
        input
    } else {
        panic!("wasm_meta was applied to an item that we currently do not support parsing. Currently only supports functions and deriveInputs");
    };

    // get everything in callback input signature |mything: &mut modulename::StructName| { ... }
    let splits: Vec<_> = attr_str.split("|").collect();
    let signature = match splits.get(1) {
        Some(s) => *s,
        None => panic!("{}", err_str),
    };
    // now signature looks like
    // mything: &mut modulename::StructName
    // actually it has spaces around it, but we can solve that by just removing the spaces
    let signature_nospace = signature.replace(" ", "");
    let after_mut = if let Some((_, b)) = signature_nospace.split_once("&mut") {
        b.trim()
    } else {
        panic!("{}", err_str);
    };
    let module_name = if let Some((mod_name, _)) = after_mut.split_once("::") {
        mod_name
    } else {
        panic!("{}", err_str);
    };
    let module_name_ident = format_ident!("{module_name}");
    let base_dir = get_wasm_base_dir();
    let module_path = format!("{base_dir}/{module_name}.rs");
    let wasm_module_source = match load_rs_wasm_module(&module_path) {
        Ok(c) => c,
        Err(e) => {
            panic!("Error while reading file '{}' for module {module_name}. {:?}", module_path, e);
        }
    };
    let parsed_wasm_code = match parse_file(&wasm_module_source) {
        Ok(p) => p,
        Err(e) => {
            panic!("Failed to parse {} as valid rust code. Error:\n{:?}", module_path, e);
        }
    };
    let exported_type = parsed_wasm_code.items.iter().find_map(|item| match item {
        syn::Item::Type(ty) => if ty.ident.to_string() == "ExportType" {
            match *ty.ty {
                Type::Path(ref ty) => {
                    match ty.path.segments.last() {
                        Some(seg) => {
                            if ty.path.segments.len() == 1 {
                                Some(seg.ident.clone())
                            } else {
                                None
                            }
                        }
                        None => None,
                    }
                },
                _ => None,
            }
        } else {
            None
        },
        _ => None,
    });
    let entrypoint_fn = parsed_wasm_code.items.iter().find(|item| {
        match item {
            syn::Item::Fn(fn_item) => {
                if fn_item.sig.ident.to_string() != "wasm_entrypoint" {
                    return false
                }
                if let syn::ReturnType::Default = fn_item.sig.output {} else {
                    return false
                }
                // enforce 2 args: the first is the LibraryObj
                // the 2nd is the callback to the user's function.
                // but too lazy to parse the callback signature right now. we just assume its valid..
                let input = if fn_item.sig.inputs.len() != 2 {
                    return false
                } else {
                    fn_item.sig.inputs.first().unwrap()
                };
                let input = match input {
                    syn::FnArg::Typed(t) => t,
                    _ => return false,
                };
                let reference = match *input.ty {
                    Type::Reference(ref r) => r.clone(),
                    _ => return false,
                };
                if reference.mutability.is_none() {
                    return false
                }
                let type_path = match *reference.elem {
                    Type::Path(p) => p,
                    _ => return false,
                };
                let first = match type_path.path.segments.first() {
                    Some(s) => s,
                    None => return false,
                };
                if first.ident.to_string() != "LibraryObj" {
                    return false
                }
                true
            }
            _ => false,
        }
    });
    if entrypoint_fn.is_none() {
        panic!("Module '{}' is missing an entrypoint function. Valid modules must contain an entrypoint within the following signature:\npub fn wasm_entrypoint(obj: &mut LibraryObj);", module_path);
    }
    let exported_name = match exported_type {
        Some(n) => n,
        None => panic!("Module '{}' is missing a valid ExportType. Expected to find statement like `pub type ExportType = SomeStruct;`", module_path)
    };

    // this is necessary to allow the compile function to find previously compiled versions in case it fails to compile.
    // it groups it by this "item_hash".
    let item_name = input_type.get_name();

    let mut pass_this = LibraryObj::default();
    pass_this.user_data = (&input_type).into();
    let mut add_to_code = LibraryObj::include_in_rs_wasm();
    add_to_code.push_str(LibraryObj::gen_entrypoint());
    add_to_code.push_str(WASM_PARSING_TRAIT_STR);
    add_to_code.push_str(library_obj_extra_impl);

    let final_wasm_source = quote! {
        pub fn wasm_main(library_obj: &mut LibraryObj) {
            #module_name_ident::wasm_entrypoint(library_obj, users_fn);
        }
        mod #module_name_ident {
            use super::LibraryObj;
            use super::UserData;
            #parsed_wasm_code
        }
        pub fn users_fn(data: &mut #module_name_ident::#exported_name) {
            let cb = #attr;
            cb(data);
        }
    };

    fn get_wasm_output(
        out_name_hash: &str,
        wasm_source: &str,
        add_to_source: Option<String>,
        data_to_pass: &LibraryObj,
    ) -> Option<LibraryObj> {
        let out_file = compile_string_to_wasm(out_name_hash, wasm_source, add_to_source).expect("compilation error");
        let wasm_file = std::fs::read(out_file).expect("failed to read wasm binary");
        let out = run_wasm(&wasm_file, data_to_pass.to_binary_slice()).expect("runtime error running wasm");
        LibraryObj::from_binary_slice(out)
    }

    // TODO: instead of hashing the whole item input, use the item name, for eg function name or struct name.
    // this way it wont change as often
    // let item_hash = adler32::adler32(item_str.as_bytes()).unwrap_or(0);
    let lib_obj = get_wasm_output(
        &item_name,
        &final_wasm_source.to_string(),
        Some(add_to_code), 
        &pass_this
    ).unwrap_or_default();

    println!("GOT BACK FROM WASM:\n{:#?}", lib_obj);

    if !lib_obj.compiler_error_message.is_empty() {
        // TODO: currently we just add a compile_error to the end of the stream..
        // in the future maybe search for a string, and replace the right hand side to compile_error
        // so that we can put it on a specific line
        let err = format!("compile_error!(\"{}\");", lib_obj.compiler_error_message);
        if let Ok(err) = proc_macro2::TokenStream::from_str(&err) {
            attr.extend([err]);
        }
    }

    input_type.apply_library_obj_changes(lib_obj);
    let item = input_type.back_to_stream();
    let user_out = quote! {
        // we use a random hash for the func name to not conflict with other invocations of this macro
        fn #func_name() {
            let cb = #attr;
        }
        #item
    };

    TokenStream::from(user_out)
}
