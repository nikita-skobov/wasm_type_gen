use std::{path::PathBuf, io::Write};
use std::str::FromStr;
use toml::Table;

use proc_macro::TokenStream;
use proc_macro2::Ident;
use syn::{
    Type,
    parse_file,
    ItemFn,
    ItemStruct,
    ItemStatic,
    ItemConst,
    ItemMod,
    Visibility,
    token::Pub,
    ExprMatch,
};
use quote::{quote, format_ident, ToTokens};
use wasm_type_gen::*;

// TODO: need to use locking? proc-macros run single threaded so i think this is safe?
static mut SHARED_FILE_DATA: Vec<MapEntry<MapEntry<String>>> = vec![];

struct MapEntry<T> {
    pub key: String,
    pub lines: Vec<T>,
}

fn get_wasm_base_dir() -> String {
    let base_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or(".".into());
    let base_dir = format!("{base_dir}/wasm_modules");
    base_dir
}

fn get_wasmgen_base_dir() -> String {
    let base_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or(".".into());
    format!("{base_dir}/wasmgen")
}

fn should_do_file_operations() -> bool {
    // through manual testing i've found that running cargo build uses RUST_BACKTRACE full
    // whereas the cargo command used by IDEs sets this to short. basically: dont output command
    // files every keystroke.. instead we only wish to do this when the user actually builds.
    let mut should_do = false;
    if let Ok(env) = std::env::var("RUST_BACKTRACE") {
        if env == "full" {
            should_do = true;
        }
    }
    // check for optional env vars set by users:
    if let Ok(env) = std::env::var("CARGO_WASMTYPEGEN_FILEOPS") {
        if env == "false" || env == "0" {
            should_do = false;
        } else if env == "true" || env == "1" {
            should_do = true;
        }
    }
    should_do
}

