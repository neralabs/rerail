//! Decision 0225 §2: a file the memo holds, under a program one of whose file-local rules was edited, is a partial
//! hit. Its chunk derives again only the relations the edit changed (the cone), and replays the rest from its
//! entry. What it hands on must be what a full evaluation hands on: the outputs, the exports, and the base rows the
//! cross-file part reads (a semi-join's among them, kept for the nodes the exports name).

use crate::code_rule_dsl::typing::file_local::Binding;
use std::collections::BTreeSet;
use std::path::PathBuf;

/// `files` (path, source) under a fresh directory named for `name`, each led by `marker` (a comment), so a run over
/// them reads nothing another test's run left in the memo.
fn corpus(name: &str, marker: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension(format!("partial-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (path, source) in files {
        std::fs::write(dir.join(path), format!("# {marker}\n{source}")).unwrap();
    }
    dir
}

/// `program`'s rows of `outputs` over `dir`, each relation's sorted, with the run's report.
fn run(dir: &std::path::Path, program: &str, outputs: &[&str]) -> (super::Report, Vec<BTreeSet<Vec<String>>>) {
    let out = dir.join(".out");
    let _ = std::fs::remove_dir_all(&out);
    let report = super::run(program, dir, None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
    let rows = outputs.iter().map(|rel| crate::code_rule_dsl::logic::plan::output::shown(&out, rel).unwrap_or_default().into_iter().collect()).collect();
    (report, rows)
}

const FILES: &[(&str, &str)] = &[
    ("a.py", "def helper(x):\n    assert x\n    return x\n\ndef other(y):\n    while y:\n        assert y\n    return helper(y)\n"),
    ("b.py", "from a import helper, other\n\ndef main():\n    assert True\n    helper(1)\n    other(2)\n    if main:\n        assert main\n"),
    ("c.py", "def lone():\n    try:\n        assert lone\n    except Exception:\n        pass\n    return other(3)\n\ndef helper(z):\n    return z\n"),
];

/// A cross-file rule (`at`) reads `col` of the nodes an export (`called`) names, joined with another file's export
/// (`defined`); a file-local rule (`flagged`) is the one edited, its body testing `kind`. `both` and `doubled` are
/// read by a later stratum (`many` reads `doubled`), so a cold run hands `both` off once `doubled` has read it, which
/// its hand-off asserts (Q172; the counting supports, gone with decision 0364, were the relations it handed before).
fn program(kind: &str) -> String {
    format!(
        r#"
.decl called(c: node, x: symbol)
called(c, x) :- kind(c, "call"), field(c, "function", f), text(f, x).
.decl defined(d: node, x: symbol)
defined(d, x) :- kind(d, "function_definition"), field(d, "name", n), text(n, x).
.decl at(x: symbol, k: number)
at(x, k) :- called(c, x), defined(_, x), col(c, k).
.decl both(d: node, e: node, x: symbol)
both(d, e, x) :- defined(d, x), defined(e, x), d != e.
.decl doubled(x: symbol)
doubled(x) :- both(_, _, x).
.decl many(x: symbol)
many(x) :- doubled(x), at(x, _).
.decl flagged(n: node)
flagged(n) :- kind(n, "assert_statement"), !kind(n, "{kind}").
.decl flag_col(k: number, l: number)
flag_col(k, l) :- flagged(n), col(n, k), line(n, l).
.output at, flag_col, many
"#
    )
}

const OUTPUTS: &[&str] = &["at", "flag_col", "many"];

/// An edit of `flagged`'s body: the second run reads every file as a partial hit, and gives the rows a run with no
/// memo gives, its cross-file part seeing no input changed (no export nor base relation the edit did not touch).
#[test]
fn an_edited_rule_body_reads_partial_hits_as_a_full_run_would() {
    let marker = format!("{:?}", std::time::SystemTime::now());
    let dir = corpus("edit", &marker, FILES);
    let (first, before) = run(&dir, &program("try_statement"), OUTPUTS);
    assert_eq!(first.memo_partial, 0, "a first run has no entries to read");
    assert!(!before[0].is_empty(), "the cross-file rule finds calls of defined functions: {before:?}");
    let (edited, after) = run(&dir, &program("while_statement"), OUTPUTS);
    assert_eq!(edited.memo_partial, FILES.len(), "every file is a partial hit");
    let fresh = corpus("fresh", &format!("{marker} fresh"), FILES);
    let (_, expected) = run(&fresh, &program("while_statement"), OUTPUTS);
    assert_eq!(after, expected, "the partial hits' rows are a full run's");
    assert!(edited.cross_kept.is_some(), "the cross-file part is kept between runs, so its inputs are compared");
    assert!(edited.cross_changed.is_empty(), "the cross-file part's inputs are as the last run's: {:?} changed", edited.cross_changed);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&fresh);
}

/// Q20: an edit of one file reads the others for their outputs alone (decision 0117 step 1), their export and
/// base rows the kept inputs'. Each such hit decodes none of those rows: since memo entries are images (decision
/// 0240 §4) a hit was decoded whole, and its rows were then taken for the changed files' and hashed again.
#[test]
fn a_hit_read_for_its_outputs_alone_decodes_no_export_or_base_rows() {
    let marker = format!("{:?}", std::time::SystemTime::now());
    let dir = corpus("outputs-alone", &marker, FILES);
    let program = program("try_statement");
    // Past the stat index's racy window (git's rule), so every run finds the unchanged files by their stat.
    std::thread::sleep(std::time::Duration::from_nanos(crate::content_store::RACY_NS as u64 + 200_000_000));
    let (cold, _) = run(&dir, &program, OUTPUTS);
    assert_eq!(cold.stats_kept, Some(Ok((3, 0))), "a cold run counts every file in");
    assert_unchanged(&run(&dir, &program, OUTPUTS).0, "a rerun with nothing changed");
    let mut kept = 0;
    let mut edits = Vec::new();
    for (k, tail) in ["\nhelper(4)\n", "\nhelper(5)\n", "\nhelper(6)\n"].into_iter().enumerate() {
        std::fs::write(dir.join("c.py"), format!("# {marker}\n{}{tail}", FILES[2].1)).unwrap();
        let (edited, rows) = run(&dir, &program, OUTPUTS);
        assert_steady(&edited, &format!("edit {k}"));
        kept += edited.memo_kept.0;
        edits.push((tail, rows));
    }
    // Each edit's rows are a full run's, each over a corpus of its own.
    for (k, (tail, rows)) in edits.into_iter().enumerate() {
        let c = format!("{}{tail}", FILES[2].1);
        let fresh = corpus(&format!("outputs-alone-fresh-{k}"), &format!("{marker} fresh {k}"), &[FILES[0], FILES[1], ("c.py", &c)]);
        let (_, expected) = run(&fresh, &program, OUTPUTS);
        let _ = std::fs::remove_dir_all(&fresh);
        assert_eq!(rows, expected, "edit {k}: the rows of a full run");
    }
    assert_eq!(kept, 3 * 2, "each edit reads both unchanged files for their outputs alone");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A steady edit of one of `FILES` (c.py): the other two are memo hits read for their outputs alone, decoding none
/// of their export or base rows, and the statistics keeper counts the edited file out and in, and no other.
fn assert_steady(edited: &super::Report, what: &str) {
    assert!(edited.cross_kept.is_some_and(|(run, all)| run < all), "{what}: the cross-file part kept, some strata not run: {:?}", edited.cross_kept);
    assert_eq!(edited.cross_handed, 0, "{what}: a run driven by deltas hands nothing off");
    assert_eq!(edited.cross_unloaded, 0, "{what}: nor lets an input go: a session keeps its inputs for the next run");
    assert_eq!(edited.memo.0, 2, "{what}: the unchanged files are memo hits");
    assert_eq!(edited.memo_kept.0, 2, "{what}: the hits are read for their outputs alone (decision 0117 step 1)");
    assert_eq!(edited.memo_kept.1, 0, "{what}: {} hits read for their outputs alone decoded export or base rows", edited.memo_kept.0);
    assert_eq!(edited.stats_kept, Some(Ok((1, 1))), "{what}: only the edited file is counted in and out (decision 0096 §3)");
}

/// A run with nothing changed since the last (no whole-run cutoff asked, `options.reuse`): every file a memo hit read
/// for its outputs alone, none of their export or base rows decoded, and no file counted in or out.
fn assert_unchanged(report: &super::Report, what: &str) {
    assert_eq!(report.memo.0, FILES.len(), "{what}: every file is a memo hit");
    assert_eq!(report.memo_kept, (FILES.len(), 0), "{what}: every hit read for its outputs alone, no row decoded");
    assert_eq!(report.stats_kept, Some(Ok((0, 0))), "{what}: no file counted in or out");
}

/// A new engine build over the store an older build left (decision 0096 §3, Q71). Its first edit finds none of the
/// old build's entries, so it evaluates every file; and it counts every file in afresh, since what the old build's
/// keeper counted is of entries this build's plan does not hold (it could neither count the edited file out nor
/// trust the others' counts). Its next edit is a steady one. Before, that first edit kept the old build's statistics
/// and gave them up at the edited file, and the next edit replayed every hit whole to count every file in.
#[test]
fn a_new_builds_first_edit_counts_every_file_in_so_its_next_edit_is_steady() {
    let marker = format!("{:?}", std::time::SystemTime::now());
    let dir = corpus("new-build", &marker, FILES);
    let program = program("try_statement");
    std::thread::sleep(std::time::Duration::from_nanos(crate::content_store::RACY_NS as u64 + 200_000_000));
    let edit = |tail: &str| {
        std::fs::write(dir.join("c.py"), format!("# {marker}\n{}{tail}", FILES[2].1)).unwrap();
        run(&dir, &program, OUTPUTS)
    };
    let (cold, _) = run(&dir, &program, OUTPUTS);
    assert_eq!(cold.stats_kept, Some(Ok((3, 0))), "a cold run counts every file in");
    assert!(cold.cross_handed > 0, "a cold run hands its cross-file relations off as it goes, not all held to its end: {} handed; strata {:?}", cold.cross_handed, cold.cross_kept);
    assert_steady(&edit("\nhelper(4)\n").0, "an edit of the first build");
    crate::code_rule_dsl::build_ids::rebuild_for_tests();
    let (first, _) = edit("\nhelper(5)\n");
    assert_eq!(first.memo.0, 0, "no entry of the old build's is a hit");
    assert_eq!(first.stats_kept, Some(Ok((3, 0))), "the new build's first edit counts every file in");
    // Nor is anything of the old build's cross-file part read (decision 0077 §4: kept per engine build): no
    // stratum's rows (their identities are of the build), and no input's rows, which it read to make deltas with
    // nothing to apply them to. So every input is new to it, `defined` too, which the edit leaves as it was.
    assert!(first.cross_kept.is_none_or(|(run, all)| run == all), "the new build derives every cross-file stratum: {:?}", first.cross_kept);
    assert!(first.cross_changed.iter().any(|r| r == "defined"), "no input compared with the old build's rows: {:?} changed", first.cross_changed);
    assert!(first.cross_handed > 0, "its cross-file relations handed off as it goes");
    assert!(first.cross_unloaded > 0, "its cross-file inputs let go after their last reader");
    let tail = "\nhelper(6)\n";
    let (next, rows) = edit(tail);
    assert_steady(&next, "the new build's second edit");
    assert_unchanged(&run(&dir, &program, OUTPUTS).0, "the new build's rerun with nothing changed");
    let c = format!("{}{tail}", FILES[2].1);
    let fresh = corpus("new-build-fresh", &format!("{marker} fresh"), &[FILES[0], FILES[1], ("c.py", &c)]);
    let (_, expected) = run(&fresh, &program, OUTPUTS);
    assert_eq!(rows, expected, "the rows of a full run");
    let _ = std::fs::remove_dir_all(&fresh);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A run stopped after it wrote its first entries (killed, out of memory, a strict abort) leaves the statistics as the
/// last finished run kept them, valid: it ran the same program, whose entries it wrote are of the contents it read,
/// and the contents the keeper counted are still in the memo. The next edit is a steady one. Before, the stopped run's
/// mark ("a run is under way", of no program) emptied the keeper, and the next edit counted every file in, decoding
/// every hit whole.
#[test]
fn an_edit_after_a_stopped_run_of_the_same_program_is_steady() {
    let marker = format!("{:?}", std::time::SystemTime::now());
    let dir = corpus("stopped", &marker, FILES);
    let program = program("try_statement");
    std::thread::sleep(std::time::Duration::from_nanos(crate::content_store::RACY_NS as u64 + 200_000_000));
    let edit = |tail: &str| std::fs::write(dir.join("c.py"), format!("# {marker}\n{}{tail}", FILES[2].1)).unwrap();
    let (cold, _) = run(&dir, &program, OUTPUTS);
    assert_eq!(cold.stats_kept, Some(Ok((3, 0))), "a cold run counts every file in");
    edit("\nhelper(4)\n");
    super::STOP_AFTER_ENTRIES.with(|s| s.set(true));
    let out = dir.join(".out");
    let stopped = super::run(&program, &dir, None, &out, "program", Binding::Cpu, 1 << 18);
    assert!(stopped.is_err_and(|e| e.to_string().contains("stopped after its first entries")), "the run stops where the hook stops it");
    assert!(!super::STOP_AFTER_ENTRIES.with(std::cell::Cell::get), "the hook was reached");
    edit("\nhelper(5)\n");
    let (next, rows) = run(&dir, &program, OUTPUTS);
    assert_steady(&next, "the edit after a stopped run");
    let c = format!("{}\nhelper(5)\n", FILES[2].1);
    let fresh = corpus("stopped-fresh", &format!("{marker} fresh"), &[FILES[0], FILES[1], ("c.py", &c)]);
    let (_, expected) = run(&fresh, &program, OUTPUTS);
    assert_eq!(rows, expected, "the rows of a full run");
    let _ = std::fs::remove_dir_all(&fresh);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The other side of the last test: a stopped run of another program (a rule edited) rewrote some of the entries
/// the statistics count (its partial hits, decision 0225 §2), so the next run of the first program trusts none of
/// what was kept: it counts every file in, and its rows are a full run's.
#[test]
fn a_stopped_run_of_another_program_leaves_nothing_kept_trusted() {
    let marker = format!("{:?}", std::time::SystemTime::now());
    let dir = corpus("stopped-other", &marker, FILES);
    let (first, edited) = (program("try_statement"), program("while_statement"));
    std::thread::sleep(std::time::Duration::from_nanos(crate::content_store::RACY_NS as u64 + 200_000_000));
    let (cold, _) = run(&dir, &first, OUTPUTS);
    assert_eq!(cold.stats_kept, Some(Ok((3, 0))), "a cold run counts every file in");
    super::STOP_AFTER_ENTRIES.with(|s| s.set(true));
    let stopped = super::run(&edited, &dir, None, &dir.join(".out"), "program", Binding::Cpu, 1 << 18);
    assert!(stopped.is_err_and(|e| e.to_string().contains("stopped after its first entries")), "the edited program's run stops where the hook stops it");
    let tail = "\nhelper(4)\n";
    std::fs::write(dir.join("c.py"), format!("# {marker}\n{}{tail}", FILES[2].1)).unwrap();
    let (next, rows) = run(&dir, &first, OUTPUTS);
    assert_eq!(next.memo_kept.0, 0, "no hit is read for its outputs alone: what was kept is not trusted");
    assert_eq!(next.stats_kept, Some(Ok((3, 0))), "every file counted in again");
    let c = format!("{}{tail}", FILES[2].1);
    let fresh = corpus("stopped-other-fresh", &format!("{marker} fresh"), &[FILES[0], FILES[1], ("c.py", &c)]);
    let (_, expected) = run(&fresh, &first, OUTPUTS);
    assert_eq!(rows, expected, "the rows of a full run");
    let _ = std::fs::remove_dir_all(&fresh);
    let _ = std::fs::remove_dir_all(&dir);
}

const COMMENTED: &[(&str, &str)] = &[
    ("a.py", "def helper(x):\n    assert x\n    return x\n\ndef other(y):\n    while y:\n        assert y\n    return y\n"),
    ("b.py", "from a import helper\n\n# helper\ndef main():\n    assert True  # other\n    return helper(1)\n\nhelper\n"),
];

/// File-local exports of spans the edit leaves (`noted`: Python's comments, which no node holds; `said`: the
/// source's lines), and cross-file rules (`remark`, `spoken`) reading their `line` and `col`. A span's base rows are
/// made only where its relation is, so a partial hit's must come from its entry: its chunk makes only what the
/// edited rule reads.
fn spans_program(kind: &str) -> String {
    format!(
        r#"
.decl defined(d: node, x: symbol)
defined(d, x) :- kind(d, "function_definition"), field(d, "name", n), text(n, x).
.decl noted(c: node, x: symbol)
noted(c, x) :- comment(c, x).
.decl remark(x: symbol, l: number, k: number)
remark(x, l, k) :- noted(c, x), defined(_, x), line(c, l), col(c, k).
.decl said(sp: span, x: symbol)
said(sp, x) :- src_line(_, _, sp), text(sp, x).
.decl spoken(x: symbol, l: number, k: number)
spoken(x, l, k) :- said(sp, x), defined(_, x), line(sp, l), col(sp, k).
.decl flagged(n: node)
flagged(n) :- kind(n, "assert_statement"), !kind(n, "{kind}").
.decl flag_col(k: number, l: number)
flag_col(k, l) :- flagged(n), col(n, k), line(n, l).
.output remark, spoken, flag_col
"#
    )
}

/// The edit of `flagged` over files with comments and lines: the cross-file rule sees each comment's place as a full run does.
#[test]
fn a_partial_hit_hands_on_the_base_rows_of_spans_its_chunk_does_not_make() {
    let marker = format!("{:?}", std::time::SystemTime::now());
    let outputs = &["remark", "spoken", "flag_col"];
    let dir = corpus("spans", &marker, COMMENTED);
    let (_, before) = run(&dir, &spans_program("try_statement"), outputs);
    assert_eq!(before[0].len(), 2, "both comments name a function defined in a.py: {before:?}");
    assert_eq!(before[1].len(), 1, "b.py's line `helper` names one: {before:?}");
    let (edited, after) = run(&dir, &spans_program("while_statement"), outputs);
    assert_eq!(edited.memo_partial, COMMENTED.len(), "every file is a partial hit");
    let fresh = corpus("spans-fresh", &format!("{marker} fresh"), COMMENTED);
    let (_, expected) = run(&fresh, &spans_program("while_statement"), outputs);
    assert_eq!(after, expected, "the partial hits' span rows are a full run's");
    assert!(edited.cross_kept.is_some(), "the cross-file part is kept between runs, so its inputs are compared");
    assert!(edited.cross_changed.is_empty(), "the cross-file part's inputs are as the last run's: {:?} changed", edited.cross_changed);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&fresh);
}

const LISTED: &[(&str, &str)] = &[
    ("a.py", "def helper(x):\n    return x\n"),
    ("sub/b.py", "def other(y):\n    return y\n"),
    ("sub/c.py", "x = 1\n"),
];

/// A cross-file rule over a directory's entries (`dir_entry`, answered on request) and every file's exports. The
/// directory is computed (`cat`, as the libraries' `env.rr` computes theirs), or a constant (a request fact).
fn listing_program(dir: &str) -> String {
    format!(
        r#"
.decl dir_entry(dir: symbol, name: symbol, is_dir: number)
.decl defined(d: node, x: symbol)
defined(d, x) :- kind(d, "function_definition"), field(d, "name", n), text(n, x).
.decl listed(n: symbol, x: symbol)
listed(n, x) :- defined(_, x), {dir}.
.output listed
"#
    )
}

/// The same program over the same files, run again and again: the directory's entries are as they were, so the
/// cross-file part sees no input changed, on the second run (after the one that kept it) and every later one.
fn listing_is_no_change(dir: &str) {
    let marker = format!("{:?}", std::time::SystemTime::now());
    let root = corpus(&format!("listing-{}", dir.len()), &marker, &[]);
    std::fs::create_dir_all(root.join("sub")).unwrap();
    for (path, source) in LISTED {
        std::fs::write(root.join(path), format!("# {marker}\n{source}")).unwrap();
    }
    let program = listing_program(dir);
    let (_, first) = run(&root, &program, &["listed"]);
    assert_eq!(first[0].len(), 4, "two entries of sub, two functions: {first:?}");
    for k in 2..=3 {
        let (again, rows) = run(&root, &program, &["listed"]);
        assert_eq!(rows, first, "run {k}: the rows of the first");
        assert!(again.cross_kept.is_some(), "run {k}: the cross-file part is kept");
        assert!(again.cross_changed.is_empty(), "run {k}: the cross-file part's inputs are as the last run's: {:?} changed", again.cross_changed);
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_unchanged_directory_listing_is_no_change_of_the_cross_part() {
    listing_is_no_change(r#"s = cat("su", "b"), dir_entry(s, n, 0)"#);
}

/// A constant directory lowers to a request fact (`rr_dir_req("sub")`), which no stratum derives: the stratum of
/// its answers is not replayed from kept rows (none are kept of a fact) but runs, and the cross part's inputs are as
/// they were.
#[test]
fn an_unchanged_constant_directory_listing_is_no_change_of_the_cross_part() {
    listing_is_no_change(r#"dir_entry("sub", n, 0)"#);
}

/// An edit of a file-local rule (`flagged`'s body) changes no input of the cross-file part, which reads a directory's
/// entries (an input neither exported by the files nor a base relation of theirs): each input's kept frame is found
/// again, under the cross part's own program, not the whole program's texts.
#[test]
fn a_file_local_edit_finds_the_cross_parts_inputs_kept() {
    let marker = format!("{:?}", std::time::SystemTime::now());
    let root = corpus("listing-edit", &marker, &[]);
    std::fs::create_dir_all(root.join("sub")).unwrap();
    for (path, source) in LISTED {
        std::fs::write(root.join(path), format!("# {marker}\n{source}")).unwrap();
    }
    let program = |kind: &str| {
        format!(
            "{}\n.decl flagged(n: node)\nflagged(n) :- kind(n, \"function_definition\"), !kind(n, \"{kind}\").\n.output flagged\n",
            listing_program(r#"s = cat("su", "b"), dir_entry(s, n, 0)"#)
        )
    };
    let (_, first) = run(&root, &program("try_statement"), &["listed"]);
    assert_eq!(first[0].len(), 4, "two entries of sub, two functions: {first:?}");
    for (k, kind) in ["while_statement", "for_statement"].into_iter().enumerate() {
        let (edited, rows) = run(&root, &program(kind), &["listed"]);
        assert_eq!(rows, first, "edit {k}: the cross-file rows are as before");
        assert!(edited.cross_kept.is_some(), "edit {k}: the cross-file part is kept");
        assert!(edited.cross_changed.is_empty(), "edit {k}: no input of the cross part changed: {:?}", edited.cross_changed);
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// Files with string annotations (guests, decision 0055): `Foo` and `Bar` are named only inside annotations.
const ANNOTATED: &[(&str, &str)] = &[
    ("a.py", "from typing import List, Optional\n\nx: \"List[Foo]\" = []\n\ndef f(a: \"Optional[Bar]\") -> None:\n    assert a\n"),
    ("b.py", "def g(b: \"Foo\"):\n    assert b\n    while b:\n        assert b\n"),
];

/// `names` reads the annotations' identifiers through `parsed` and `idents` (so the run makes guests, and hiding
/// copies `idents` for them, folded back into it: `fold_own`); `flagged` and `ids` are the rules an edit changes.
fn annotated(names_edit: &str, flagged_edit: &str, ids_edit: &str) -> String {
    format!(
        r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl text(n: node, s: symbol)
.decl idents(m: node, x: symbol)
idents(m, x) :- node(_, m, "identifier", _, _), text(m, x).
.decl names(x: symbol)
names(x) :- string_annotation(n), parsed(n, "annotation", r), within(m, r), idents(m, x), x != "{names_edit}".
.decl flagged(n: node)
flagged(n) :- kind(n, "assert_statement"), !kind(n, "{flagged_edit}").
.decl flag_col(k: number, l: number)
flag_col(k, l) :- flagged(n), col(n, k), line(n, l).
.decl ids(m: node, x: symbol)
ids(m, x) :- idents(m, x), x != "{ids_edit}".
.output names, flag_col, ids
"#
    )
}

const ANNOTATED_OUTPUTS: &[&str] = &["names", "flag_col", "ids"];

/// An edit's chunks of partial hits run its cone alone, and make only the guests the cone can see
/// (`guests::guests_seen`): an edit of `flagged`, which names no guest and reads no relation an annotation reaches,
/// makes no annotation guest; one of `names`, which reads them through `parsed`, makes them all. Either way the rows
/// are a full run's. Before, every chunk made every guest its entries asked for (27,373 on an edit of ep2, 1.6 s).
#[test]
fn a_partial_hit_makes_only_the_guests_its_cone_can_see() {
    let marker = format!("{:?}", std::time::SystemTime::now());
    let dir = corpus("guests-seen", &marker, ANNOTATED);
    let (first, _) = run(&dir, &annotated("n0", "f0", "i0"), ANNOTATED_OUTPUTS);
    assert_eq!((first.memo_partial, first.guests_unseen), (0, 0), "a first run makes every guest");
    let fresh = |k: &str, program: &str| {
        let fresh = corpus(&format!("guests-seen-fresh-{k}"), &format!("{marker} fresh {k}"), ANNOTATED);
        let (_, rows) = run(&fresh, program, ANNOTATED_OUTPUTS);
        let _ = std::fs::remove_dir_all(&fresh);
        rows
    };
    // `flagged`'s edit: its cone names no guest.
    let program = annotated("n0", "while_statement", "i0");
    let (edited, rows) = run(&dir, &program, ANNOTATED_OUTPUTS);
    assert_eq!(edited.memo_partial, ANNOTATED.len(), "every file is a partial hit");
    assert!(edited.guests_unseen > 0, "the cone of `flagged` sees no annotation: none is made");
    assert_eq!(rows, fresh("flagged", &program), "the rows of a full run");
    // `names`' edit: its cone reads the annotations.
    let program = annotated("n1", "while_statement", "i0");
    let (edited, rows) = run(&dir, &program, ANNOTATED_OUTPUTS);
    assert_eq!(edited.memo_partial, ANNOTATED.len(), "every file is a partial hit");
    assert_eq!(edited.guests_unseen, 0, "the cone of `names` reads every annotation");
    assert!(rows[0].iter().any(|r| r[0] == "Foo"), "the annotations' names: {:?}", rows[0]);
    assert_eq!(rows, fresh("names", &program), "the rows of a full run");
    // `ids`' edit: it names no guest, but reads `idents`, which hiding widened to the annotations it copied it for
    // (`fold_own`): it sees them, as a full run's does.
    let program = annotated("n1", "while_statement", "i1");
    let (edited, rows) = run(&dir, &program, ANNOTATED_OUTPUTS);
    assert_eq!(edited.memo_partial, ANNOTATED.len(), "every file is a partial hit");
    assert!(rows[2].iter().any(|r| r[1] == "Bar"), "`ids` sees the annotations' identifiers: {:?}", rows[2]);
    assert_eq!(rows, fresh("ids", &program), "the rows of a full run");
    let _ = std::fs::remove_dir_all(&dir);
}
