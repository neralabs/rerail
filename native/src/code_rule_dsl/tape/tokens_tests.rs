//! Named-only tapes (RER-131): `tokens::fold` and `unfold` round trips, sizes and costs.

use super::tokens::{FoldStats, TokenGrammar, fold, grammar_json, unfold};
use super::*;
use std::path::PathBuf;

fn dialect(label: &str) -> GrammarDialect {
    match label {
        "python" => GrammarDialect::Python,
        "javascript" => GrammarDialect::JavaScript,
        "typescript" => GrammarDialect::TypeScript,
        "tsx" => GrammarDialect::Tsx,
        "sql" => GrammarDialect::Sql,
        "vue" => GrammarDialect::Vue,
        "rust" => GrammarDialect::Rust,
        "php" => GrammarDialect::Php,
        other => panic!("unknown dialect {other}"),
    }
}

/// This thread's CPU time, in seconds.
fn cpu() -> f64 {
    let mut time = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut time) };
    time.tv_sec as f64 + time.tv_nsec as f64 * 1e-9
}

/// The encoder's preorder nodes and memberships for `source`, as `encode_tree_image` builds them.
fn tree_nodes(grammar: &Grammar, source: &str) -> Option<(Vec<Pending>, Vec<(u32, u16)>, bool)> {
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&grammar.lang.get_ts_language()).unwrap();
    let parsed = crate::code_rule_dsl::language::adapter::parse_input(grammar.lang, source);
    let tree = parser.parse(parsed.as_bytes(), None)?;
    let language = grammar.kinds.grammar()?;
    let (mut nodes, mut supertypes) = (Vec::new(), Vec::new());
    encode_tree(&mut nodes, &mut supertypes, tree.root_node(), language, None, &|| false).ok()?;
    Some((nodes, supertypes, tree.root_node().has_error()))
}

fn same(a: &Pending, b: &Pending) -> bool {
    a.kf == b.kf && a.parent == b.parent && a.start == b.start && a.length == b.length && a.extra == b.extra && a.error == b.error && a.missing == b.missing && a.kind_alias == b.kind_alias
}

/// The first difference between two node lists, for a failure message.
fn difference(a: &[Pending], b: &[Pending], language: &tree_sitter::Language, source: &str) -> Option<String> {
    let describe = |n: &Pending| format!("{:?} f{} parent {:?} {}..{} {:?}", language.node_kind_for_id(n.kf & 1023), n.kf >> 10, n.parent, n.start, n.start + n.length, &source[n.start as usize..(n.start + n.length) as usize]);
    if let Some(i) = (0..a.len().min(b.len())).find(|&i| !same(&a[i], &b[i])) {
        return Some(format!("node {i}: tree {} / unfolded {}", describe(&a[i]), describe(&b[i])));
    }
    (a.len() != b.len()).then(|| format!("{} nodes / {} unfolded", a.len(), b.len()))
}

#[derive(Default)]
struct Totals {
    files: usize,
    with_errors: usize,
    failures: usize,
    stats: FoldStats,
    source: usize,
    bytes: [usize; 2],
    zstd: [usize; 2],
    codes: [usize; 2],
    rec_bytes: [usize; 2],
    time_fold: f64,
    time_unfold: f64,
    time_pack: [f64; 2],
    time_restore: [f64; 2],
}

