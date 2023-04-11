use proc_macro::TokenStream;
use syn::{parse_macro_input, DeriveInput, Data, Fields, Type};
use quote::{quote, ToTokens};

#[proc_macro]
pub fn generate_wasm_entrypoint(item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    println!("{:#?}", item);
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
            fn add_to_slice(&self, data: &mut Vec<u8>) {
                let self_bytes = self.as_bytes();
                let len_u32 = self_bytes.len() as u32;
                let len_be_bytes = len_u32.to_be_bytes();
                data.extend(len_be_bytes);
                data.extend(self_bytes);
            }
        }

        impl ToBinarySlice for u32 {
            fn add_to_slice(&self, data: &mut Vec<u8>) {
                let self_bytes = self.to_be_bytes();
                let len_u32 = self_bytes.len() as u32;
                let len_be_bytes = len_u32.to_be_bytes();
                data.extend(len_be_bytes);
                data.extend(self_bytes);
            }
        }

        impl FromBinarySlice for u32 {
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
            "Option" => {
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
                    #ty::include_in_rs_wasm()
                });
            }
        }
    }
}

#[proc_macro_derive(MyThing)]
pub fn module(item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let item_cloned = item.clone();
    let thing = parse_macro_input!(item_cloned as DeriveInput);
    let struct_name = thing.ident;
    let structdef = item.to_string();

    // Get a list of the fields in the struct
    let fields = match thing.data {
        Data::Struct(ref data) => match data.fields {
            Fields::Named(ref fields) => &fields.named,
            _ => unimplemented!(),
        },
        _ => unimplemented!(),
    };

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

    let transfer_impl_block = quote! {
        impl ToBinarySlice for #struct_name {
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
            fn get_from_slice(index: &mut usize, data: &[u8]) -> Option<Self> {
                // let mut index = 0;
                #(#get_from_slice_fields)*
                Some(Self {
                    #(#field_names)*
                })
            }
        }

        impl #struct_name {
            #[allow(dead_code)]
            pub fn to_binary_slice(&self) -> Vec<u8> {
                let mut out = vec![];
                self.add_to_slice(&mut out);
                out
            }
            #[allow(dead_code)]
            #[allow(unused_assignments)]
            pub fn from_binary_slice(data: Vec<u8>) -> Option<Self> {
                let mut index = 0;
                let first_4 = data.get(index..index + 4)?;
                index += 4;
                let first_4_u32_bytes = [first_4[0], first_4[1], first_4[2], first_4[3]];
                let len = u32::from_be_bytes(first_4_u32_bytes) as usize;
                // let next_data = data.get(index..index + len)?;
                let out: Self = <_>::get_from_slice(&mut index, &data)?;
                index += len;
                Some(out)
            }
        }
    };
    let transfer_impl_block_str = transfer_impl_block.to_string();

    let expanded = quote! {
        #transfer_impl_block

        impl #struct_name {
            pub fn include_in_rs_wasm() -> String {
                let strings = [
                    #structdef,
                    #transfer_impl_block_str,
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

    // println!("OUTPUTTING\n{}", expanded.to_string());

    // Hand the output tokens back to the compiler
    TokenStream::from(expanded)
}
