// Rerail-authored fixture (Apache-2.0).
use std::collections::HashMap;
use std::fmt::{self, Display as D};
use std::io::*;
use std::process::exit;
use crate::items::helper;
pub use crate::items::Widget as PublicWidget;
use self::inner::value;
use std::mem::drop as _;

mod inner { pub fn value() -> i32 { 1 } }

pub fn imports() -> String {
    let mut map: HashMap<String, i32> = HashMap::new();
    map.insert(String::from("x"), value());
    let _ = helper(map.len() as i32);
    let _display: &dyn D = &map.len();
    let _ = fmt::Error;
    let _ = stdin();
    let _ = exit as fn(i32) -> !;
    format!("{}", map.len())
}

pub fn prelude_names() -> Box<Option<String>> {
    let value: Vec<String> = vec![String::from("x")];
    Box::new(value.into_iter().next())
}
