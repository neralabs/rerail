//! Per-content linker inputs: the `cgn_*` rows of the dead-code plan's per-file exports (the w6
//! `*-dead-code.jsonl` files, `facts-a xexport` output), one `Summary` per (path, content), with
//! every string interned. Export ids are the compiled plan's (`facts-b gnorm --dump all`):
//! 3 `in_scope`, 7 `cgn_def`, 8 `cgn_edge`, 9 `cgn_alias`, 10 `cgn_opaque`, 11 `cgn_module`,
//! 12 `cgn_open`, 13 `cgn_reference_alias`.
use super::intern::{Interner, Sym};
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use serde_json::Value as Json;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawDef {
    pub idx: u32,
    pub name: Sym,
    pub start: u32,
    pub end: u32,
    pub flags: u32,
}

#[derive(Default, Debug)]
pub struct VRows {
    pub defs: Vec<RawDef>,
    pub edges: Vec<(u32, Sym)>,
    pub aliases: Vec<(Sym, Sym)>,
    pub opaque: Vec<Sym>,
    pub module: Vec<Sym>,
    pub open: Vec<i64>,
    /// `cgn_reference_alias` rows whose raw key holds `.<base-slot>.`
    pub base_slots: Vec<(Sym, Sym)>,
}

#[derive(Default, Debug)]
pub struct Summary {
    pub in_scope: bool,
    pub failure: bool,
    /// module, init, unpackaged
    pub v: [VRows; 3],
}

pub fn variant_index(v: &str) -> usize {
    match v {
        "module" => 0,
        "init" => 1,
        "unpackaged" => 2,
        o => panic!("variant {o}"),
    }
}

#[derive(Default)]
pub struct Summaries {
    pub by_key: FxHashMap<(Sym, Sym), u32>,
    pub all: Vec<Summary>,
}

impl Summaries {
    pub fn get(&self, path: Sym, content: Sym) -> Option<&Summary> {
        self.by_key.get(&(path, content)).map(|&i| &self.all[i as usize])
    }

    /// Heap bytes of the rows (capacities), for the inputs line of the footprint.
    pub fn bytes(&self) -> usize {
        let mut b = self.by_key.capacity() * 16 + self.all.capacity() * std::mem::size_of::<Summary>();
        for s in &self.all {
            for v in &s.v {
                b += v.defs.capacity() * 20 + v.edges.capacity() * 8 + v.aliases.capacity() * 8 + v.opaque.capacity() * 4
                    + v.module.capacity() * 4 + v.open.capacity() * 8 + v.base_slots.capacity() * 8;
            }
        }
        b
    }
}

/// One JSON line, parsed but not interned (parallel phase).
struct Parsed {
    path: String,
    hash: String,
    in_scope: bool,
    failure: bool,
    rows: [Vec<(usize, Vec<String>)>; 7],
}

fn text(v: &Json) -> String {
    match v {
        Json::String(s) => s.clone(),
        Json::Number(n) => n.to_string(),
        Json::Null => String::new(),
        o => o.to_string(),
    }
}

fn parse_line(line: &[u8]) -> Parsed {
    let j: Json = serde_json::from_slice(line).expect("export line");
    let ex = &j["exports"];
    let rows_of = |id: &str| -> Vec<(usize, Vec<String>)> {
        ex.get(id)
            .and_then(Json::as_array)
            .map(|rs| {
                rs.iter()
                    .map(|r| {
                        let r = r.as_array().unwrap();
                        // [__path, variant, ...]
                        (variant_index(r[1].as_str().unwrap()), r[2..].iter().map(text).collect())
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    Parsed {
        path: j["path"].as_str().unwrap().to_owned(),
        hash: j["hash"].as_str().unwrap().to_owned(),
        in_scope: ex.get("3").and_then(Json::as_array).is_some_and(|a| !a.is_empty()),
        failure: !j["failure"].is_null(),
        rows: [rows_of("7"), rows_of("8"), rows_of("9"), rows_of("10"), rows_of("11"), rows_of("12"), rows_of("13")],
    }
}

fn num(s: &str) -> i64 {
    // SQLite CAST(text AS INTEGER): the leading integer (the exports hold plain decimals).
    s.parse::<i64>().unwrap_or_else(|_| panic!("not an integer: {s:?}"))
}

/// Load every line of `file` (only lines whose (path, hash) `want` admits, when given).
pub fn load(file: &Path, it: &mut Interner, out: &mut Summaries, want: Option<&(dyn Fn(&str, &str) -> bool + Sync)>) -> usize {
    let data = std::fs::read(file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
    let mut lines: Vec<&[u8]> = Vec::new();
    let mut at = 0;
    for nl in memchr::memchr_iter(b'\n', &data) {
        lines.push(&data[at..nl]);
        at = nl + 1;
    }
    if at < data.len() {
        lines.push(&data[at..]);
    }
    let mut loaded = 0;
    for chunk in lines.chunks(4096) {
        let parsed: Vec<Parsed> = chunk
            .par_iter()
            .filter(|l| !l.is_empty())
            .filter(|l| match want {
                None => true,
                Some(f) => {
                    // keys are at the end of the line: …,"hash":"<64>","path":"…"}
                    let s = std::str::from_utf8(l).unwrap();
                    let h = s.rfind("\"hash\":\"").map(|i| &s[i + 8..i + 72]).unwrap();
                    let p = s.rfind("\"path\":").map(|i| &s[i + 7..s.len() - 1]).unwrap();
                    let p: String = serde_json::from_str(p).unwrap();
                    f(&p, h)
                }
            })
            .map(|l| parse_line(l))
            .collect();
        for p in parsed {
            let path = it.intern(&p.path);
            let hash = it.intern(&p.hash);
            if out.by_key.contains_key(&(path, hash)) {
                continue;
            }
            let mut s = Summary { in_scope: p.in_scope, failure: p.failure, ..Default::default() };
            let [defs, edges, aliases, opaque, module, open, refs] = p.rows;
            for (v, r) in defs {
                s.v[v].defs.push(RawDef { idx: num(&r[0]) as u32, name: it.intern(&r[1]), start: num(&r[2]) as u32, end: num(&r[3]) as u32, flags: num(&r[4]) as u32 });
            }
            for (v, r) in edges {
                s.v[v].edges.push((num(&r[0]) as u32, it.intern(&r[1])));
            }
            for (v, r) in aliases {
                s.v[v].aliases.push((it.intern(&r[0]), it.intern(&r[1])));
            }
            for (v, r) in opaque {
                s.v[v].opaque.push(it.intern(&r[0]));
            }
            for (v, r) in module {
                s.v[v].module.push(it.intern(&r[0]));
            }
            for (v, r) in open {
                s.v[v].open.push(num(&r[0]));
            }
            for (v, r) in refs {
                if r[0].contains(".<base-slot>.") {
                    s.v[v].base_slots.push((it.intern(&r[0]), it.intern(&r[1])));
                }
            }
            out.by_key.insert((path, hash), out.all.len() as u32);
            out.all.push(s);
            loaded += 1;
        }
    }
    loaded
}
