use proc_macro::TokenStream;
use syn::{parse_macro_input, DeriveInput, Data, Fields, Type, FieldsNamed, DataEnum, FieldsUnnamed};
use quote::{quote, format_ident};

#[proc_macro]
pub fn generate_wasm_entrypoint(item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let token = item.into_iter().next().expect("Must provide a type to generate_wasm_entrypoint!(YourType)");
    let id_str = match token {
        proc_macro::TokenTree::Ident(id) => id.to_string(),
        e => {
            panic!("Expected identifier to generate_wasm_entrypoint!(). Instead found {:?}", e);
        }
    };

    let out_str = format!(r#"
    extern \"C\" {{
        fn get_entrypoint_alloc_size() -> u32;
        fn get_entrypoint_data(ptr: *const u8, len: u32);
    }}

    #[no_mangle]
    pub extern fn wasm_entrypoint() -> u32 {{
        let input_obj = unsafe {{
            let len = get_entrypoint_alloc_size() as usize;
            let mut data: Vec<u8> = Vec::with_capacity(len);
            data.set_len(len);
            let ptr = data.as_ptr();
            let len = data.len();
            get_entrypoint_data(ptr, len as _);
            {id_str}::from_binary_slice(data).unwrap()
        }};
        wasm_main(input_obj)
    }}
    "#);

    format!("\"{out_str}\"").parse().unwrap()
}

#[proc_macro]
pub fn generate_parsing_traits(_item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let trait_stuff = quote! {
        pub trait ToBinarySlice {
            fn add_to_slice(&self, data: &mut Vec<u8>);
        }

        pub trait FromBinarySlice {
            fn get_from_slice(index: &mut usize, data: &[u8]) -> Option<Self> where Self: Sized;
        }

        impl ToBinarySlice for String {
            #[inline(always)]
            fn add_to_slice(&self, data: &mut Vec<u8>) {
                let self_bytes = self.as_bytes();
                let len_u32 = self_bytes.len() as u32;
                let len_be_bytes = len_u32.to_be_bytes();
                data.extend(len_be_bytes);
                data.extend(self_bytes);
            }
        }

        impl ToBinarySlice for u32 {
            #[inline(always)]
            fn add_to_slice(&self, data: &mut Vec<u8>) {
                let self_bytes = self.to_be_bytes();
                let len_u32 = self_bytes.len() as u32;
                let len_be_bytes = len_u32.to_be_bytes();
                data.extend(len_be_bytes);
                data.extend(self_bytes);
            }
        }

        impl FromBinarySlice for u32 {
            #[inline(always)]
            fn get_from_slice(index: &mut usize, data: &[u8]) -> Option<Self> {
                // u32's len component will always be 4.. we can skip it
                *index += 4;
                let next_data = data.get(*index..*index+4)?;
                let out = Some(u32::from_be_bytes([next_data[0], next_data[1], next_data[2], next_data[3]]));
                // skip 4 again because we consumed the u32
                *index += 4;
                out
            }
        }

        impl FromBinarySlice for String {
            #[inline(always)]
            fn get_from_slice(index: &mut usize, data: &[u8]) -> Option<Self> {
                let first_4 = data.get(*index..*index + 4)?;
                *index += 4;
                let first_4_u32_bytes = [first_4[0], first_4[1], first_4[2], first_4[3]];
                let len = u32::from_be_bytes(first_4_u32_bytes) as usize;
                let next_data = data.get(*index..*index + len)?;
                let out = Some(String::from_utf8_lossy(next_data).to_string());
                *index += len;
                out
            }
        }

        impl<T: ToBinarySlice> ToBinarySlice for Option<T> {
            #[inline(always)]
            fn add_to_slice(&self, data: &mut Vec<u8>) {
                match self {
                    Some(t) => {
                        t.add_to_slice(data);
                    }
                    None => {
                        data.extend([255, 255, 255, 255]);
                    }
                }
            }
        }
        
        impl<T: FromBinarySlice> FromBinarySlice for Option<T> {
            #[inline(always)]
            fn get_from_slice(index: &mut usize, data: &[u8]) -> Option<Self>where Self:Sized {
                let first_4 = data.get(*index..*index + 4)?;
                let first_4_u32_bytes = [first_4[0], first_4[1], first_4[2], first_4[3]];
                if first_4_u32_bytes == [255, 255, 255, 255] {
                    *index += 4;
                    return Some(None);
                }
                let t_thing = T::get_from_slice(index, data)?;
                Some(Some(t_thing))
            }
        }

        impl<T: ToBinarySlice> ToBinarySlice for Vec<T> {
            #[inline(always)]
            fn add_to_slice(&self, data: &mut Vec<u8>) {
                let len_u32 = self.len() as u32;
                let len_be_bytes = len_u32.to_be_bytes();
                data.extend(len_be_bytes);
                for obj in self.iter() {
                    obj.add_to_slice(data);
                }
            }
        }
        impl<T: FromBinarySlice> FromBinarySlice for Vec<T> {
            #[inline(always)]
            fn get_from_slice(index: &mut usize, data: &[u8]) -> Option<Self>where Self:Sized {
                let first_4 = data.get(*index..*index + 4)?;
                let first_4_u32_bytes = [first_4[0], first_4[1], first_4[2], first_4[3]];
                let len = u32::from_be_bytes(first_4_u32_bytes) as usize;
                *index += 4;
                let mut out = Vec::with_capacity(len);
                for i in 0..len {
                    out.push(T::get_from_slice(index, data)?);
                }
                Some(out)
            }
        }
    };

    let trait_stuff_str = trait_stuff.to_string();
    let expanded = quote! {
        #trait_stuff

        const PARSING_TRAIT_STR: &'static str = #trait_stuff_str;
    };

    TokenStream::from(expanded)
}

