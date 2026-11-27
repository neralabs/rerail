//! The registry's table of interfaces (decision 0686, Operations §3): each operation's interface, read from its UKL
//! declaration (`src/operations.uglykl`, `DECLARE_KERNEL2` of `<ukl/operation.h>`), written as Rust statics
//! (`op::interfaces`). The declaration is the one definition: a slot is a buffer row (its access the slot's mode, its
//! type its block's kind), and the `ARGS` row names the params' structure.

use std::fmt::Write as _;

const SOURCE: &str = "src/operations.uglykl";

fn main() {
    println!("cargo:rerun-if-changed={SOURCE}");
    let text = std::fs::read_to_string(SOURCE).unwrap_or_else(|e| panic!("{SOURCE}: {e}"));
    let mut out = String::from("// Written by build.rs from src/operations.uglykl: each operation's interface.\n");
    let mut names = Vec::new();
    for decl in declarations(&without_comments(&text)) {
        let (name, rust) = interface(&decl).unwrap_or_else(|e| panic!("{SOURCE}: {e}"));
        if names.contains(&name) {
            panic!("{SOURCE}: {name}: declared twice");
        }
        out += &rust;
        names.push(name);
    }
    let dest = std::path::Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("interfaces.rs");
    std::fs::write(&dest, out).unwrap_or_else(|e| panic!("{}: {e}", dest.display()));
}

/// The text with its `//` and `/* */` comments blanked (strings kept).
fn without_comments(s: &str) -> String {
    let (b, mut out, mut i) = (s.as_bytes(), String::new(), 0);
    while i < b.len() {
        if b[i] == b'"' {
            let end = i + 1 + s[i + 1..].find('"').expect("an unterminated string");
            out += &s[i..=end];
            i = end + 1;
        } else if s[i..].starts_with("//") {
            i += s[i..].find('\n').unwrap_or(s.len() - i);
        } else if s[i..].starts_with("/*") {
            i += s[i..].find("*/").expect("an unterminated comment") + 2;
        } else {
            let c = s[i..].chars().next().unwrap();
            out.push(c);
            i += c.len_utf8();
        }
    }
    out
}

/// Each `DECLARE_KERNEL2(...)`'s arguments, the text between its parentheses.
fn declarations(s: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = s;
    while let Some(at) = rest.find("DECLARE_KERNEL2(") {
        let body = &rest[at + "DECLARE_KERNEL2(".len()..];
        let (mut depth, mut quoted, mut end) = (1, false, None);
        for (i, c) in body.char_indices() {
            match c {
                '"' => quoted = !quoted,
                '(' if !quoted => depth += 1,
                ')' if !quoted => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let end = end.expect("a DECLARE_KERNEL2 with no closing parenthesis");
        found.push(body[..end].to_string());
        rest = &body[end + 1..];
    }
    found
}

/// The arguments split at their top-level commas, each trimmed.
fn split(s: &str) -> Vec<&str> {
    let (mut parts, mut start, mut quoted) = (Vec::new(), 0, false);
    for (i, c) in s.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => {
                parts.push(s[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(s[start..].trim());
    parts
}

/// A declaration's interface as a Rust static: `(name, code)`.
fn interface(decl: &str) -> Result<(String, String), String> {
    let args = split(decl);
    let [_library, name, rows @ ..] = args.as_slice() else { return Err(format!("a declaration with no name: {decl}")) };
    if rows.len() % 4 != 0 {
        return Err(format!("{name}: its rows are not quads (access, type, name, bounds)"));
    }
    let rows: Vec<&[&str]> = rows.chunks(4).collect();
    let Some((last, slots)) = rows.split_last() else { return Err(format!("{name}: no rows")) };
    let ["ARGS", params, _, _] = **last else { return Err(format!("{name}: its last row is not its params' ARGS row")) };
    let mut code = format!("pub static {}: Interface = Interface {{\n    name: {name:?},\n    slots: &[\n", name.to_uppercase());
    for row in slots {
        let [access, ty, slot, _] = **row else { unreachable!() };
        let kind = ty
            .strip_prefix("ukl_")
            .and_then(|t| t.strip_suffix("_block"))
            .filter(|k| ["facts", "scratch", "atoms", "control", "cache", "tape"].contains(k))
            .ok_or_else(|| format!("{name}: slot {slot}: {ty} is not a block's type (`<ukl/operation.h>`)"))?;
        let mode = match (access, kind) {
            ("ROBUFFER", _) => "Read",
            ("WOBUFFER", _) => "Write",
            ("RWBUFFER", "scratch") => "Scratch",
            ("RWBUFFER", _) => "ReadWrite",
            _ => return Err(format!("{name}: slot {slot}: {access} is not a slot's access (ROBUFFER, WOBUFFER, RWBUFFER)")),
        };
        writeln!(code, "        SlotSpec {{ name: {slot:?}, mode: Mode::{mode}, kind: kind::{} }},", kind.to_uppercase()).unwrap();
    }
    writeln!(code, "    ],\n    params: {params:?},\n}};\n").unwrap();
    Ok((name.to_string(), code))
}
