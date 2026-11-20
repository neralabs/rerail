//! C's and C++'s preprocessor as answers inside the fixpoint (decision 0327).
//!
//! - **Mode (a)**, `c_pp_file_req(path)`: a file's directives whatever the configuration, by the
//!   compiled directive grammar's tape (`rerail_c_directives`), the lean scan where it refuses the
//!   file (`Directives`).
//! - **Mode (b)**, `c_pp_tu_req(tu, spec)`: a translation unit preprocessed under a configuration, by
//!   `rerail_c_pp::Pp` (`Units`). `spec` is the configuration as rules write it (`c/env.rerail`'s `c_cfg_spec`).
//!
//! The rules that read them are `docs/rule-syntax/lib/c/preprocess.rerail`. Positions are the files' bytes (a
//! directive's is its `#`'s, as mode (a)'s `c_directive` start); `line` is from 1.

use crate::code_rule_dsl::language::{Answerer, Cell, Reader};
use rerail_c_pp::files::{DirKind, Files, Search};
use rerail_c_pp::sink::{Diag, FactSink};
use rerail_c_pp::{Config, Options, Pp};
use std::collections::HashMap;

/// The answerer of `c_pp_file_req`.
pub struct Directives;

const RELATIONS: &[&str] = &["c_directive", "c_define", "c_undef", "c_include", "c_cond", "c_cond_next", "c_cond_parent", "c_pragma", "c_line", "c_text", "c_guard", "c_pp_problem"];

/// How a file's directives are read: a `.c` file as C23 with GNU's extensions, every other (a header could be
/// either) as C++26 with them. The two differ only where C++ has a token C lacks (a raw string, `::` before C23).
pub fn options_for(path: &str) -> Options {
    if path.ends_with(".c") || path.ends_with(".m") { Options::c(23, true) } else { Options::cpp(26, true) }
}

fn text(b: &[u8]) -> Cell {
    Cell::Text(String::from_utf8_lossy(b).into_owned())
}

impl Answerer for Directives {
    fn request(&self) -> &'static str {
        "c_pp_file_req"
    }

    fn produces(&self) -> &'static [&'static str] {
        RELATIONS
    }

    fn answer(&self, key: &[&str], files: &mut dyn Reader, wanted: &dyn Fn(&str) -> bool, out: &mut dyn FnMut(&'static str, &[Cell])) {
        let path = key[0];
        let Some(bytes) = files.bytes(path) else { return };
        // The compiled grammar's tape where it reads the file, else the lean scan (0327).
        let d = rerail_c_directives::facts(&bytes, options_for(path));
        let int = |v: u32| Cell::Int(i64::from(v));
        let pos = |(a, b): (u32, u32)| [int(a), int(b)];
        if wanted("c_directive") {
            for (i, x) in d.directives.iter().enumerate() {
                out("c_directive", &[int(i as u32), Cell::Text(x.kind.into()), int(x.start), int(x.end), int(x.line)]);
            }
        }
        if wanted("c_cond_parent") {
            for (i, x) in d.directives.iter().enumerate() {
                if let Some(p) = x.parent {
                    out("c_cond_parent", &[int(i as u32), int(p)]);
                }
            }
        }
        if wanted("c_define") {
            for x in &d.defines {
                let [a, b] = pos(x.body);
                out("c_define", &[int(x.i), text(&x.name), Cell::Int(i64::from(x.fnlike)), text(&x.params.join(&b","[..])), Cell::Text(x.variadic.into()), a, b]);
            }
        }
        if wanted("c_undef") {
            for (i, name) in &d.undefs {
                out("c_undef", &[int(*i), text(name)]);
            }
        }
        if wanted("c_include") {
            for x in &d.includes {
                out("c_include", &[int(x.i), Cell::Text(x.kind.into()), Cell::Text(x.form.into()), text(&x.spelled)]);
            }
        }
        if wanted("c_cond") || wanted("c_cond_next") {
            for x in &d.conds {
                if wanted("c_cond") {
                    let [a, b] = pos(x.expr);
                    out("c_cond", &[int(x.i), Cell::Text(x.kind.into()), a, b]);
                }
                if let (true, Some(j)) = (wanted("c_cond_next"), x.next) {
                    out("c_cond_next", &[int(x.i), int(j)]);
                }
            }
        }
        if wanted("c_pragma") {
            for x in &d.pragmas {
                out("c_pragma", &[int(x.i), text(&x.first), text(&x.second), text(&x.text)]);
            }
        }
        if wanted("c_line") {
            for x in &d.lines {
                // A line given by macros (`n` unknown) is -1, its tokens in `file`'s place.
                let n = x.n.map_or(-1, |n| i64::try_from(n).unwrap_or(i64::MAX));
                let file = if x.n.is_some() { text(&x.file) } else { text(&x.rest) };
                out("c_line", &[int(x.i), Cell::Int(n), file]);
            }
        }
        if wanted("c_text") {
            for (i, t) in &d.texts {
                out("c_text", &[int(*i), text(t)]);
            }
        }
        if let (true, Some((i, name))) = (wanted("c_guard"), &d.guard) {
            out("c_guard", &[int(*i), text(name)]);
        }
        if wanted("c_pp_problem") {
            for &(i, kind) in &d.problems {
                out("c_pp_problem", &[Cell::Int(i), Cell::Text(kind.into())]);
            }
        }
    }
}

