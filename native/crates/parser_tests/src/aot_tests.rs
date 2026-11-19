//! Tier 2's Rust (`rerail_compiler::rust_stream`, the ahead-of-time backend) against the JIT on every step of two
//! Bitlex ladders, JSON's (`compiler/bitlex_ladder`) and `.rr`'s (`rr_syntax/bitlex_ladder`), and a probe whose words
//! edit a node the writer's ops already have (`RETAGGED_ROOT`). Kotlin's ladder is not here: the Rust backend refuses
//! its grammar ("more than one kind of extra"). Each step's Rust, in
//! format 15 and in format 16 (decision 0434), is built by rustc with the rest of its ladder into one library, each
//! step a module, run on the JIT's harness, and its tape compared with the JIT's byte for byte.

use rerail_compiler::ladder::{Compiled, Lang, compile_step, documents_with, seed};
use rerail_compiler::native_cpu_stream::Entry;
use rerail_compiler::rust_stream;

/// A ladder: its steps, the bytes its documents' mutations write, and documents of its own.
struct Ladder {
    name: &'static str,
    steps: fn() -> Vec<(String, String)>,
    alphabet: &'static [u8],
    fixed: fn() -> Vec<String>,
}

const LADDERS: [Ladder; 3] = [
    Ladder { name: "json", steps: || steps(concat!(env!("CARGO_MANIFEST_DIR"), "/../compiler/bitlex_ladder")), alphabet: b"<>();=#^ \nav9~,eE-{}[]\":/*", fixed: json_fixed },
    Ladder { name: "rr", steps: || steps(concat!(env!("CARGO_MANIFEST_DIR"), "/../rr_syntax/bitlex_ladder")), alphabet: b"().,:-=<>!\"`/*#_ \naZ9~;@{}\\", fixed: Vec::new },
    Ladder { name: "probe", steps: || vec![("retagged-root".to_owned(), RETAGGED_ROOT.to_owned())], alphabet: b"();!? \nvraifus", fixed: probe_fixed },
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

/// The probe's documents: lists at the top level, then the root retagged, past several blocks too.
fn probe_fixed() -> Vec<String> {
    let mut docs: Vec<String> = ["(vrai);(fausse)!", "(vrai);(fausse);(vrai;(fausse))!", "vrai;(vrai)!", "(vrai)!", "(vrai);(fausse)"].map(String::from).to_vec();
    docs.push(format!("{}!", vec!["(vrai;fausse)"; 2000].join(";")));
    docs
}

/// The JSON ladder's documents of `tier2_tests` (the compiler's format 16 run against `f16::write_from`): its
/// constructs, a top level of many nodes, and depths, each past several blocks.
fn json_fixed() -> Vec<String> {
    let mut docs: Vec<String> = [
        "vrai",
        "vrai fausse",
        "(vrai;fausse;nul)",
        "((vrai);())",
        "<#a# = vrai; #b# = (1;2)>",
        "#t^#x^^#",
        "~3,14e~2",
        "-- note\nvrai",
        "{- note -} vrai",
        "{\"a\": [1, 2.5e3, true, null, \"x\\n\"]}",
        "// c\n[1]",
    ]
    .map(String::from)
    .to_vec();
    docs.push(format!("({})", vec!["vrai"; 3000].join(";")));
    docs.push(vec!["vrai"; 3000].join(" "));
    docs.push(format!("{}vrai{}", "(".repeat(300), ")".repeat(300)));
    docs.push(format!("<{}>", (0..1500).map(|k| format!("#k{k}# = (1;~2,5e3;#t^#x#)")).collect::<Vec<_>>().join("; ")));
    docs.push(format!("[{}]", vec!["{\"a\": [1, 2.5e3, true, null, \"x\\n\"]} // c\n"; 800].join(", ")));
    docs.push(format!("{}1{}", "[".repeat(400), "]".repeat(400)));
    docs
}

/// A ladder's steps: each file's name and text, in order.
fn steps(dir: &str) -> Vec<(String, String)> {
    let mut paths: Vec<_> = std::fs::read_dir(dir).expect("the ladder").map(|e| e.expect("an entry").path()).filter(|p| p.extension().is_some_and(|x| x == "bitlex")).collect();
    paths.sort();
    paths.iter().map(|p| (p.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_owned(), std::fs::read_to_string(p).expect("a step"))).collect()
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

/// A step's documents: the ladder's (`documents_with`), and its accepted ones joined, one after another, past
/// several of format 16's blocks (1,024 nodes each), where the grammar takes them so. With which hold 3,000 nodes or
/// more.
fn documents(c: &Compiled, name: &str, ladder: &Ladder) -> (Vec<Vec<u8>>, Vec<bool>) {
    let lang = Lang::of(&c.stmts);
    let (mut docs, _) = documents_with(&lang, seed(name), &|b| c.parser.parse(b).is_ok(), ladder.alphabet);
    docs.extend((ladder.fixed)().into_iter().map(String::into_bytes));
    let accepted: Vec<&Vec<u8>> = docs.iter().filter(|d| d.len() > 2 && c.parser.parse(d).is_ok()).collect();
    for sep in [&b"\n"[..], b";", b","] {
        let mut long = Vec::new();
        for d in accepted.iter().cycle().take(accepted.len().max(1) * 400) {
            if !long.is_empty() {
                long.extend_from_slice(sep);
            }
            long.extend_from_slice(d);
            if long.len() > 100_000 {
                break;
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

/// Each step's Rust in format 16 writes the JIT's format 16 tapes (and 16z), presence and all, on every document the
/// JIT writes one for, and refuses (to the engine) every one the JIT refuses; in format 15, the JIT's image. Where the
/// Rust backend refuses a step, the reason is named, and the steps it takes are counted.
#[test]
fn ladder_steps_rust_tapes_are_the_jits() {
    use rerail_tape_format::f16::{Lang as F16Lang, Written};
    let home = |w: &Written| {
        assert!(w.presence_bytes().is_some(), "a format 16 run's presence");
        w.extent().to_vec()
    };
    let mut refused: Vec<String> = Vec::new();
    let (mut taken, mut written, mut long) = (0, 0, 0);
    // `RERAIL_AOT_LADDER`: one ladder only, by name.
    let only = std::env::var("RERAIL_AOT_LADDER").ok();
    // `RERAIL_AOT_STEPS`: the steps whose names hold one of these words (comma-separated), only.
    let pick = std::env::var("RERAIL_AOT_STEPS").ok();
    for ladder in LADDERS.iter().filter(|l| only.as_deref().is_none_or(|o| o == l.name)) {
        let mut built: Vec<(String, Compiled, String, String)> = Vec::new();
        for (name, source) in (ladder.steps)().into_iter().filter(|(n, _)| pick.as_deref().is_none_or(|p| p.split(',').any(|w| n.contains(w)))) {
            let t0 = std::time::Instant::now();
            let c = compile_step(&name, &source);
            eprintln!("{} {name}: compiled in {:?}", ladder.name, t0.elapsed());
            let (widths, tag) = c.parser.tier2.widths_tag();
            let (w16, t16) = c.parser.sixteen().expect("a format 16 tier 2").widths_tag();
            match (rust_stream::rust_source_at(&c.native, widths, tag), rust_stream::rust_source_16(&c.native, w16, t16)) {
                (Ok(s15), Ok(s16)) => {
                    // `RERAIL_DUMP_TIER2`: each step's sources, in that directory.
                    if let Ok(dir) = std::env::var("RERAIL_DUMP_TIER2") {
                        let at = std::path::Path::new(&dir);
                        std::fs::write(at.join(format!("{}-{name}-15.rs", ladder.name)), &s15).expect("writes");
                        std::fs::write(at.join(format!("{}-{name}-16.rs", ladder.name)), &s16).expect("writes");
                    }
                    built.push((name, c, s15, s16))
                }
                (a, b) => refused.push(format!("{} {name}: {:?}", ladder.name, a.err().or(b.err()))),
            }
        }
        let sources: Vec<String> = built.iter().flat_map(|(_, _, a, b)| [a.clone(), b.clone()]).collect();
        let entries = build(&sources);
        for (k, (name, mut c, _, _)) in built.into_iter().enumerate() {
            // Each document's tapes by the JIT, then by the Rust, in the same parser: format 15's image, and format
            // 16's home and 16z's where the JIT writes format 15 (its table names the language).
            let (docs, longs) = documents(&c, &name, ladder);
            // The step's language: every key its documents' tapes hold (format 15's tables, joined), as the engine's
            // language holds every key of its grammar; the numbers once, for every document.
            let mut keys: Vec<rerail_tape_format::tape::Entry> = Vec::new();
            for doc in &docs {
                let Ok(f15) = c.parser.image(doc) else { continue };
                let (_, table) = rerail_tape_format::tape::f15_columns(&f15).expect("a format 15 tape");
                for e in table.entries() {
                    if !keys.contains(e) {
                        keys.push(e.clone());
                    }
                }
            }
            let lang = F16Lang::new(rerail_tape_format::tape::CodeTable::new(keys), [1, 2, 2], 1).expect("a language");
            let numbers = c.parser.sixteen().expect("a format 16 tier 2").numbers(&lang);
            let tapes = |c: &Compiled| -> Vec<(Option<Vec<u8>>, Vec<Option<Vec<u8>>>, Vec<Option<String>>)> {
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
            // SAFETY: this step's code, from `rust_stream`, on the JIT's harness for its format (`write16` for 16).
            unsafe {
                c.parser.tier2.set_entry(entries[2 * k], 4);
                c.parser.sixteen_mut().expect("a format 16 tier 2").set_entry(entries[2 * k + 1], 4);
            }
            let got = tapes(&c);
            taken += 1;
            for (d, (doc, ((w15, w16, _), (g15, g16, why)))) in docs.iter().zip(want.iter().zip(&got)).enumerate() {
                let shown = String::from_utf8_lossy(&doc[..doc.len().min(80)]).into_owned();
                assert!(g15 == w15, "{} {name}: format 15 on {shown:?}", ladder.name);
                for (z, (w, g)) in w16.iter().zip(g16).enumerate() {
                    if g != w {
                        let (g, w) = (g.clone().unwrap_or_default(), w.clone().unwrap_or_default());
                        let first = (0..g.len().min(w.len())).find(|&i| g[i] != w[i]);
                        panic!("{} {name} (16z {}) on {shown:?}: the Rust's format 16 home, lengths {} and the JIT's {}, first differing at {first:?}; the Rust's {:?}", ladder.name, z == 1, g.len(), w.len(), why[z]);
                    }
                    if w.is_some() {
                        written += 1;
                        long += usize::from(longs[d]);
                    }
                }
            }
        }
    }
    eprintln!("{taken} steps taken, {written} format 16 homes the JIT's ({long} of documents of 3,000 nodes or more); refused:\n{}", refused.join("\n"));
    assert!(taken >= if only.is_some() || pick.is_some() { 1 } else { 30 } && long > 0, "the steps the Rust backend takes");
}