/// Folds, packs, restores and unfolds each file of `files`; asserts every round trip.
fn run(lang: GrammarDialect, files: &[PathBuf], verbose: bool) -> Totals {
    let grammar = Grammar::new(lang).unwrap();
    let language = grammar.kinds.grammar().unwrap().clone();
    let start = cpu();
    let tokens = TokenGrammar::from_json(grammar_json(lang).expect("grammar.json"), &language).unwrap();
    if verbose {
        eprintln!("{lang:?}: {} kinds, built in {:.3}s CPU; unsupported {:?}", tokens.kind_count(), cpu() - start, tokens.unsupported);
    }
    let mut totals = Totals::default();
    for path in files {
        let Ok(source) = std::fs::read_to_string(path) else { continue };
        let Some((nodes, supertypes, errors)) = tree_nodes(&grammar, &source) else { continue };
        totals.files += 1;
        totals.with_errors += usize::from(errors);
        totals.source += source.len();
        let t = cpu();
        let (folded, classes, stats) = fold(&nodes, &supertypes, &tokens, &language, &source);
        totals.time_fold += cpu() - t;
        totals.stats.add(&stats);
        let t = cpu();
        let unfolded = unfold(&folded, &classes, &tokens, &language, &source);
        totals.time_unfold += cpu() - t;
        let ok = match &unfolded {
            Ok((back, back_classes)) => {
                let mut want = supertypes.clone();
                want.sort_unstable();
                let mut got = back_classes.clone();
                got.sort_unstable();
                match difference(&nodes, back, &language, &source) {
                    None if want == got => true,
                    None => {
                        eprintln!("{}: memberships differ", path.display());
                        false
                    }
                    Some(diff) => {
                        eprintln!("{}: {diff}", path.display());
                        false
                    }
                }
            }
            Err(error) => {
                eprintln!("{}: unfold failed: {error}", path.display());
                false
            }
        };
        if !ok {
            totals.failures += 1;
            continue;
        }
        for (side, (list, classes)) in [(nodes, supertypes), (folded, classes)].into_iter().enumerate() {
            let t = cpu();
            let image = TapeImage::finish(list, classes, &grammar, &source, &|| false).unwrap();
            totals.time_pack[side] += cpu() - t;
            let bytes = if side == 1 {
                // As v12 stores it.
                let mut stored = image.clone();
                stored.widths[0] |= NAMED_ONLY;
                stored.to_bytes().unwrap()
            } else {
                image.to_bytes().unwrap()
            };
            // RERAIL_DUMP_TAPES=<dir>: each file's full (0) and named-only (1) image bytes, for
            // compression studies; files are numbered in list order.
            if let Some(dir) = std::env::var_os("RERAIL_DUMP_TAPES") {
                let dir = PathBuf::from(dir).join(format!("{lang:?}").to_lowercase()).join(["full", "named"][side]);
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(dir.join(format!("{:05}.tape", totals.files)), &bytes).unwrap();
            }
            totals.bytes[side] += bytes.len();
            totals.zstd[side] += zstd::encode_all(&bytes[..], 3).unwrap().len();
            totals.codes[side] += image.codes.len();
            let source = Arc::new(source.clone());
            let lines = Arc::new(LineTable::new(&source, Terminators::of(lang)));
            let t = cpu();
            let doc = TapeDoc::restore_tree(Arc::clone(&source), Arc::clone(&grammar), &image, lines, &|| false).unwrap();
            totals.time_restore[side] += cpu() - t;
            totals.rec_bytes[side] += doc.data.len() * std::mem::size_of::<Rec>() + doc.supertypes.len() * 8;
        }
    }
    totals
}

fn report(label: &str, t: &Totals) {
    let s = &t.stats;
    let pct = |a: usize| 100.0 * a as f64 / s.tokens.max(1) as f64;
    eprintln!(
        "{label}: {} files ({} with errors, {} round-trip failures), {:.1} MB source\n  \
         nodes {} = named {} + tokens {} ({:.1}%); tokens implied {} ({:.1}%), kept: not default {} ({:.1}%), unrecoverable {} ({:.1}%), node flagged {} ({:.1}%) in {} nodes\n  \
         kept nodes {} = {:.3}x; bytes {:.2} -> {:.2} MB ({:.3}x); zstd {:.2} -> {:.2} MB ({:.3}x); restored records {:.1} -> {:.1} MB ({:.3}x)\n  \
         CPU s: fold {:.3} unfold {:.3}; pack {:.3} -> {:.3}; restore {:.3} -> {:.3}",
        t.files, t.with_errors, t.failures, t.source as f64 / 1e6,
        s.nodes, s.named, s.tokens, 100.0 * s.tokens as f64 / s.nodes.max(1) as f64,
        s.implied, pct(s.implied), s.kept_not_default, pct(s.kept_not_default), s.kept_unrecoverable, pct(s.kept_unrecoverable), s.kept_flagged, pct(s.kept_flagged), s.flagged_nodes,
        s.nodes - s.implied, (s.nodes - s.implied) as f64 / s.nodes.max(1) as f64,
        t.bytes[0] as f64 / 1e6, t.bytes[1] as f64 / 1e6, t.bytes[1] as f64 / t.bytes[0].max(1) as f64,
        t.zstd[0] as f64 / 1e6, t.zstd[1] as f64 / 1e6, t.zstd[1] as f64 / t.zstd[0].max(1) as f64,
        t.rec_bytes[0] as f64 / 1e6, t.rec_bytes[1] as f64 / 1e6, t.rec_bytes[1] as f64 / t.rec_bytes[0].max(1) as f64,
        t.time_fold, t.time_unfold, t.time_pack[0], t.time_pack[1], t.time_restore[0], t.time_restore[1],
    );
    let mut flagged: Vec<_> = s.flagged_kinds.iter().collect();
    flagged.sort_by(|a, b| b.1.cmp(a.1));
    eprintln!("  flagged kinds: {:?}", flagged.iter().take(12).collect::<Vec<_>>());
    let mut gaps: Vec<_> = s.not_default.iter().collect();
    gaps.sort_by(|a, b| b.1.cmp(a.1));
    eprintln!("  kept, not default: {:?}", gaps.iter().take(25).collect::<Vec<_>>());
}

