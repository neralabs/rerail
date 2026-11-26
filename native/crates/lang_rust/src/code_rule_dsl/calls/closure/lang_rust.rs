//! Rust's linking of a call graph's names

use super::*;

/// Rust's linking of the call graph (`CallRules::link`).
pub(crate) trait RustLinks {
    /// Links a Rust crate path (`calls::rust::summary`): the one function of the workspace the
    /// path names. A path two definitions share (an inherent and a trait method of one type, one
    /// function behind two `cfg`s), a trait's method (the call dispatches), a path no function
    /// gives (a constructor of another file, a function pointer) and any path of a crate a summary
    /// names opaque (a `#[path]` module: its files do not tell its module paths) are open.
    fn link_rust(&self, name: &Name, followed: &mut Vec<Name>) -> Link;
}

impl RustLinks for Graph {
    /// Links a Rust crate path (`calls::rust::summary`): the one function of the workspace the
    /// path names. A path two definitions share (an inherent and a trait method of one type, one
    /// function behind two `cfg`s), a trait's method (the call dispatches), a path no function
    /// gives (a constructor of another file, a function pointer) and any path of a crate a summary
    /// names opaque (a `#[path]` module: its files do not tell its module paths) are open.
    fn link_rust(&self, name: &Name, followed: &mut Vec<Name>) -> Link {
        let root = Name::new(name.family(), crate::code_rule_dsl::calls::rust::crate_root(name.text()));
        let opaque = self.opaque.contains_key(&root);
        followed.push(root);
        if opaque {
            return Link {
                defs: Vec::new(),
                open: true,
            };
        }
        match self.by_name.get(name).map(Vec::as_slice) {
            Some([(file, (local, flags))]) if flags & crate::code_rule_dsl::calls::rust::TRAIT_METHOD == 0 => Link {
                defs: vec![DefRef {
                    file: *file,
                    local: *local,
                }],
                open: false,
            },
            _ => Link {
                defs: Vec::new(),
                open: true,
            },
        }
    }
}