fn set_include_wasm(add_includes: &mut Vec<proc_macro2::TokenStream>, ty: &Type) {
    if let syn::Type::Path(p) = ty {
        let type_path = p.path.segments.last()
            .map(|f| f.ident.to_string()).unwrap_or("u32".to_string());
        match type_path.as_str() {
            "u32" => {},
            "String" => {},
            "Option" | "Vec" => {
                if let Some(last_seg) = &p.path.segments.last() {
                    if let syn::PathArguments::AngleBracketed(ab) = &last_seg.arguments {
                        if let Some(syn::GenericArgument::Type(p)) = ab.args.first() {
                            set_include_wasm(add_includes, p);
                        }
                    }
                }
            },
            // this is a non-standard type, so we
            // want to add this to the string that should be exported to the wasm file.
            _ => {
                add_includes.push(quote! {
                    #ty::include_in_rs_wasm(),
                });
            }
        }
    }
}

/// Returns a tuple of:
/// - Vec of token streams, each one is an 'add_include' to the generated include_in_rs_wasm() function
/// - and the impl block as 1 TokenStream
fn wasm_type_gen_struct_named_fields(
    struct_name: &proc_macro2::Ident,
    fields: &FieldsNamed,
) -> (Vec<proc_macro2::TokenStream>, proc_macro2::TokenStream) {
    let fields = &fields.named;
    let add_to_slice_fields = fields.iter().map(|field| {
        let ident = &field.ident;
        quote! {
            self.#ident.add_to_slice(&mut self_data);
        }
    });

    let get_from_slice_fields = fields.iter().map(|field| {
        let ty = &field.ty;
        let ident = &field.ident;
        quote! {
            let #ident: #ty = <_>::get_from_slice(index, data)?;
        }
    });

    let field_names = fields.iter().map(|field| {
        let ident = &field.ident;
        quote! {
            #ident,
        }
    });

    let mut add_includes = vec![];
    for field in fields.iter() {
        let ty = &field.ty;
        set_include_wasm(&mut add_includes, ty);
    }

    (add_includes, quote! {
        impl ToBinarySlice for #struct_name {
            #[inline(always)]
            fn add_to_slice(&self, data: &mut Vec<u8>) {
                let mut self_data = vec![];
                #(#add_to_slice_fields)*
                let self_data_len = self_data.len() as u32;
                let self_data_bytes = self_data_len.to_be_bytes();
                data.extend(self_data_bytes);
                data.extend(self_data);
            }
        }

        impl FromBinarySlice for #struct_name {
            #[allow(unused_assignments)]
            #[inline(always)]
            fn get_from_slice(index: &mut usize, data: &[u8]) -> Option<Self> {
                // to skip the size of Self
                *index += 4;
                #(#get_from_slice_fields)*
                Some(Self {
                    #(#field_names)*
                })
            }
        }
    })
}