fn snippet(lang: GrammarDialect, name: &str, source: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rerail-named-only-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.{lang:?}"));
    std::fs::write(&path, source).unwrap();
    path
}

#[test]
fn named_only_tapes_round_trip_on_samples() {
    let samples = [
        (GrammarDialect::Python, "async def f(a, b=1, *c, **d) -> int:\n    return (a + b) if a is not b else [x for x in c if x not in d]\n\nfrom .m import (x, y,)\nt = (1,)\nimport a.b as c\nclass C(B, metaclass=M):\n    x: int = 3  # note\n    def g(self): ...\n"),
        (GrammarDialect::TypeScript, "export const f = async <T,>(a: T, b?: number): Promise<void> => { let x = [1,,2]; for (const k of a as any) { if (k?.y ?? 0) continue; } };\ninterface I { a: string; b(): void, }\nclass K { static readonly z = 1; get y() { return 1 } }\n"),
        (GrammarDialect::JavaScript, "const a = [1,,3]\nfunction f(x, y,) { return x ? y : `t${x}` }\nlet o = {a, b: 2, ...c,};\n"),
        (GrammarDialect::Rust, "pub fn f<'a, T: Clone>(x: &'a [T]) -> Option<&'a T> { let b = b\"x\"; if let Some(y) = x.first() { return Some(y); } macro_rules! m { ($a:expr) => { $a + 1 }; } None }\n"),
        (GrammarDialect::Php, "<?php\nnamespace A;\nFUNCTION f($a, $b = [1, 2,]) { return $a ?? $b; }\nclass C extends D implements E { public function g(): ?int { return null; } }\n"),
        (GrammarDialect::Sql, "SELECT a, b AS c FROM t WHERE x IN (1, 2) AND y IS NOT NULL ORDER BY a DESC;\nCREATE TABLE t (a int[] NOT NULL, b text);\n"),
    ];
    for (lang, source) in samples {
        let path = snippet(lang, "sample", source);
        let totals = run(lang, &[path], false);
        report(&format!("{lang:?}"), &totals);
        assert_eq!(totals.failures, 0, "{lang:?}: a round trip failed");
        assert!(totals.stats.implied > 0, "{lang:?}: nothing implied");
    }
}

/// `RERAIL_NAMED_ONLY='<dialect>=<list file>[:step];...'`: each list file names one source path
/// per line, every `step`-th read. One report per dialect.
#[test]
#[ignore = "corpus measurement: RERAIL_NAMED_ONLY='python=list.txt:5;...', --ignored --nocapture"]
fn named_only_tapes_on_corpora() {
    let spec = std::env::var("RERAIL_NAMED_ONLY").expect("RERAIL_NAMED_ONLY");
    let mut failures = 0;
    for entry in spec.split(';').filter(|e| !e.trim().is_empty()) {
        let (label, list) = entry.split_once('=').expect("<dialect>=<list>");
        let (list, step) = list.split_once(':').map_or((list, 1), |(l, s)| (l, s.parse().unwrap()));
        let files: Vec<PathBuf> = std::fs::read_to_string(list).unwrap().lines().enumerate().filter(|(i, _)| i % step == 0).map(|(_, l)| PathBuf::from(l)).collect();
        let totals = run(dialect(label.trim()), &files, true);
        report(label.trim(), &totals);
        failures += totals.failures;
    }
    assert_eq!(failures, 0, "round trips failed");
}

/// The Rust and PHP grammars the automata are built from are the crates' own, byte for byte:
/// a crate upgrade without refreshing `grammars/token-grammars` fails here (where the crate's
/// sources are on disk).
#[test]
fn token_grammars_match_the_parser_crates() {
    let home = std::env::var("CARGO_HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap()).join(".cargo"));
    let Ok(registries) = std::fs::read_dir(home.join("registry/src")) else { return };
    let registries: Vec<PathBuf> = registries.filter_map(|entry| Some(entry.ok()?.path())).collect();
    let cargo = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../Cargo.lock")).unwrap();
    for (lang, krate, inner) in [(GrammarDialect::Rust, "tree-sitter-rust", "src"), (GrammarDialect::Php, "tree-sitter-php", "php/src")] {
        let version = cargo.split("[[package]]").find_map(|package| {
            package.contains(&format!("name = \"{krate}\"")).then(|| package.lines().find_map(|line| line.strip_prefix("version = \"")?.strip_suffix('"').map(str::to_owned)))?
        });
        let Some(version) = version else { continue };
        let Some(path) = registries.iter().map(|registry| registry.join(format!("{krate}-{version}")).join(inner).join("grammar.json")).find(|path| path.exists()) else { continue };
        assert_eq!(grammar_json(lang).unwrap(), std::fs::read_to_string(&path).unwrap(), "{krate} {version}: refresh grammars/token-grammars");
    }
}
