// Rerail-authored fixture (Apache-2.0).

use std::collections::BTreeMap;
use std::fmt::Debug;
const UNUSED_CONST: i32 = 1;
static UNUSED_STATIC: i32 = 2;

pub fn lint_examples() {
    let unused_variable = 1;
    let _prefixed = 2;
    let mut unused_mut = 3;
    unused_mut = 4;
    let mut assigned;
    assigned = 5;
    let _ = assigned;
    let _unused_closure = || unused_variable;
}

#[allow(unused)]
mod allowed {
    use super::BTreeMap;
    const HIDDEN: i32 = 4;
    fn hidden() -> i32 { HIDDEN }
    pub fn visible() {}
}

fn dead_private() {}
