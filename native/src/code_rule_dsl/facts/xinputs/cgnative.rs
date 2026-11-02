//! Native's per-file Python call summary (`calls::summarize`, the reference graph's leaf;
//! facts-engine SCHEMA.md § 3.4), extracted per **content** by calling native with a symbolic
//! module path.
//!
//! Native qualifies every name of a summary through the file's `ModulePath`, which depends on the
//! worktree's `__init__.py` set. To keep the export content-keyed, the summary is extracted once
//! per variant with a symbolic module whose segments are sentinels (`\u{1}0` … `\u{1}15`, then
//! `\u{1}S` for the stem, or `__init__`). Exported names spell each maximal run of sentinel
//! segments as one token `\u{1}<run length>` (`compact`). The reader substitutes the real module
//! (Y1's `path::block`).
use ast_grep_core::AstGrep;
use crate::code_rule_dsl::calls::contracts;
use crate::code_rule_dsl::calls::summary::Summary;
use crate::code_rule_dsl::calls::summarize;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::module_path::ModulePath;
use crate::code_rule_dsl::tape::TapeDoc;
use serde_json::Value as Json;
use std::collections::BTreeMap;

/// Directory sentinels of the symbolic module (more than any real package depth needs:
/// `MAX_DIRS` bounds the relative-import level that stays exact).
pub const MAX_DIRS: usize = 16;
pub const STEM: &str = "\u{1}S";

/// The symbolic module a content is summarised under.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Variant {
    /// A module of a package (`pkg/mod.py`): directories, then the stem.
    Module,
    /// A package's `__init__.py`: directories, then `__init__`.
    Init,
    /// A file outside any package: Ruff's stem fallback (`ModulePath::stem`, not a package).
    Unpackaged,
}

impl Variant {
    pub const ALL: [Variant; 3] = [Variant::Module, Variant::Init, Variant::Unpackaged];
    pub fn name(self) -> &'static str {
        match self {
            Variant::Module => "module",
            Variant::Init => "init",
            Variant::Unpackaged => "unpackaged",
        }
    }
    pub fn module(self) -> ModulePath {
        let dirs: Vec<String> = (0..MAX_DIRS).map(|i| format!("\u{1}{i}")).collect();
        let mut names: Vec<&str> = dirs.iter().map(String::as_str).collect();
        match self {
            Variant::Module => names.push(STEM),
            Variant::Init => names.push("__init__"),
            Variant::Unpackaged => return ModulePath::stem(&format!("{STEM}.py")).expect("stem"),
        }
        ModulePath::names(&names).expect("symbolic module")
    }
}

/// A summary's relations rendered as text rows, names spelled out (SCHEMA § 3.4).
#[derive(Default, Debug)]
pub struct Rendered {
    pub rels: BTreeMap<&'static str, Vec<Vec<String>>>,
}

pub fn render(s: &Summary) -> Rendered {
    let n = |id: u32| s.name(id).to_owned();
    let i = |x: u32| x.to_string();
    let mut r = Rendered::default();
    let mut put = |rel: &'static str, rows: Vec<Vec<String>>| {
        r.rels.insert(rel, rows);
    };
    put("cgn_module", s.module.iter().map(|&m| vec![n(m)]).collect());
    put("cgn_def", s.defs.iter().enumerate().map(|(k, d)| vec![i(k as u32), n(d.name), i(d.start), i(d.end), i(d.flags as u32)]).collect());
    put("cgn_edge", s.edges.iter().map(|&(d, c)| vec![i(d), n(c)]).collect());
    put("cgn_reference", s.references.iter().map(|&(d, c)| vec![i(d), n(c)]).collect());
    put("cgn_reference_target", s.reference_targets.iter().map(|&c| vec![n(c)]).collect());
    put("cgn_reference_alias", s.reference_aliases.iter().map(|&(a, b)| vec![n(a), n(b)]).collect());
    put("cgn_reference_decorator", s.reference_decorators.iter().map(|&(d, c)| vec![i(d), n(c)]).collect());
    put("cgn_target", s.targets.iter().map(|&c| vec![n(c)]).collect());
    put("cgn_alias", s.aliases.iter().map(|&(a, b)| vec![n(a), n(b)]).collect());
    put("cgn_opaque", s.opaque.iter().map(|&c| vec![n(c)]).collect());
    put("cgn_star", s.stars.iter().map(|&c| vec![n(c)]).collect());
    put("cgn_super", s.supers.iter().map(|&(a, b, k)| vec![n(a), n(b), i(k as u32)]).collect());
    put("cgn_open", vec![vec![i(s.open as u32), i(s.open_sites as u32)]]);
    r
}

/// The exported spelling of a symbolic name: each maximal run of sentinel segments
/// (`\u{1}0.\u{1}1. … .\u{1}S`) becomes one token `\u{1}<run length>`, the only thing the
/// substitution reads.
pub fn compact(name: &str) -> String {
    if !name.contains('\u{1}') {
        return name.to_owned();
    }
    let mut out = String::with_capacity(name.len());
    let mut rest = name;
    while let Some(at) = rest.find('\u{1}') {
        out.push_str(&rest[..at]);
        let mut run = 0usize;
        let mut end = at;
        loop {
            let seg = &rest[end..];
            let len = seg.find('.').unwrap_or(seg.len());
            let token = &seg[..len];
            let is_sentinel = token.strip_prefix('\u{1}').is_some_and(|t| t == "S" || t.parse::<usize>().is_ok_and(|d| d < MAX_DIRS));
            assert!(is_sentinel || run > 0, "stray sentinel in {name:?}");
            if !is_sentinel {
                break;
            }
            run += 1;
            end += len;
            if rest[end..].starts_with(".\u{1}") {
                end += 1;
            } else {
                break;
            }
        }
        out.push('\u{1}');
        out.push_str(&run.to_string());
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

pub fn compacted(r: &Rendered) -> Rendered {
    Rendered { rels: r.rels.iter().map(|(rel, rows)| (*rel, rows.iter().map(|row| row.iter().map(|c| compact(c)).collect()).collect())).collect() }
}

/// Native's compiled links for the canonical JSON `{"constructorFields": …, "siblingLinks": …}`
/// (either key optional), as `calls::contracts::compile` reads a rule's declaration.
pub fn links_key(links: &str) -> Result<contracts::Key, String> {
    let v: Json = serde_json::from_str(links).map_err(|e| format!("links: {e}"))?;
    contracts::compile(v.get("siblingLinks"), v.get("constructorFields"), "links", 1 << 20).map_err(|e| format!("links: {e:?}"))
}

/// Native's summary of one document under `module` (`calls::summarize`, as the workspace calls it).
pub fn summary(doc: &TapeDoc, path: &str, module: &ModulePath, has_errors: bool, bytes: usize) -> Summary {
    let grep = AstGrep::doc(doc.clone());
    summarize(grep.root(), GrammarDialect::Python, path, Some(module), has_errors, bytes, None).0
}