/// Returns a tuple of:
/// - Vec of token streams, each one is an 'add_include' to the generated include_in_rs_wasm() function
/// - and the impl block as 1 TokenStream
fn wasm_type_gen_enum_named_fields(
    name: &proc_macro2::Ident,
    dataenum: &DataEnum,
) -> (Vec<proc_macro2::TokenStream>, proc_macro2::TokenStream) {
    let variants = &dataenum.variants;
    let num_variants = variants.len();
    if num_variants == 0 {
        panic!("Cannot derive WasmTypeGen for enum with 0 variants");
    }

    let add_to_slice_variants = variants.iter().enumerate().map(|(index, v)| {
        let ident = &v.ident;
        match &v.fields {
            Fields::Named(fields) => {
                let field_names = fields.named.iter().map(|field| {
                    let ident = &field.ident;
                    quote! {
                        #ident,
                    }
                });
                let field_names_add_to_self_data = fields.named.iter().map(|field| {
                    let ident = &field.ident;
                    quote! {
                        #ident.add_to_slice(&mut self_data);
                    }
                });
                quote! {
                    Self::#ident { #(#field_names)* } => {
                        let variant: u32 = #index as _;
                        let variant_bytes = variant.to_be_bytes();
                        self_data.extend(variant_bytes);
                        // there's data so add it:
                        #(#field_names_add_to_self_data)*
                    }
                }
            }
            Fields::Unnamed(fields) => {
                let field_names = fields.unnamed.iter().enumerate().map(|(index, _)| {
                    let varname = format_ident!("a{}", index);
                    quote! {
                        #varname,
                    }
                });
                let field_names_add_to_self_data = fields.unnamed.iter().enumerate().map(|(index, _)| {
                    let varname = format_ident!("a{}", index);
                    quote! {
                        #varname.add_to_slice(&mut self_data);
                    }
                });
                quote! {
                    Self::#ident(#(#field_names)*) => {
                        let variant: u32 = #index as _;
                        let variant_bytes = variant.to_be_bytes();
                        self_data.extend(variant_bytes);
                        // there's data so add it:
                        #(#field_names_add_to_self_data)*
                    }
                }
            }
            Fields::Unit => {
                quote! {
                    Self::#ident => {
                        let variant: u32 = #index as _;
                        let variant_bytes = variant.to_be_bytes();
                        self_data.extend(variant_bytes);
                        // unit variant, no need to add data.
                    }
                }
            }
        }
    });

    let get_from_slice_variants = variants.iter().enumerate().map(|(index, v)| {
        let ident = &v.ident;
        let index = index as u32;

        let variant_data_fill = match &v.fields {
            Fields::Named(fields) => {
                let field_names = fields.named.iter().map(|field| {
                    let ident = &field.ident;
                    quote! {
                        #ident,
                    }
                });
                let fields_fill_data = fields.named.iter().map(|field| {
                    let ident = &field.ident;
                    let ty = &field.ty;
                    quote! {
                        let #ident: #ty = <_>::get_from_slice(index, data)?;
                    }
                });
                quote!{
                    #(#fields_fill_data)*
                    Self::#ident { #(#field_names)* }
                }
            }
            Fields::Unnamed(fields) => {
                let field_names = fields.unnamed.iter().enumerate().map(|(index, _)| {
                    let varname = format_ident!("a{}", index);
                    quote! {
                        #varname,
                    }
                });
                let fields_fill_data = fields.unnamed.iter().enumerate().map(|(index, field)| {
                    let varname = format_ident!("a{}", index);
                    let ty = &field.ty;
                    quote! {
                        let #varname: #ty = <_>::get_from_slice(index, data)?;
                    }
                });
                quote!{
                    #(#fields_fill_data)*
                    Self::#ident(#(#field_names)*)
                }
            }
            Fields::Unit => {
                quote!{
                    Self::#ident
                }
            }
        };

        quote! {
            #index => {
                #variant_data_fill
            }
        }
    });

    let mut add_includes = vec![];
    for variant in variants {
        match &variant.fields {
            Fields::Unit => {}
            Fields::Named(fields) => {
                for field in &fields.named {
                    set_include_wasm(&mut add_includes, &field.ty);
                }
            }
            Fields::Unnamed(fields) => {
                for field in &fields.unnamed {
                    set_include_wasm(&mut add_includes, &field.ty);
                }
            }
        }
    }
    (add_includes, quote! {
        impl ToBinarySlice for #name {
            #[inline(always)]
            fn add_to_slice(&self, data: &mut Vec<u8>) {
                let mut self_data: Vec<u8> = vec![];
                match self {
                    #(#add_to_slice_variants)*
                }
                let self_data_len = self_data.len() as u32;
                let self_data_bytes = self_data_len.to_be_bytes();
                data.extend(self_data_bytes);
                data.extend(self_data);
            }
        }

        impl FromBinarySlice for #name {
            #[allow(unused_assignments)]
            #[inline(always)]
            fn get_from_slice(index: &mut usize, data: &[u8]) -> Option<Self> {
                // skip self len
                *index += 4;
                let first_4 = data.get(*index..*index + 4)?;
                *index += 4;
                let first_4_u32_bytes = [first_4[0], first_4[1], first_4[2], first_4[3]];
                let variant = u32::from_be_bytes(first_4_u32_bytes);
                Some(match variant {
                    #(#get_from_slice_variants)*
                    _ => return None,
                })
            }
        }
    })
}