/// The answerer of `c_pp_tu_req(tu, spec)`.
pub struct Units;

const UNIT_RELATIONS: &[&str] = &["c_pp_read", "c_pp_included", "c_pp_include_wanted", "c_pp_cond_taken", "c_pp_active", "c_pp_expansion", "c_pp_diag"];

/// A configuration, read from its spec: items, each a tag and fields, every field `n:text` with `n` the text's
/// length in characters (`c/env.rerail`'s `c_cfg_item`):
///
/// | tag | fields | from |
/// |---|---|---|
/// | `L` | language, standard | `c_lang` |
/// | `D` | name, body | `c_predefine` |
/// | `U` | name | `c_config_undef` |
/// | `I` | kind, dir | `c_include_dir` |
/// | `F` | test, operand, value | `c_feature` |
/// | `X` | name, value | `c_fixed` |
///
/// `None` where the spec is not one.
fn read_spec(spec: &str) -> Option<(Config, Vec<(String, DirKind)>)> {
    let mut rest = spec;
    let field = |rest: &mut &str| -> Option<String> {
        let (n, tail) = rest.split_once(':')?;
        let n: usize = n.parse().ok()?;
        let end = tail.char_indices().nth(n).map_or(tail.len(), |(k, _)| k);
        (tail[..end].chars().count() == n).then_some(())?;
        *rest = &tail[end..];
        Some(tail[..end].to_owned())
    };
    let mut items: Vec<(char, Vec<String>)> = Vec::new();
    while let Some(tag) = rest.chars().next() {
        rest = &rest[tag.len_utf8()..];
        let n = match tag {
            'L' | 'D' | 'I' | 'X' => 2,
            'U' => 1,
            'F' => 3,
            _ => return None,
        };
        let fields = (0..n).map(|_| field(&mut rest)).collect::<Option<Vec<_>>>()?;
        items.push((tag, fields));
    }
    let opts = items.iter().find(|(t, _)| *t == 'L').map_or(Options::c(23, true), |(_, f)| options_of(&f[0], &f[1]));
    let mut cfg = Config::new(opts);
    let mut dirs = Vec::new();
    for (tag, f) in items {
        match tag {
            'D' => cfg.predefines.push((f[0].clone(), Some(f[1].clone()))),
            'U' => cfg.predefines.push((f[0].clone(), None)),
            'I' => {
                let kind = match f[0].as_str() {
                    "quote" => DirKind::Quote,
                    "angle" => DirKind::Angle,
                    "system" => DirKind::System,
                    "framework" => DirKind::Framework,
                    "after" => DirKind::After,
                    _ => return None,
                };
                dirs.push((f[1].clone(), kind));
            }
            'F' => {
                cfg.features.insert((f[0].clone(), f[1].clone()), f[2].parse().ok()?);
            }
            'X' => match f[0].as_str() {
                "__DATE__" => cfg.date = f[1].clone(),
                "__TIME__" => cfg.time = f[1].clone(),
                "__TIMESTAMP__" => cfg.timestamp = f[1].clone(),
                _ => return None,
            },
            _ => {}
        }
    }
    Some((cfg, dirs))
}

