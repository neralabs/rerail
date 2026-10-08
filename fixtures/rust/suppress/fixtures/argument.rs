// Rerail-authored fixture (Apache-2.0). Lint attributes on elements of comma-separated expression lists (call
// arguments, array elements, struct-expression fields) are stable and scope to that element.

fn id(x: i32) -> i32 {
    x
}

pub fn call_argument() -> i32 {
    id(#[allow(clippy::double_parens)] ((1))) + id(((2)))
}

pub fn array_element() -> [i32; 2] {
    [#[allow(clippy::double_parens)] ((3)), ((4))]
}

pub struct P {
    pub a: i32,
    pub b: i32,
}

pub fn struct_field() -> P {
    P {
        #[allow(clippy::double_parens)]
        a: ((5)),
        b: ((6)),
    }
}
