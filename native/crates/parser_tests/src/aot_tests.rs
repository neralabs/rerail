//! Tier 2's Rust (`rerail_compiler::rust_stream`, the ahead-of-time backend) against the JIT on every grammar of
//! three sets: JSON's (`json_parser/`), `.rr`'s (`rr_parser/rr.bitlex`), and probes whose words edit a node the
//! writer's ops already have (`RETAGGED_ROOT`). Each grammar's Rust, in format 15 and in format 16 (decision 0434), is
//! built by rustc with the rest of its set into one library, each grammar a module, run on the JIT's harness, and its
//! tape compared with the JIT's byte for byte.

use rerail_compiler::ladder::{Compiled, Lang, compile_step, documents_with, seed};
use rerail_compiler::native_cpu_stream::Entry;
use rerail_compiler::rust_stream;

/// A set of grammars: each one's name and text, the bytes its documents' mutations write, and documents of its own.
struct Set {
    name: &'static str,
    grammars: fn() -> Vec<(String, String)>,
    alphabet: &'static [u8],
    fixed: fn() -> Vec<String>,
}

const SETS: [Set; 3] = [
    Set { name: "json", grammars: rerail_compiler::ladder::grammars, alphabet: b" \nav9,eE-{}[]\":/*\\tu", fixed: json_fixed },
    Set { name: "rr", grammars: || vec![("rr".to_owned(), include_str!("../../rr_parser/rr.bitlex").to_owned())], alphabet: b"().,:-=<>!\"`/*#_ \naZ9~;@{}\\", fixed: Vec::new },
    Set { name: "probe", grammars: || vec![("retagged-root".to_owned(), RETAGGED_ROOT.to_owned()), ("wrapped-twice".to_owned(), WRAPPED_TWICE.to_owned()), ("folded-chain".to_owned(), FOLDED_CHAIN.to_owned())], alphabet: b"();!? \nvraifus", fixed: probe_fixed },
];

/// A grammar whose words edit nodes the writer's ops already have: a `!` after the top level's lists retags the
/// root, which the ops hold open (`retag16`), after the lists before were passed at their commit points.
const RETAGGED_ROOT: &str = "lexer probe

class ws    = ' ' '\\t' '\\n' '\\r'
class alpha = 'a'..'z'
class op    = ';' '(' ')' '!'
mask code   = ~empty