/// A language (`c`, `c++`, `objective-c`, `objective-c++`) and standard (`c17`, `gnu17`, `c++20`, `gnu++2c`, …) as
/// the lexer's options; an unknown standard is the latest.
fn options_of(lang: &str, std: &str) -> Options {
    let cpp = lang.ends_with("++");
    let gnu = std.starts_with("gnu");
    let y = std.trim_start_matches("gnu").trim_start_matches('c').trim_start_matches("++");
    // A standard's draft names: C's `2x` and `2y`, C++'s `0x`, `1y`, `1z`, `2a`, `2b`, `2c`.
    let year = match y {
        "2x" => 23,
        "2y" => 26,
        "0x" => 11,
        "1y" => 14,
        "1z" => 17,
        "2a" => 20,
        "2b" => 23,
        "2c" => 26,
        y => y.parse().unwrap_or(if cpp { 26 } else { 23 }),
    };
    let year = if !cpp && year == 90 { 89 } else { year };
    if cpp { Options::cpp(year, gnu) } else { Options::c(year, gnu) }
}

fn diag_name(d: &Diag) -> &'static str {
    match d {
        Diag::Error => "error",
        Diag::Warning => "warning",
        Diag::NotFound => "not_found",
        Diag::TooDeep => "too_deep",
        Diag::Unterminated => "unterminated",
        Diag::Unmatched => "unmatched",
        Diag::AfterElse => "after_else",
        Diag::BadExpr(_) => "bad_expr",
        Diag::UnterminatedCall => "unterminated_call",
        Diag::ArgCount => "arg_count",
        Diag::BadPaste => "bad_paste",
        Diag::Malformed => "malformed",
        Diag::UnknownDirective => "unknown_directive",
        Diag::Redefined => "redefined",
        Diag::Poisoned => "poisoned",
    }
}

impl Answerer for Units {
    fn request(&self) -> &'static str {
        "c_pp_tu_req"
    }

    fn key_width(&self) -> usize {
        2
    }

    fn produces(&self) -> &'static [&'static str] {
        UNIT_RELATIONS
    }

    fn read_set(&self) -> Option<&'static str> {
        Some("c_pp_read")
    }

    fn answer(&self, key: &[&str], files: &mut dyn Reader, wanted: &dyn Fn(&str) -> bool, out: &mut dyn FnMut(&'static str, &[Cell])) {
        let (tu, spec) = (key[0], key[1]);
        // Every path the pass looked at, found or not: a header made where the search looked first changes the unit.
        let mut seen: HashMap<String, Option<Vec<u8>>> = HashMap::new();
        let Some((cfg, dirs)) = read_spec(spec) else {
            out("c_pp_diag", &[text(tu.as_bytes()), Cell::Int(-1), Cell::Text("bad_spec".into()), Cell::Text(String::new())]);
            return;
        };
        let mut facts = FactSink::default();
        {
            let mut io = Search::new(dirs, |p: &str| seen.entry(p.to_owned()).or_insert_with(|| files.bytes(p)).clone());
            let mut pp = Pp::new(cfg, &mut io as &mut dyn Files, &mut facts);
            pp.run(tu);
        }
        let path = |f: u32| Cell::Text(facts.paths.get(f as usize).cloned().unwrap_or_default());
        let int = |v: u32| Cell::Int(i64::from(v));
        // Always given, whether the program reads it or not: the journal keeps the answer by it.
        let mut read: Vec<&String> = seen.keys().collect();
        read.sort();
        for p in read {
            out("c_pp_read", &[Cell::Text(p.clone())]);
        }
        if wanted("c_pp_included") {
            for &(f, at, to, depth) in &facts.included {
                out("c_pp_included", &[path(f), int(at), path(to), int(depth)]);
            }
        }
        if wanted("c_pp_include_wanted") {
            for (f, at, spelled, angle) in &facts.missing {
                out("c_pp_include_wanted", &[path(*f), int(*at), text(spelled), Cell::Text(if *angle { "angle" } else { "quote" }.into())]);
            }
        }
        if wanted("c_pp_cond_taken") {
            for &(f, at, taken) in &facts.conds {
                out("c_pp_cond_taken", &[path(f), int(at), Cell::Int(i64::from(taken))]);
            }
        }
        if wanted("c_pp_active") {
            // Each entered file's bytes less its skipped groups (the main file is file 1, `<built-in>` 0).
            let mut skipped: HashMap<u32, Vec<(u32, u32)>> = HashMap::new();
            for &(f, s, e) in &facts.skipped {
                skipped.entry(f).or_default().push((s, e));
            }
            for f in 1..facts.paths.len() as u32 {
                let p = &facts.paths[f as usize];
                let Some(len) = seen.get(p).and_then(|b| b.as_ref()).map(|b| b.len() as u32) else { continue };
                let mut gaps = skipped.remove(&f).unwrap_or_default();
                gaps.sort();
                let mut at = 0;
                for (s, e) in gaps.into_iter().chain([(len, len)]) {
                    if s > at {
                        out("c_pp_active", &[path(f), int(at), int(s)]);
                    }
                    at = at.max(e);
                }
            }
        }
        if wanted("c_pp_expansion") {
            for e in &facts.expansions {
                let ((df, da), (f, at)) = (e.defined, e.called);
                out("c_pp_expansion", &[int(e.id), text(&e.name), path(df), int(da), path(f), int(at), int(e.parent)]);
            }
        }
        if wanted("c_pp_diag") {
            for (f, at, d, t) in &facts.diags {
                out("c_pp_diag", &[path(*f), int(*at), Cell::Text(diag_name(d).into()), text(t)]);
            }
        }
    }
}

