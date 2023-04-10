impl ToBinarySlice for Thing
{
    fn add_to_slice(& self, data : & mut Vec < u8 >)
    {
        let mut self_data = vec! [] ; self.s.add_to_slice(& mut self_data) ;
        self.q.add_to_slice(& mut self_data) ;
        self.opt.add_to_slice(& mut self_data) ; let self_data_len =
        self_data.len() as u32 ; let self_data_bytes =
        self_data_len.to_be_bytes() ; data.extend(self_data_bytes) ;
        data.extend(self_data) ;
    }
} impl FromBinarySlice for Thing
{
    #[allow(unused_assignments)] fn
    get_from_slice(index : & mut usize, data : & [u8]) -> Option < Self >
    {
        let s : String = < _ > :: get_from_slice(index, data) ? ; let q : u32
        = < _ > :: get_from_slice(index, data) ? ; let opt : Option < u32 > =
        < _ > :: get_from_slice(index, data) ? ; Some(Self { s, q, opt, })
    }
} impl Thing
{
    #[allow(dead_code)] pub fn to_binary_slice(& self) -> Vec < u8 >
    { let mut out = vec! [] ; self.add_to_slice(& mut out) ; out }
    #[allow(dead_code)] #[allow(unused_assignments)] pub fn
    from_binary_slice(data : Vec < u8 >) -> Option < Self >
    {
        let mut index = 0 ; let first_4 = data.get(index .. index + 4) ? ;
        index += 4 ; let first_4_u32_bytes =
        [first_4 [0], first_4 [1], first_4 [2], first_4 [3]] ; let len = u32
        :: from_be_bytes(first_4_u32_bytes) as usize ; let out : Self = < _ >
        :: get_from_slice(& mut index, & data) ? ; index += len ; Some(out)
    }
} impl Thing
{
    pub fn include_in_rs_wasm() -> String
    {
        let strings =
        ["pub struct Thing { pub s : String, pub q : u32, pub opt : Option < u32 >, }",
        "impl ToBinarySlice for Thing\n{\n    fn add_to_slice(& self, data : & mut Vec < u8 >)\n    {\n        let mut self_data = vec! [] ; self.s.add_to_slice(& mut self_data) ;\n        self.q.add_to_slice(& mut self_data) ;\n        self.opt.add_to_slice(& mut self_data) ; let self_data_len =\n        self_data.len() as u32 ; let self_data_bytes =\n        self_data_len.to_be_bytes() ; data.extend(self_data_bytes) ;\n        data.extend(self_data) ;\n    }\n} impl FromBinarySlice for Thing\n{\n    #[allow(unused_assignments)] fn\n    get_from_slice(index : & mut usize, data : & [u8]) -> Option < Self >\n    {\n        let s : String = < _ > :: get_from_slice(index, data) ? ; let q : u32\n        = < _ > :: get_from_slice(index, data) ? ; let opt : Option < u32 > =\n        < _ > :: get_from_slice(index, data) ? ; Some(Self { s, q, opt, })\n    }\n} impl Thing\n{\n    #[allow(dead_code)] pub fn to_binary_slice(& self) -> Vec < u8 >\n    { let mut out = vec! [] ; self.add_to_slice(& mut out) ; out }\n    #[allow(dead_code)] #[allow(unused_assignments)] pub fn\n    from_binary_slice(data : Vec < u8 >) -> Option < Self >\n    {\n        let mut index = 0 ; let first_4 = data.get(index .. index + 4) ? ;\n        index += 4 ; let first_4_u32_bytes =\n        [first_4 [0], first_4 [1], first_4 [2], first_4 [3]] ; let len = u32\n        :: from_be_bytes(first_4_u32_bytes) as usize ; let out : Self = < _ >\n        :: get_from_slice(& mut index, & data) ? ; index += len ; Some(out)\n    }\n}",
        "",] ; let mut out = strings.join("\n").to_string() ; let extras : &
        [String] = & [Option < u32 > :: include_in_rs_wasm()] ; for extra in
        extras { out.push('\n') ; out.push_str(& extra) ; } out
    }
}