token word = start starts(alpha) last lasts(alpha)
keywords word { \"vrai\" \"fausse\" } required
token punctuation = start code & op
keywords punctuation { \";\" \"(\" \")\" \"!\" } required

skip ws
error \"a byte no token takes\" when ~ws & ~alpha & ~op

tree probe {
  node document = item (';' item)* [shout]
  choice shout = retag shouted '!'
  choice item = vrai | fausse | list
  node list nested = '(' item (';' item)* ')'
}
";

/// A grammar whose words reach back past a commit point to a node an edit moved it back to: a `!` wraps the top
/// level's node made (`wrap`), which the close before committed; a second `!` wraps the wrapper, which the first's
/// close committed only after the edit moved the commit point back to it (`uncode`). Were it not moved back, the
/// wrapper would be passed to the ops before the second wrap, which then finds it passed (`fail::PASSED`).
const WRAPPED_TWICE: &str = "lexer probe

class ws    = ' ' '\\t' '\\n' '\\r'
class alpha = 'a'..'z'
class op    = ';' '(' ')' '!'
mask code   = ~empty

token word = start starts(alpha) last lasts(alpha)
keywords word { \"vrai\" \"fausse\" } required
token punctuation = start code & op
keywords punctuation { \";\" \"(\" \")\" \"!\" } required

skip ws
error \"a byte no token takes\" when ~ws & ~alpha & ~op

tree probe {
  node document = item (';' item)*
  choice item = base (wrap shouted inner '!')*
  choice base = vrai | fausse | list
  node list nested = '(' item (';' item)* ')'
}
";

/// A grammar whose words fold the root's two leaves again and again (`fold`): each fold retags the first, the one
/// leaf the folds before made, and moves its end.
const FOLDED_CHAIN: &str = "lexer probe

class ws    = ' ' '\\t' '\\n' '\\r'
class alpha = 'a'..'z'
class op    = ';' '(' ')' '!'
mask code   = ~empty

token word = start starts(alpha) last lasts(alpha)
keywords word { \"vrai\" \"fausse\" } required
token punctuation = start code & op
keywords punctuation { \";\" \"(\" \")\" \"!\" } required

skip ws
error \"a byte no token takes\" when ~ws & ~alpha & ~op

tree probe {
  node document = item item (fold 'pair' item)*
  choice item = vrai | fausse | '!'
}
";

/// The probe's documents: lists at the top level, then the root retagged, past several blocks too.
fn probe_fixed() -> Vec<String> {
    let mut docs: Vec<String> = ["(vrai);(fausse)!", "(vrai);(fausse);(vrai;(fausse))!", "vrai;(vrai)!", "(vrai)!", "(vrai);(fausse)"].map(String::from).to_vec();
    docs.push(format!("{}!", vec!["(vrai;fausse)"; 2000].join(";")));
    // `WRAPPED_TWICE`'s: a top level's node wrapped once, twice and more, after a list and a leaf.
    // `WRAPPED_TWICE`'s: a top level's node wrapped once, twice and more, after a list and a leaf.
    docs.extend(["(vrai)!!", "vrai!!", "(vrai)!", "(vrai);(fausse)!!!", "vrai;(fausse;vrai!!)!!", "((vrai)!!)!!;vrai!"].map(String::from));
    docs.push(vec!["(vrai;fausse)!!"; 2000].join(";"));
    // `FOLDED_CHAIN`'s: two leaves, then each one more folded in.
    docs.extend(["vrai fausse", "vrai fausse vrai", "vrai ! ! fausse", "! vrai !", "vrai vrai vrai vrai vrai"].map(String::from));
    docs.push(vec!["vrai !"; 2000].join(" "));
    docs
}

/// JSON's documents: its constructs, a list of many nodes, and depths, each past several blocks.
fn json_fixed() -> Vec<String> {
    let mut docs: Vec<String> = ["{\"a\": [1, 2.5e3, true, null, \"x\\n\"]}", "// c\n[1]", "[true, false, null]", "\"t\\\"x\""].map(String::from).to_vec();
    docs.push(format!("[{}]", vec!["true"; 3000].join(",")));
    docs.push(format!("[{}]", vec!["{\"a\": [1, 2.5e3, true, null, \"x\\n\"]} // c\n"; 800].join(", ")));
    docs.push(format!("{}1{}", "[".repeat(400), "]".repeat(400)));
    // Decision 0428's escape texts: a token's check that reads back (a lone low surrogate needs a high one six bytes
    // before, an escape's), after runs of backslashes, at the file's start (the frame before it) and further on.
    let escapes = [r"\uD800\uDC00", r"\uDC00", r"\uD800", r"x\uDC00", r"\\uD800\uDC00", r"\\\uD800\uDC00", r"\\\\uDC00", r"\uD83D\uDE00\uDC00", r"\n\uDC00", r"\uDBFF\uDFFF"];
    for e in escapes {
        docs.push(format!("\"{e}\""));
        docs.push(format!("[\"{e}\", \"a{e}\"]"));
        docs.push(format!("{{\"{}\": \"{e}\"}}", "k".repeat(40)));
    }
    docs
}

/// The sources built into one library, each a module with its entry `rerail_tier2_entry_{k}`; the entries, in order.
fn build(sources: &[String]) -> Vec<Entry> {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(sources, &mut h);
    // A directory a set of sources: the loader gives back the library already loaded at a path.
    let dir = std::env::temp_dir().join(format!("rerail-ladder-aot-{}-{:x}", std::process::id(), std::hash::Hasher::finish(&h)));
    std::fs::create_dir_all(&dir).expect("a directory");
    let (rs, lib) = (dir.join("ladder.rs"), dir.join("libladder.dylib"));
    let mut text = String::new();
    for (k, src) in sources.iter().enumerate() {
        text.push_str(&format!("mod s{k} {{\n{src}\n}}\n#[unsafe(no_mangle)]\npub unsafe extern \"C\" fn rerail_tier2_entry_{k}(ctx: u64) -> u64 {{\n    unsafe {{ s{k}::tier2_entry(ctx) }}\n}}\n"));
    }
    std::fs::write(&rs, text).expect("writes");
    let out = std::process::Command::new("rustc").args(["--edition", "2024", "--crate-type", "cdylib", "-C", "opt-level=1", "-o"]).arg(&lib).arg(&rs).output().expect("runs rustc");
    assert!(out.status.success(), "rustc: {}", String::from_utf8_lossy(&out.stderr).chars().take(4000).collect::<String>());
    let path = std::ffi::CString::new(lib.to_str().expect("a path")).expect("a path");
    // SAFETY: a library rustc just built from the sources above; kept loaded for the process.
    let entries = unsafe {
        let h = libc::dlopen(path.as_ptr(), libc::RTLD_NOW);
        assert!(!h.is_null(), "dlopen");
        (0..sources.len())
            .map(|k| {
                let name = std::ffi::CString::new(format!("rerail_tier2_entry_{k}")).expect("a name");
                let f = libc::dlsym(h, name.as_ptr());
                assert!(!f.is_null(), "dlsym");
                std::mem::transmute::<*mut libc::c_void, Entry>(f)
            })
            .collect()
    };
    let _ = std::fs::remove_dir_all(&dir);
    entries
}

/// A grammar's documents, and which are long (`documents`).
type Docs = (Vec<Vec<u8>>, Vec<bool>);
/// A document's tapes: format 15's image, format 16's and 16z's homes, and why either was refused.
type Tapes = (Option<Vec<u8>>, Vec<Option<Vec<u8>>>, Vec<Option<String>>);

/// A grammar's documents: drawn from it (`documents_with`), and its accepted ones joined, one after another, past
/// several of format 16's blocks (1,024 nodes each), where the grammar takes them so. With which hold 3,000 nodes or
/// more.
fn documents(c: &Compiled, name: &str, set: &Set) -> (Vec<Vec<u8>>, Vec<bool>) {
    let lang = Lang::of(&c.stmts);
    let (mut docs, _) = documents_with(&lang, seed(name), &|b| c.parser.parse(b).is_ok(), set.alphabet);
    docs.extend((set.fixed)().into_iter().map(String::into_bytes));
    let accepted: Vec<&Vec<u8>> = docs.iter().filter(|d| d.len() > 2 && c.parser.parse(d).is_ok()).collect();
    for sep in [&b"\n"[..], b";", b","] {
        // Only the documents that still parse joined to themselves: one that does not (its last token running on into
        // the next, a comment to the line's end) would refuse the whole join.
        let joins: Vec<&&Vec<u8>> = accepted.iter().filter(|d| c.parser.parse(&[&d[..], sep, &d[..]].concat()).is_ok()).collect();
        // Joined until the join's nodes pass 3,000 (a document of comments holds few for its bytes, and the join
        // merges their roots), to 4 MB: each round adds as many again as the nodes still short.
        let nodes: Vec<usize> = joins.iter().map(|d| c.parser.parse(d).map_or(0, |n| n.len())).collect();
        let (mut long, mut held, mut want) = (Vec::new(), 0, 3200);
        let mut each = joins.iter().zip(&nodes).cycle().take(if nodes.iter().any(|&n| n > 0) { usize::MAX } else { 0 });
        while long.len() <= 4 << 20 {
            let Some((d, n)) = each.next() else { break };
            if !long.is_empty() {
                long.extend_from_slice(sep);
            }
            long.extend_from_slice(d);
            held += n;
            if held > want {
                match c.parser.parse(&long).map(|n| n.len()) {
                    Ok(got) if got <= 3000 && got > 0 => want = held * 3200 / got,
                    _ => break,
                }
            }
        }
        if c.parser.parse(&long).is_ok_and(|n| n.len() > 3000) {
            docs.push(long);
            break;
        }
    }
    let long = docs.iter().map(|d| c.parser.parse(d).is_ok_and(|n| n.len() >= 3000)).collect();
    (docs, long)
}

/// Each grammar's Rust in format 16 writes the JIT's format 16 tapes (and 16z), presence and all, on every document the
/// JIT writes one for, and refuses (to the engine) every one the JIT refuses; in format 15, the JIT's image. Where the
/// Rust backend refuses a grammar, the reason is named, and the grammars it takes are counted.
#[test]
fn grammars_rust_tapes_are_the_jits() {
    use rerail_tape_format::f16::{Lang as F16Lang, Written};
    let home = |w: &Written| {
        assert!(w.presence_bytes().is_some(), "a format 16 run's presence");
        w.extent().to_vec()
    };
    let mut refused: Vec<String> = Vec::new();
    let (mut taken, mut written, mut long, mut direct) = (0, 0, 0, 0);
    // `RERAIL_AOT_SET`: one set only, by name.
    let only = std::env::var("RERAIL_AOT_SET").ok();
    // `RERAIL_AOT_GRAMMARS`: the grammars whose names hold one of these words (comma-separated), only.
    let pick = std::env::var("RERAIL_AOT_GRAMMARS").ok();
    let mut total = 0;
    for set in SETS.iter().filter(|l| only.as_deref().is_none_or(|o| o == l.name)) {
        let mut built: Vec<(String, Compiled, String, String, Docs, F16Lang)> = Vec::new();
        for (name, source) in (set.grammars)().into_iter().filter(|(n, _)| pick.as_deref().is_none_or(|p| p.split(',').filter(|w| !w.is_empty()).any(|w| n.contains(w)))) {
            total += 1;
            let c = compile_step(&name, &source);
            let (widths, tag) = c.parser.tier2.widths_tag();
            let (w16, t16) = c.parser.sixteen().expect("a format 16 tier 2").widths_tag();
            // The grammar's documents and its language, before its Rust: the language is every key a direct run writes
            // (`rust_stream::direct_entries`), then every other key the documents' tapes hold (format 15's tables,
            // joined), as the engine's language holds every key of its grammar, and the source's numbers are its
            // literals (decision 0516).
            let docs = documents(&c, &name, set);
            let mut keys: Vec<rerail_tape_format::tape::Entry> = rust_stream::direct_entries(&c.native, w16, t16).unwrap_or_default();
            for doc in &docs.0 {
                let Ok(f15) = c.parser.image(doc) else { continue };
                let (_, table) = rerail_tape_format::tape::f15_columns(&f15).expect("a format 15 tape");
                for e in table.entries() {
                    if !keys.contains(e) {
                        keys.push(e.clone());
                    }
                }
            }
            let lang = F16Lang::new(rerail_tape_format::tape::CodeTable::new(keys), [1, 2, 2], 1).expect("a language");
            match (rust_stream::rust_source_at(&c.native, widths, tag), rust_stream::rust_source_16(&c.native, w16, t16, &lang)) {
                (Ok(s15), Ok(s16)) => {
                    // `RERAIL_DUMP_TIER2`: each grammar's sources, in that directory.
                    if let Ok(dir) = std::env::var("RERAIL_DUMP_TIER2") {
                        let at = std::path::Path::new(&dir);
                        std::fs::write(at.join(format!("{}-{name}-15.rs", set.name)), &s15).expect("writes");
                        std::fs::write(at.join(format!("{}-{name}-16.rs", set.name)), &s16).expect("writes");
                    }
                    // The JIT's run passes each node at its word (`direct`) where the Rust does (`rust_stream::direct_keys`),
                    // and then calls no step on working rows.
                    let s = c.parser.sixteen().expect("a format 16 tier 2");
                    assert_eq!(s.direct, rust_stream::direct_keys(&c.native, w16, t16).is_some(), "{name}: the JIT direct where the Rust is");
                    assert!(!s.direct || s.row_calls == 0, "{name}: a direct run calls {} steps on the rows", s.row_calls);
                    // Its tape holds none of format 15's buffers, and a direct run's no working row either.
                    let [f15, rows] = s.tape(1000).held();
                    assert!(f15 == 0 && (rows == 0) == s.direct, "{name}: a format 16 run's tape holds {f15} bytes of format 15's, {rows} of rows (direct {})", s.direct);
                    direct += usize::from(s.direct);
                    built.push((name, c, s15, s16, docs, lang))
                }
                (a, b) => refused.push(format!("{} {name}: {:?}", set.name, a.err().or(b.err()))),
            }
        }
        let sources: Vec<String> = built.iter().flat_map(|(_, _, a, b, ..)| [a.clone(), b.clone()]).collect();
        let entries = build(&sources);
        for (k, (name, mut c, _, _, (docs, longs), lang)) in built.into_iter().enumerate() {
            // Each document's tapes by the JIT, then by the Rust, in the same parser: format 15's image, and format
            // 16's home and 16z's where the JIT writes format 15 (its table names the language); the numbers once, for
            // every document.
            let numbers = c.parser.sixteen().expect("a format 16 tier 2").numbers(&lang);
            let tapes = |c: &Compiled| -> Vec<Tapes> {
                docs.iter()
                    .map(|doc| {
                        let image = c.parser.image(doc).ok();
                        let runs: Vec<_> = [false, true].into_iter().map(|z| c.parser.image16(doc, &lang, &numbers, z)).collect();
                        let why = runs.iter().map(|r| r.as_ref().err().cloned()).collect();
                        (image, runs.into_iter().map(|r| r.ok().map(|w| home(&w))).collect(), why)
                    })
                    .collect()
            };
            let want = tapes(&c);
            // SAFETY: this grammar's code, from `rust_stream`, on the JIT's harness for its format (`write16` for 16).
            unsafe {
                c.parser.tier2.set_entry(entries[2 * k], 4);
                c.parser.sixteen_mut().expect("a format 16 tier 2").set_entry(entries[2 * k + 1], 4);
            }
            let got = tapes(&c);
            taken += 1;
            for (d, (doc, ((w15, w16, jwhy), (g15, g16, why)))) in docs.iter().zip(want.iter().zip(&got)).enumerate() {
                let shown = String::from_utf8_lossy(&doc[..doc.len().min(80)]).into_owned();
                assert!(g15 == w15, "{} {name}: format 15 on {shown:?}", set.name);
                // Where the JIT writes format 15, its format 16 run writes too: a failure both runs share (an edit
                // that finds its node passed, `fail::PASSED`) is no tape.
                assert!(w15.is_none() || w16.iter().all(Option::is_some), "{} {name} on {shown:?}: format 15 written, the JIT's format 16 run fails: {jwhy:?}", set.name);
                for (z, (w, g)) in w16.iter().zip(g16).enumerate() {
                    if g != w {
                        let (g, w) = (g.clone().unwrap_or_default(), w.clone().unwrap_or_default());
                        let first = (0..g.len().min(w.len())).find(|&i| g[i] != w[i]);
                        panic!("{} {name} (16z {}) on {shown:?}: the Rust's format 16 home, lengths {} and the JIT's {}, first differing at {first:?}; the Rust's {:?}", set.name, z == 1, g.len(), w.len(), why[z]);
                    }
                    if w.is_some() {
                        written += 1;
                        long += usize::from(longs[d]);
                    }
                }
            }
        }
    }
    eprintln!("{taken} grammars taken ({direct} direct), {written} format 16 homes the JIT's ({long} of documents of 3,000 nodes or more)");
    assert!(refused.is_empty() && taken == total && taken > 0 && long > 0, "the Rust backend refuses:\n{}", refused.join("\n"));
}