/// The answerer of `c_cc_file_req(path)`: a `compile_commands.json`'s entries (decision 0327, "Configuration is
/// facts"). The rules that make configurations of them are `docs/rule-syntax/lib/c/compile-commands.rerail`.
pub struct CompileCommands;

const CC_RELATIONS: &[&str] = &["c_compile_command", "c_compile_arg"];

impl Answerer for CompileCommands {
    fn request(&self) -> &'static str {
        "c_cc_file_req"
    }

    fn produces(&self) -> &'static [&'static str] {
        CC_RELATIONS
    }

    /// Entry `e` (from 0): `c_compile_command(path, e, file, directory)`, its `file` as written (rules join it to
    /// `directory`), and `c_compile_arg(path, e, i, arg)`, its `arguments`, or its `command` split as a shell
    /// splits it (`shell_words`), from 0 (the compiler). A file that is no JSON array gives no row.
    fn answer(&self, key: &[&str], files: &mut dyn Reader, wanted: &dyn Fn(&str) -> bool, out: &mut dyn FnMut(&'static str, &[Cell])) {
        let Some(bytes) = files.bytes(key[0]) else { return };
        let Ok(serde_json::Value::Array(entries)) = serde_json::from_slice::<serde_json::Value>(&bytes) else { return };
        for (e, entry) in entries.iter().enumerate() {
            let field = |k: &str| entry.get(k).and_then(serde_json::Value::as_str);
            let (Some(file), Some(dir)) = (field("file"), field("directory")) else { continue };
            let e = Cell::Int(e as i64);
            if wanted("c_compile_command") {
                out("c_compile_command", &[e.clone(), Cell::Text(file.to_owned()), Cell::Text(dir.to_owned())]);
            }
            if wanted("c_compile_arg") {
                let args: Vec<String> = match (entry.get("arguments").and_then(serde_json::Value::as_array), field("command")) {
                    (Some(a), _) => a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect(),
                    (None, Some(c)) => crate::code_rule_dsl::logic::strings::functions::shell_words(c),
                    (None, None) => Vec::new(),
                };
                for (i, a) in args.into_iter().enumerate() {
                    out("c_compile_arg", &[e.clone(), Cell::Int(i as i64), Cell::Text(a)]);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spec_reads_back_as_its_configuration() {
        let (cfg, dirs) = read_spec("L1:c5:gnu17D3:FOO1:1D6:SQ(x)é5:x*x/éU3:BARI5:quote3:incF13:__has_feature3:abc1:1X8:__DATE__11:Feb  2 2024").expect("a spec");
        assert!(!cfg.opts.cpp && !cfg.opts.digit_separators);
        assert_eq!(cfg.predefines, vec![("FOO".into(), Some("1".into())), ("SQ(x)é".into(), Some("x*x/é".into())), ("BAR".into(), None)]);
        assert_eq!(dirs, vec![("inc".into(), DirKind::Quote)]);
        assert_eq!(cfg.features.get(&("__has_feature".into(), "abc".into())), Some(&1));
        assert_eq!(cfg.date, "Feb  2 2024");
        assert!(read_spec("D3:FO").is_none());
        assert!(options_of("c++", "gnu++2c").cpp);
        assert!(Options::c(99, false).trigraphs && !Options::c(99, false).digit_separators);
    }
}
