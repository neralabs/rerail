//! A request a native step answers (t33 K3, decision 0065 §2a): declared in the library as
//!
//! ```text
//! .answer rr_dir_req -> dir_entry
//! .answer c_pp_file_req -> c_directive, c_define, c_undef
//! ```
//!
//! the request relation, and the relations its answer adds rows to. The engine's answerers (`rr::WORLD`, and `load`'s
//! `loaded`) are the native steps; every one is declared in the library, and the library declares no other
//! (`rr::tests::every_native_answer_is_declared_in_the_library`). A declaration is read like `.cache`
//! (`front::Program::settings`); no evaluation reads it.
//!
//! The lint here: each `.answer` is of that form, and no relation is answered by two requests. A program need not
//! declare what an answer names: one that reads none of it (a pruned program, an overlay's merge) has the library's
//! declaration all the same, and a language's answer gives only the relations the program declares (0327).

use super::Error;
use crate::code_rule_dsl::logic::program_image::Version;
use crate::code_rule_dsl::logic::program_image::fields::ProgramFields;
use std::collections::BTreeMap;

/// A request and the relations its answer adds rows to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub req: String,
    pub produces: Vec<String>,
}

/// `.answer`'s text (`REQ -> R1, R2`), read; an error says what is wrong.
pub fn parse(text: &str) -> Result<Answer, String> {
    let shape = || format!("`.answer {text}`: write `.answer REQ -> R1, R2`");
    let (req, produces) = text.split_once("->").ok_or_else(shape)?;
    let name = |s: &str| {
        let s = s.trim().trim_end_matches('.');
        (!s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')).then(|| s.to_owned())
    };
    let req = name(req).ok_or_else(shape)?;
    let produces: Vec<String> = produces.split(',').map(name).collect::<Option<_>>().ok_or_else(shape)?;
    Ok(Answer { req, produces })
}

/// The program's declared answers, linted; an error names what the lint refuses.
pub fn answers(v: &Version) -> Result<Vec<Answer>, Error> {
    let bad = |msg: String| Error::Syntax(format!(".answer: {msg} (t33 K3: a request a native step answers, and the relations it adds rows to)"));
    let mut out = Vec::new();
    let mut by: BTreeMap<String, String> = BTreeMap::new();
    for (_, text) in ProgramFields::settings(v).iter().filter(|(k, _)| k == "answer") {
        let answer = parse(text).map_err(bad)?;
        for rel in &answer.produces {
            if let Some(other) = by.insert(rel.clone(), answer.req.clone())
                && other != answer.req
            {
                return Err(bad(format!("{rel} is answered by both {other} and {}", answer.req)));
            }
        }
        out.push(answer);
    }
    Ok(out)
}