fn struct_item_to_doc_comment(item: &mut ItemStruct) -> String {
    let mut s = "# Full Definition:\n\n```\n".to_string();
    s.push_str(&item.vis.to_token_stream().to_string());
    s.push(' ');
    s.push_str(&item.struct_token.to_token_stream().to_string());
    s.push(' ');
    s.push_str(&item.ident.to_string());
    s.push_str(" {\n");
    for field in item.fields.iter() {
        for attr in field.attrs.iter() {
            if let syn::Meta::NameValue(nv) = &attr.meta {
                if let syn::Expr::Lit(l) = &nv.value {
                    let mut out_comment = l.lit.to_token_stream().to_string();
                    while out_comment.starts_with('"') && out_comment.ends_with('"') {
                        out_comment.remove(0);
                        out_comment.pop();
                    }
                    s.push_str(&format!("  ///{}\n", out_comment));
                }
            }
        }
        s.push_str(&format!("  {} {}: {},\n",
            field.vis.to_token_stream().to_string(),
            field.ident.to_token_stream().to_string(),
            field.ty.to_token_stream().to_string(),
        ));
    }
    s.push_str("}\n");
    s.push_str("```\n");

    s
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
    let mut required_crates = vec![];
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
        let mut parsed_wasm_code = match parse_file(&wasm_code) {
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
        let reqs = parsed_wasm_code.items.iter().find_map(|item| {
            if let syn::Item::Const(c) = item {
                if c.ident.to_string() == "REQUIRED_CRATES" {
                    let arr = match &*c.expr {
                        syn::Expr::Array(arr) => arr,
                        syn::Expr::Reference(r) => {
                            if let syn::Expr::Array(arr) = &*r.expr {
                                arr
                            } else {
                                return None;
                            }
                        }
                        _ => {
                            return None;
                        }
                    };
                    let mut out: Vec<String> = vec![];
                    for item in arr.elems.iter() {
                        if let syn::Expr::Lit(l) = item {
                            if let syn::Lit::Str(s) = &l.lit {
                                let mut s = s.token().to_string();
                                while s.starts_with('"') && s.ends_with('"') {
                                    s.remove(0);
                                    s.pop();
                                }
                                out.push(s);
                            }
                        }
                    }
                    return Some(out);
                }
            }
            None
        });
        if let Some(requireds) = reqs {
            required_crates.push((original_path.clone(), requireds));
        }

        // search the file again and export its type inline:
        let export_item = parsed_wasm_code.items.iter_mut().find_map(|thing| {
            match thing {
                syn::Item::Struct(s) => if s.ident.to_string() == export_type {
                    let struct_def = struct_item_to_doc_comment(s);
                    let attrs = std::mem::take(&mut s.attrs);
                    Some((s, struct_def, attrs))
                } else {
                    None
                },
                _ => None,
            }
        });
        if let Some((export, export_str, attrs)) = export_item {
            exports.push(quote! {
                mod #module_name {
                    #(#attrs)*
                    #[doc = #export_str]
                    #export
                }
            });
        }
    }

    if !required_crates.is_empty() {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or(".".into());
        let manifest_file_path = format!("{manifest_dir}/Cargo.toml");
        let cargo_file_str = match std::fs::read_to_string(manifest_file_path) {
            Ok(o) => o,
            Err(e) => panic!("One or more of your wasm modules has REQUIRED_CRATES. But failed to find Cargo.toml.\nError:\n{:?}", e),
        };
        let value = cargo_file_str.parse::<Table>().unwrap();
        let mut dependencies = vec![];
        if let Some(deps) = value.get("dependencies") {
            if let toml::Value::Table(deps) = deps {
                for (key, _) in deps {
                    dependencies.push(key);
                }
            }
        }
        for (wasm_mod_name, requireds) in required_crates {
            let mut missing = vec![];
            for req in requireds.iter() {
                if !dependencies.contains(&req) {
                    missing.push(req);
                }
            }
            if !missing.is_empty() {
                panic!("Wasm module '{}' depends on the following crates:\n{:#?}\nFailed to find:\n{:#?}\nPlease edit your Cargo.toml file to add these dependencies\n", wasm_mod_name, requireds, missing);
            }
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

#[derive(Debug)]
enum GlobalVariable {
    Constant(ItemConst),
    Static(ItemStatic),
}

#[derive(Debug)]
enum InputType {
    Struct(ItemStruct),
    Function(ItemFn),
    GlobalVar(GlobalVariable),
    Module(ItemMod),
    Match(ExprMatch),
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
            InputType::Match(mi) => mi.expr.to_token_stream().to_string(),
        }
    }
    /// use_name is only necessary for Match input types. for match statements
    /// we hide the match inside a function, otherwise most match statements arent valid
    /// in a const context, but const contexts is the only way we can conveniently read + parse them
    pub fn back_to_stream(self, use_name: &str) -> proc_macro2::TokenStream {
        match self {
            InputType::Struct(s) => s.into_token_stream(),
            InputType::Function(f) => f.into_token_stream(),
            InputType::GlobalVar(g) => match g {
                GlobalVariable::Constant(c) => c.into_token_stream(),
                GlobalVariable::Static(s) => s.into_token_stream(),
            }
            InputType::Match(m) => {
                let use_name_ident = format_ident!("{use_name}");
                quote! {
                    fn #use_name_ident() {
                        #m
                    }
                }
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
        if input.ident.to_string() == "_" {
            if let syn::Expr::Match(m) = *input.expr {
                return Some(InputType::Match(m));
            }
        }
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

fn is_public(vis: &Visibility) -> bool {
    match vis {
        Visibility::Public(_) => true,
        _ => false,
    }
}

fn set_visibility(vis: &mut Visibility, is_pub: bool) {
    let p = Pub::default();
    match (&vis, is_pub) {
        (Visibility::Public(_), false) => {
            *vis = Visibility::Inherited;
        }
        (Visibility::Restricted(_), true) => {
            *vis = Visibility::Public(p);
        }
        (Visibility::Inherited, true) => {
            *vis = Visibility::Public(p);
        }
        _ => {}
    }
}

fn merge_shared_files(
    wasm_module_name: &str,
    data: Vec<MapEntry<MapEntry<(bool, String, Option<String>)>>>
) -> Result<(), String> {
    unsafe {
        // merge the current data with the previous data
        for entry in data {
            let file_name = entry.key;
            // this is how we enforce that shared files only get output
            // to the shared directory. basically: it can only be a file name, not a path.
            if file_name.contains("/") || file_name.contains("\\") {
                return Err(format!("Wasm module '{wasm_module_name}' attempted to output a shared file outside the shared file directory {:?}", file_name));
            }
            for file_data in entry.lines {
                let label = file_data.key;

                let label_entry = if let Some(e) = SHARED_FILE_DATA.iter_mut().find(|x| x.key == file_name) {
                    if let Some(l) = e.lines.iter_mut().find(|x| x.key == label) {
                        l
                    } else {
                        let index = e.lines.len();
                        e.lines.push(MapEntry { key: label.clone(), lines: vec![] });
                        &mut e.lines[index]
                    }
                } else {
                    let index = SHARED_FILE_DATA.len();
                    SHARED_FILE_DATA.push(MapEntry { key: file_name.clone(), lines: vec![MapEntry { key: label.clone(), lines: vec![] }] });
                    &mut SHARED_FILE_DATA[index].lines[0]
                };

                for (unique, line, after) in file_data.lines {
                    if unique {
                        if !label_entry.lines.contains(&line) {
                            label_entry.lines.push(line);
                        }
                        continue;
                    }
                    // if after is provided, then treat 'line' as a search string, and
                    // try to insert the after portion immediately after the search string.
                    // if not found, output a newline concatenation of line+after
                    if let Some(after) = after {
                        let found_str = label_entry.lines.iter_mut()
                            .find_map(|l| l.find(&line).map(|index| (l, index + line.len())));
                        if let Some((found_str, index)) = found_str {
                            // found, now insert the after portion at the index
                            found_str.insert_str(index, &after);
                        } else {
                            // not found, just concatenate and output
                            label_entry.lines.push(format!("{line}{after}"));
                        }
                        continue;
                    }
                    // otherwise, its just a normal line entry
                    label_entry.lines.push(line);
                }
            }
        }
    
        Ok(())
    }
}

fn output_shared_files(
    wasm_module_name: &str,
    data: Vec<MapEntry<MapEntry<(bool, String, Option<String>)>>>
) -> Result<(), String> {
    // set the wasm_module's data into the global shared data object.
    merge_shared_files(wasm_module_name, data)?;
    // iterate the shared data object and output to the shared file(s)

    let shared_dir = get_wasmgen_base_dir();

    unsafe {
        for file_entry in SHARED_FILE_DATA.iter_mut() {
            let file_name = &file_entry.key;
            let file_path = format!("{shared_dir}/{file_name}");
            let mut out_f = std::fs::File::create(&file_path)
                .map_err(|e| format!("Failed to create/open file while running module '{wasm_module_name}' {:?}\nError:\n{:?}", file_path, e))?;

            // sort the labels alphabetically
            file_entry.lines.sort_by(|a, b| a.key.cmp(&b.key));

            for label_entry in file_entry.lines.iter() {
                let label = &label_entry.key;
                out_f.write_all(label.as_bytes()).map_err(|e| format!("Failed to write to file while running module '{wasm_module_name}' {:?}\nError:\n{:?}", file_path, e))?;
                out_f.write_all(b"\n").map_err(|e| format!("Failed to write to file while running module '{wasm_module_name}' {:?}\nError:\n{:?}", file_path, e))?;
                for line in label_entry.lines.iter() {
                    out_f.write_all(line.as_bytes()).map_err(|e| format!("Failed to write to file while running module '{wasm_module_name}' {:?}\nError:\n{:?}", file_path, e))?;
                    out_f.write_all(b"\n").map_err(|e| format!("Failed to write to file while running module '{wasm_module_name}' {:?}\nError:\n{:?}", file_path, e))?;
                }
            }
        }
    }

    Ok(())
}

#[proc_macro_attribute]
pub fn wasm_meta(attr: proc_macro::TokenStream, item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    // this is the data the end user passed to the macro, and we serialize it
    // and pass it to the wasm module that the user specified
    #[derive(WasmTypeGen, Debug)]
    pub enum UserData {
        /// fields are read only. modifying them in your wasm_module has no effect.
        Struct { name: String, is_pub: bool, fields: Vec<UserField> },
        /// inputs are read only. modifying them in your wasm_module has no effect.
        Function { name: String, is_pub: bool, is_async: bool, inputs: Vec<UserInput>, return_ty: String },
        Module { name: String, is_pub: bool, },
        GlobalVariable { name: String, is_pub: bool, },
        Match { name: String, is_pub: bool },
        Missing,
    }
    impl Default for UserData {
        fn default() -> Self {
            Self::Missing
        }
    }

    #[derive(WasmTypeGen, Debug)]
    pub struct UserField {
        /// only relevant for struct fields. not applicable to function params.
        pub is_public: bool,
        pub name: String,
        pub ty: String,
    }

    #[derive(WasmTypeGen, Debug)]
    pub struct UserInput {
        /// only relevant for input params to a function. not applicable to struct fields.
        pub is_self: bool,
        pub name: String,
        pub ty: String,
    }

    #[derive(WasmTypeGen, Debug)]
    pub struct FileOut {
        pub name: String,
        pub data: Vec<u8>,
    }

    #[derive(WasmTypeGen, Debug)]
    pub struct SharedOutputEntry {
        pub filename: String,
        pub label: String,
        pub line: String,
        pub unique: bool,
        pub after: Option<String>,
    }

    #[derive(WasmTypeGen, Debug, Default)]
    pub struct LibraryObj {
        pub compiler_error_message: String,
        pub add_code_after: Vec<String>,
        /// crate_name is read only. modifying this has no effect.
        pub crate_name: String,
        pub user_data: UserData,
        pub shared_output_data: Vec<SharedOutputEntry>,
    }

    fn to_map_entry(data: Vec<SharedOutputEntry>) -> Vec<MapEntry<MapEntry<(bool, String, Option<String>)>>> {
        let mut map_entries: Vec<MapEntry<MapEntry<(bool, String, Option<String>)>>> = vec![];
        for d in data {
            if let Some(m) = map_entries.iter_mut().find(|x| x.key == d.filename) {
                if let Some(m) = m.lines.iter_mut().find(|x| x.key == d.label) {
                    m.lines.push((d.unique, d.line, d.after));
                } else {
                    m.lines.push(MapEntry { key: d.label, lines: vec![(d.unique, d.line, d.after)] });
                }
            } else {
                map_entries.push(MapEntry { key: d.filename, lines: vec![MapEntry {
                    key: d.label,
                    lines: vec![(d.unique, d.line, d.after)],
                }] })
            }
        }
        map_entries
    }

    impl LibraryObj {
        pub fn handle_file_ops(&mut self, wasm_module_name: &str, _user_type_name: &str) -> Result<(), String> {
            output_shared_files(wasm_module_name, to_map_entry(std::mem::take(&mut self.shared_output_data)))
        }
    }

    impl From<&InputType> for UserData {
        fn from(value: &InputType) -> Self {
            let name = value.get_name();
            match value {
                InputType::Struct(x) => {
                    let mut fields = vec![];
                    for field in x.fields.iter() {
                        let usr_field = UserField {
                            is_public: is_public(&field.vis),
                            name: field.ident.as_ref().map(|i| i.to_string()).unwrap_or_default(),
                            ty: field.ty.to_token_stream().to_string(),
                        };
                        fields.push(usr_field);
                    }
                    Self::Struct { name, is_pub: is_public(&x.vis), fields }
                },
                InputType::Function(x) => {
                    let mut inputs = vec![];
                    for input in x.sig.inputs.iter() {
                        let usr_field = UserInput {
                            is_self: match input {
                                syn::FnArg::Receiver(_) => true,
                                syn::FnArg::Typed(_) => false,
                            },
                            name: match input {
                                syn::FnArg::Receiver(_) => "&self".into(),
                                syn::FnArg::Typed(ty) => ty.pat.to_token_stream().to_string(),
                            },
                            ty: match input {
                                syn::FnArg::Receiver(_) => "".into(),
                                syn::FnArg::Typed(ty) => ty.ty.to_token_stream().to_string(),
                            }
                        };
                        inputs.push(usr_field);
                    }
                    let return_ty = match &x.sig.output {
                        syn::ReturnType::Default => "".into(),
                        syn::ReturnType::Type(_, b) => b.to_token_stream().to_string(),
                    };
                    Self::Function { name, is_pub: is_public(&x.vis), inputs, is_async: x.sig.asyncness.is_some(), return_ty }
                }
                InputType::GlobalVar(GlobalVariable::Constant(x)) => {
                    Self::GlobalVariable { name, is_pub: is_public(&x.vis) }
                }
                InputType::GlobalVar(GlobalVariable::Static(x)) => {
                    Self::GlobalVariable { name, is_pub: is_public(&x.vis) }
                }
                InputType::Module(x) => {
                    Self::Module { name, is_pub: is_public(&x.vis) }
                }
                InputType::Match(_x) => {
                    // TODO: implement iterating match arms
                    Self::Match { name, is_pub: false }
                }
            }
        }
    }

    impl InputType {
        pub fn apply_library_obj_changes(&mut self, lib_obj: LibraryObj) {
            let user_data = lib_obj.user_data;
            match (self, user_data) {
                (InputType::Struct(x), UserData::Struct { name, is_pub, .. }) => {
                    rename_ident(&mut x.ident, &name);
                    set_visibility(&mut x.vis, is_pub);
                }
                (InputType::Function(x), UserData::Function { name, is_pub, .. }) => {
                    rename_ident(&mut x.sig.ident, &name);
                    set_visibility(&mut x.vis, is_pub);
                }
                (InputType::GlobalVar(GlobalVariable::Constant(x)), UserData::GlobalVariable { name, is_pub, .. }) => {
                    rename_ident(&mut x.ident, &name);
                    set_visibility(&mut x.vis, is_pub);
                }
                (InputType::GlobalVar(GlobalVariable::Static(x)), UserData::GlobalVariable { name, is_pub, .. }) => {
                    rename_ident(&mut x.ident, &name);
                    set_visibility(&mut x.vis, is_pub);
                }
                (InputType::Module(x), UserData::Module { name, is_pub, .. }) => {
                    rename_ident(&mut x.ident, &name);
                    set_visibility(&mut x.vis, is_pub);
                }
                _ => {}
            }
        }
    }

    #[output_and_stringify_basic(library_obj_extra_impl)]
    impl LibraryObj {
        #[allow(dead_code)]
        fn compile_error(&mut self, err_msg: &str) {
            self.compiler_error_message = err_msg.into();
        }
        /// given a file name (no paths. the file will appear in ./wasmgen/{filename})
        /// and a label, and a line (string) append to the file. create the file if it doesnt exist.
        /// the label is used to sort lines between your wasm module and other invocations.
        /// the label is also embedded to the file. so if you are outputing to a .sh file, for example,
        /// your label should start with '#'. The labels are sorted alphabetically.
        /// Example:
        /// ```rust,ignore
        /// # wasm module 1 does:
        /// append_to_file("hello.txt", "b", "line1");
        /// # wasm module 2 does:
        /// append_to_file("hello.txt", "b", "line2");
        /// # wasm module 3 does:
        /// append_to_file("hello.txt", "a", "line3");
        /// # wasm moudle 4 does:
        /// append_to_file("hello.txt", "a", "line4");
        /// 
        /// # the output:
        /// a
        /// line3
        /// line4
        /// b
        /// line1
        /// line2
        /// ```
        #[allow(dead_code)]
        fn append_to_file(&mut self, name: &str, label: &str, line: String) {
            self.shared_output_data.push(SharedOutputEntry { label: label.into(), line, filename: name.into(), unique: false, after: None });
        }

        /// same as append_to_file, but the line will be unique within the label
        #[allow(dead_code)]
        fn append_to_file_unique(&mut self, name: &str, label: &str, line: String) {
            self.shared_output_data.push(SharedOutputEntry { label: label.into(), line, filename: name.into(), unique: true, after: None });
        }

        /// like append_to_file, but given a search string, find that search string in that label
        /// and then append the `after` portion immediately after the search string. Example:
        /// ```rust,ignore
        /// // "hello " doesnt exist yet, so the whole "hello , and also my friend Tim!" gets added
        /// append_to_line("hello.txt", "a", "hello ", ", and also my friend Tim!");
        /// append_to_line("hello.txt", "a", "hello ", "world"); 
        /// 
        /// # the output:
        /// hello world, and also my friend Tim!
        /// ```
        #[allow(dead_code)]
        fn append_to_line(&mut self, name: &str, label: &str, search_str: String, after: String) {
            self.shared_output_data.push(SharedOutputEntry { label: label.into(), line: search_str, filename: name.into(), unique: false, after: Some(after) });
        }
    }
    #[output_and_stringify_basic(user_data_extra_impl)]
    impl UserData {
        /// Get the name of the user's data that they put this macro over.
        /// for example `struct MyStruct { ... }` returns "MyStruct"
        /// 
        /// or `pub fn helloworld(a: u32) { ... }` returns "helloworld"
        /// Can rename the user's data type by modifying this string directly
        #[allow(dead_code)]
        fn get_name(&mut self) -> &mut String {
            match self {
                UserData::Struct { name, .. } => name,
                UserData::Function { name, .. } => name,
                UserData::Module { name, .. } => name,
                UserData::GlobalVariable { name, .. } => name,
                UserData::Match { name, .. } => name,
                UserData::Missing => unreachable!(),
            }
        }
        /// Returns a bool of whether or not the user marked their data as pub or not.
        /// Can set this value to true or false depending on your module's purpose.
        #[allow(dead_code)]
        fn get_public_vis(&mut self) -> &mut bool {
            match self {
                UserData::Struct { is_pub, .. } => is_pub,
                UserData::Function { is_pub, .. } => is_pub,
                UserData::Module { is_pub, .. } => is_pub,
                UserData::GlobalVariable { is_pub, .. } => is_pub,
                UserData::Match { is_pub, .. } => is_pub,
                UserData::Missing => unreachable!(),
            }
        }
    }

    // this is a hack to allow people who write wasm_modules easy type hints.
    // if we detect no attributes, then we just output all of the types that
    // wasm module writers depend on, like UserData, and LibraryObj
    if attr.is_empty() {
        let mut include_str = LibraryObj::include_in_rs_wasm();
        include_str.push_str(library_obj_extra_impl);
        include_str.push_str(user_data_extra_impl);
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
    // println!("{:#?}", input_type);

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

    let should_output_command_files = should_do_file_operations();
    // this is necessary to allow the compile function to find previously compiled versions in case it fails to compile.
    // it groups it by this "item_hash".
    let item_name = input_type.get_name();

    let mut pass_this = LibraryObj::default();
    pass_this.user_data = (&input_type).into();
    pass_this.crate_name = std::env::var("CARGO_CRATE_NAME").unwrap_or("".into());
    let mut add_to_code = LibraryObj::include_in_rs_wasm();
    add_to_code.push_str(LibraryObj::gen_entrypoint());
    add_to_code.push_str(WASM_PARSING_TRAIT_STR);
    add_to_code.push_str(library_obj_extra_impl);
    add_to_code.push_str(user_data_extra_impl);

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
        let out_file = compile_string_to_wasm(out_name_hash, wasm_source, add_to_source, None).expect("compilation error");
        let wasm_file = std::fs::read(out_file).expect("failed to read wasm binary");
        let out = run_wasm(&wasm_file, data_to_pass.to_binary_slice()).expect("runtime error running wasm");
        LibraryObj::from_binary_slice(out)
    }

    // TODO: instead of hashing the whole item input, use the item name, for eg function name or struct name.
    // this way it wont change as often
    // let item_hash = adler32::adler32(item_str.as_bytes()).unwrap_or(0);
    let mut lib_obj = get_wasm_output(
        &item_name,
        &final_wasm_source.to_string(),
        Some(add_to_code), 
        &pass_this
    ).unwrap_or_default();
    // println!("GOT BACK FROM WASM:\n{:#?}", lib_obj);

    if !lib_obj.compiler_error_message.is_empty() {
        // TODO: currently we just add a compile_error to the end of the stream..
        // in the future maybe search for a string, and replace the right hand side to compile_error
        // so that we can put it on a specific line
        let err = format!("compile_error!(r#\"{}\"#);", lib_obj.compiler_error_message);
        if let Ok(err) = proc_macro2::TokenStream::from_str(&err) {
            attr.extend([err]);
        }
    }

    let mut add_after = vec![];
    for s in lib_obj.add_code_after.drain(..) {
        let tokens = match proc_macro2::TokenStream::from_str(&s) {
            Ok(o) => o,
            Err(e) => {
                panic!("Module '{}' produced invalid after_code tokens:\n{}\nError:\n{:?}", module_name, s, e);
            }
        };
        add_after.push(tokens);
    }

    if should_output_command_files {
        if let Err(e) = lib_obj.handle_file_ops(module_name, &item_name) {
            panic!("{}", e);
        }
    }

    input_type.apply_library_obj_changes(lib_obj);
    let item = input_type.back_to_stream(&format!("_b{hash}"));
    let user_out = quote! {
        // we use a random hash for the func name to not conflict with other invocations of this macro
        fn #func_name() {
            let cb = #attr;
        }
        #item

        #(#add_after)*
    };

    TokenStream::from(user_out)
}