/// Returns a tuple of:
/// - Vec of token streams, each one is an 'add_include' to the generated include_in_rs_wasm() function
/// - and the impl block as 1 TokenStream
fn wasm_type_gen_struct_unnamed_fields(
    struct_name: &proc_macro2::Ident,
    fields: &FieldsUnnamed,
) -> (Vec<proc_macro2::TokenStream>, proc_macro2::TokenStream) {
    let fields = &fields.unnamed;
    let add_to_slice_fields = fields.iter().enumerate().map(|(index, _)| {
        let index = syn::Index::from(index);
        quote! {
            self.#index.add_to_slice(&mut self_data);
        }
    });

    let get_from_slice_fields = fields.iter().enumerate().map(|(index, field)| {
        let ty = &field.ty;
        let index = syn::Index::from(index);
        let varname = format_ident!("a{}", index);
        quote! {
            let #varname: #ty = <_>::get_from_slice(index, data)?;
        }
    });

    let field_names = fields.iter().enumerate().map(|(index, _)| {
        let index = syn::Index::from(index);
        let varname = format_ident!("a{}", index);
        quote! {
            #varname,
        }
    });

    let mut add_includes = vec![];
    for field in fields.iter() {
        let ty = &field.ty;
        set_include_wasm(&mut add_includes, ty);
    }

    (add_includes, quote! {
        impl ToBinarySlice for #struct_name {
            #[inline(always)]
            fn add_to_slice(&self, data: &mut Vec<u8>) {
                let mut self_data = vec![];
                #(#add_to_slice_fields)*
                let self_data_len = self_data.len() as u32;
                let self_data_bytes = self_data_len.to_be_bytes();
                data.extend(self_data_bytes);
                data.extend(self_data);
            }
        }

        impl FromBinarySlice for #struct_name {
            #[allow(unused_assignments)]
            #[inline(always)]
            fn get_from_slice(index: &mut usize, data: &[u8]) -> Option<Self> {
                // to skip the size of Self
                *index += 4;
                #(#get_from_slice_fields)*
                Some(Self(
                    #(#field_names)*
                ))
            }
        }
    })
}


#[proc_macro_derive(WasmTypeGen)]
pub fn module(item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let item_cloned = item.clone();
    let thing = parse_macro_input!(item_cloned as DeriveInput);
    let name = thing.ident;
    let structdef = item.to_string();

    // Get a list of the fields in the struct
    let (add_includes, transfer_impl_block) = match thing.data {
        Data::Struct(ref data) => match data.fields {
            Fields::Named(ref fields) => wasm_type_gen_struct_named_fields(&name, fields),
            Fields::Unnamed(ref fields) => wasm_type_gen_struct_unnamed_fields(&name, fields),
            Fields::Unit => unimplemented!("WasmTypeGen not implemented for Unit structs"),
        },
        Data::Enum(ref data) => {
            wasm_type_gen_enum_named_fields(&name, data)
        },
        _ => unimplemented!(),
    };
    let transfer_impl_block2 = quote! {
        impl #name {
            #[allow(dead_code)]
            #[inline(always)]
            pub fn to_binary_slice(&self) -> Vec<u8> {
                let mut out = vec![];
                self.add_to_slice(&mut out);
                out
            }
            #[allow(dead_code)]
            #[allow(unused_assignments)]
            #[inline(always)]
            pub fn from_binary_slice(data: Vec<u8>) -> Option<Self> {
                let mut index = 0;
                // let first_4 = data.get(index..index + 4)?;
                // index += 4;
                // let first_4_u32_bytes = [first_4[0], first_4[1], first_4[2], first_4[3]];
                // let len = u32::from_be_bytes(first_4_u32_bytes) as usize;
                // // let next_data = data.get(index..index + len)?;
                let out: Self = <_>::get_from_slice(&mut index, &data)?;
                // index += len;
                Some(out)
            }
        }
    };
    let transfer_impl_block_str = transfer_impl_block.to_string();
    let transfer_impl_block2_str = transfer_impl_block2.to_string();

    let expanded = quote! {
        #transfer_impl_block
        #transfer_impl_block2

        impl #name {
            pub fn include_in_rs_wasm() -> String {
                let strings = [
                    #structdef,
                    #transfer_impl_block_str,
                    #transfer_impl_block2_str,
                    "",
                ];
                let mut out = strings.join("\n").to_string();
                let extras: &[String] = &[
                    #(#add_includes)*
                ];
                for extra in extras {
                    out.push('\n');
                    out.push_str(&extra);
                }
                out
            }
        }
    };

    // Hand the output tokens back to the compiler
    TokenStream::from(expanded)
}
