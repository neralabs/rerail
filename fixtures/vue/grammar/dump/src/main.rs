//! Offline T1 helper for the Vue pack (`../measure.mjs` runs it; never CI).
//!
//! `rerail-vue-grammar-dump <grammar.dylib> [--repeat N]` loads `tree_sitter_vue` from the shared library, reads one
//! file path per stdin line, parses each file with the tree-sitter 0.27.0 runtime Rerail's kernel links, and prints one
//! JSON line per file: `{ path, bytes, parseNs, error, nodes }`. `parseNs` is the fastest of N parses (default 3).
//! `nodes` lists every node in preorder as `[kind, startByte, endByte, parent, field, flags]`, where `parent` indexes
//! `nodes` (-1 for the root), `field` is the field name or null, and `flags` is 1 = ERROR, 2 = MISSING, 4 = extra,
//! 8 = anonymous (a grammar may spell a construct only as an anonymous token, as tree-sitter-grammars' `:`, `@`, `#`).

use std::io::{BufRead, BufWriter, Write};
use std::time::Instant;

use serde_json::{json, Value};
use tree_sitter::{Language, Parser, Tree};
use tree_sitter_language::LanguageFn;

fn load(path: &str) -> (libloading::Library, Language) {
    // SAFETY: the library is a tree-sitter grammar exporting `tree_sitter_vue`, kept alive with the language.
    unsafe {
        let library = libloading::Library::new(path).expect("load grammar library");
        let symbol: libloading::Symbol<unsafe extern "C" fn() -> *const ()> =
            library.get(b"tree_sitter_vue").expect("tree_sitter_vue symbol");
        let language = Language::new(LanguageFn::from_raw(*symbol));
        (library, language)
    }
}

fn nodes(tree: &Tree) -> Vec<Value> {
    let mut out = Vec::new();
    let mut parents: Vec<i64> = Vec::new();
    let mut cursor = tree.walk();
    loop {
        let node = cursor.node();
        let parent = parents.last().copied().unwrap_or(-1);
        let flags = u8::from(node.is_error())
            | (u8::from(node.is_missing()) << 1)
            | (u8::from(node.is_extra()) << 2)
            | (u8::from(!node.is_named()) << 3);
        out.push(json!([node.kind(), node.start_byte(), node.end_byte(), parent, cursor.field_name(), flags]));
        let index = (out.len() - 1) as i64;
        if cursor.goto_first_child() {
            parents.push(index);
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return out;
            }
            parents.pop();
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let library_path = args.get(1).expect("usage: rerail-vue-grammar-dump <grammar.dylib> [--repeat N]");
    let repeat: usize = match args.iter().position(|arg| arg == "--repeat") {
        Some(index) => args[index + 1].parse().expect("--repeat N"),
        None => 3,
    };
    let (_library, language) = load(library_path);
    let mut parser = Parser::new();
    parser.set_language(&language).expect("grammar ABI accepted by tree-sitter 0.27.0");
    let stdout = std::io::stdout();
    let mut out = BufWriter::new(stdout.lock());
    for line in std::io::stdin().lock().lines() {
        let path = line.expect("stdin");
        if path.is_empty() {
            continue;
        }
        let source = std::fs::read(&path).expect("read source");
        let mut best = u128::MAX;
        let mut tree = None;
        for _ in 0..repeat.max(1) {
            parser.reset();
            let started = Instant::now();
            let parsed = parser.parse(&source, None).expect("parse");
            best = best.min(started.elapsed().as_nanos());
            tree = Some(parsed);
        }
        let tree = tree.expect("parsed");
        let record = json!({
            "path": path,
            "bytes": source.len(),
            "parseNs": best,
            "error": tree.root_node().has_error(),
            "nodes": nodes(&tree),
        });
        serde_json::to_writer(&mut out, &record).expect("write");
        out.write_all(b"\n").expect("write");
    }
}
