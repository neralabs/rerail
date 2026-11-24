use super::*;
use crate::code_rule_dsl::logic::program_image::fields::ProgramFields;
use crate::code_rule_dsl::logic::program_image::manager::{self, Pipeline};
use crate::code_rule_dsl::logic::program_image::{Builder, CmpOp, Lit as L, RuleRef, TermRef, Version};
use std::sync::Arc;

/// Program text `text` as a version, as a run reads it.
fn read(text: &str) -> Arc<Version> {
    crate::code_rule_dsl::logic::rr::parse_version(text).unwrap()
}

/// `v`'s file-local split (`FileLocal`).
fn split_of(v: &Arc<Version>) -> Arc<FileLocal> {
    manager::analysis::<FileLocal>(v).unwrap()
}

/// `v` with the file-local joins of its cross-file rules pushed into the chunks (`PushLocalJoins`), and the joins made.
fn pushed(v: Arc<Version>) -> (Arc<Version>, Vec<String>) {
    let mut pass = PushLocalJoins::default();
    let mut p = Pipeline::new("test", v);
    p.program(&mut pass);
    (p.finish().unwrap().0, pass.made)
}

/// `v` with every row the chunks hand on located (`locate_all`), and the relations located.
fn located(v: Arc<Version>) -> (Arc<Version>, Result<Vec<String>, Vec<String>>) {
    let mut p = Pipeline::new("test", v);
    let found = locate_all(&mut p);
    (p.finish().unwrap().0, found)
}

/// The relations `locate_all` moved to the cross-file part (`CROSS_FILE`).
fn moved(v: &Version) -> Vec<String> {
    ProgramFields::qualifiers(v).into_iter().filter(|(_, w)| w.contains(CROSS_FILE)).map(|(r, _)| r).collect()
}

/// `v` with each string written as a branded atom's tag (`"#k12"`, `"#n0"`) made that atom (decision 0270), as a
/// run's program writes its kinds, fields and names: the rule text cannot write one.
fn branded(v: Arc<Version>) -> Arc<Version> {
    use crate::code_rule_dsl::logic::front::Branded;
    use crate::code_rule_dsl::logic::program_image::{Rewrite, Site, T};
    struct Brand<'a>(&'a Version);
    impl Rewrite for Brand<'_> {
        fn term(&mut self, _site: Site<'_>, t: TermRef<'_>) -> Option<T<'static>> {
            let TermRef::Str(a) = t else { return None };
            let text = self.0.text(a).strip_prefix('#')?;
            let mut chars = text.chars();
            let tag = chars.next()?;
            Branded::of_tag(tag, chars.as_str().parse().ok()?).map(T::Branded)
        }
    }
    let mut b = Builder::derive(&v);
    b.rewrite_all(&v, &mut Brand(&v)).unwrap();
    b.finish().unwrap().0
}

/// `v` with the qualifiers `words` (relation, word) added, as the passes add them (`strategy`): the rule text cannot
/// write one.
fn qualified(v: Arc<Version>, words: &[(&str, &str)]) -> Arc<Version> {
    use crate::code_rule_dsl::logic::program_image::manager::{Cx, ProgramPass, ProgramW};
    struct Qualify(Vec<(String, String)>);
    impl ProgramPass for Qualify {
        const NAME: &'static str = "qualify";
        fn run(&mut self, _cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
            let words: Vec<(&str, &str)> = self.0.iter().map(|(r, q)| (r.as_str(), q.as_str())).collect();
            w.on()?.fields().add_qualifiers(&words);
            Ok(())
        }
    }
    let mut p = Pipeline::new("test", v);
    p.program(&mut Qualify(words.iter().map(|(r, q)| ((*r).to_owned(), (*q).to_owned())).collect()));
    p.finish().unwrap().0
}

/// `v`'s rules for `head`.
fn rules_of<'v>(v: &'v Version, head: &str) -> Vec<RuleRef<'v>> {
    v.rules().filter(|r| v.rel_name(r.head().rel()) == head).collect()
}

/// The relations rule `r`'s body atoms read, in order.
fn reads(v: &Version, r: RuleRef<'_>) -> Vec<String> {
    r.body().filter_map(|l| l.atom()).map(|a| v.rel_name(a.rel()).to_owned()).collect()
}

/// Source spans are locations but are not tree nodes. Their explicit document column must
/// survive the local split, so locating their rows never adds `node(d, span, ...)`.
#[test]
fn source_relations_keep_their_document_location() {
    for (relation, columns, arguments) in [
        ("source", "d: number, sp: loc", "d, sp"),
        ("src_line", "d: number, l: number, sp: loc", "d, l, sp"),
        ("line_count", "d: number, k: number", "d, k"),
        ("docof", "sp: loc, d: number", "sp, d"),
    ] {
        let text = format!(".decl {relation}({columns})\n.input {relation}\n.decl selected(d: number)\nselected(d) :- {relation}({arguments}).\n.output selected\n");
        let v = read(&text);
        assert_eq!(split_of(&v).locations["selected"], [0], "{relation}: {:?}", split_of(&v).locations);
        let (out, found) = located(v);
        found.unwrap();
        assert!(!out.rules().any(|r| reads(&out, r).iter().any(|a| a == "node")), "{relation}: a span acquired a tree-node constraint");
    }
}

/// A relation whose rows name no document (`aware()`, as data-layer's `ma_branch_aware`), handed on to a cross-file
/// reader (`found`, as `MA033` over `ma_owner(_)`), is moved to the cross-file part with its file-local readers
/// (decision 0290), so `locate_all` leaves every row the chunks hand on located.
#[test]
fn a_relation_naming_no_document_moves_to_the_cross_file_part() {
    let text = ".decl doc(d: number, p: symbol, x: number)\n.input doc\n.decl owner(d: number)\nowner(d) :- doc(d, \"a.py\", _).\n\
                .decl aware()\naware() :- owner(d), d > 0.\n.decl found(k: number)\nfound(1) :- owner(_), !aware().\n.output found\n";
    let v = read(text);
    assert!(split_of(&v).local.contains("aware"), "the case: aware is file-local before the move");
    let (v, found) = located(v);
    let found = found.expect("every row the chunks hand on located");
    assert_eq!(moved(&v), ["aware"]);
    let parts = split_of(&v);
    assert!(!parts.local.contains("aware") && !parts.local.contains("found") && parts.local.contains("owner"), "{:?}, located {found:?}", parts.local);
    assert_eq!(parts.causes["aware"], "its rows name no document (moved by `locate`)");
    // Located again, nothing more moves.
    let (v, found) = located(v);
    found.unwrap();
    assert_eq!(moved(&v), ["aware"]);
}

/// An earlier phase's export is a run constant, but not placeless (decision 0083 §2.3): a rule that joins it is
/// cross-file, maintained there by the export's delta, so that an edit's delta of it (a function added) does not
/// miss every file's memo entry. The run's options stay placeless: a rule that joins them stays file-local.
#[test]
fn a_rule_joining_a_phase_export_is_cross_file_and_one_joining_an_option_is_not() {
    let text = ".decl kind(n: node, k: symbol)\n.input kind\n.decl types__sig(f: symbol, t: symbol)\n.input types__sig\n\
                .decl option(k: symbol, v: symbol)\n.input option\n\
                .decl typed(n: node, t: symbol)\ntyped(n, t) :- kind(n, k), types__sig(k, t).\n.output typed\n\
                .decl opted(n: node)\nopted(n) :- kind(n, k), option(\"x\", k).\n.output opted\n";
    let v = read(text);
    let parts = split_of(&v);
    assert!(parts.local.contains("opted"), "an option is placeless: {:?}", parts.local);
    assert!(!parts.local.contains("typed"), "a phase's export is every file's rows: {:?}", parts.local);
    assert_eq!(parts.causes["typed"], "reads types__sig with no location");
}

/// A file's path names its document (decision 0665): a relation keyed by `doc`'s path, joined with another by that
/// path, stays per-file, and `locate` finds its rows' document through the path. A path is no location: the
/// published locations (which a memo reads a document off) leave it out, and the path is a key. A path written out
/// names one document too, the same in every atom that writes it (a policy about a named file, `bcgp_block("a.py",
/// …)`); two paths are two. A setting's path is not one (`package_for`, bound whole beside every chunk, holds every
/// file's row): two files of one package is a setting, and a per-file rule reading another file's row by it is
/// per-file only by its own document.
#[test]
fn a_join_by_a_files_own_path_stays_per_file() {
    let text = ".decl doc(d: number, path: symbol, module: symbol)\n.input doc\n.decl src_line(d: number, l: number, sp: span)\n.input src_line\n\
                .decl package_for(path: symbol, dir: symbol)\n.input package_for\n\
                .decl line_of(p: symbol, l: number)\nline_of(p, l) :- doc(d, p, _), src_line(d, l, _).\n\
                .decl last(p: symbol, k: number)\nlast(p, k) :- line_of(p, _), k = max l : { line_of(p, l) }.\n\
                .decl lead(p: symbol, l: number)\nlead(p, l) :- last(p, k), line_of(p, l), l < k.\n.output lead\n\
                .decl same_pkg(p: symbol, q: symbol)\nsame_pkg(p, q) :- package_for(p, dir), package_for(q, dir), p != q.\n.output same_pkg\n\
                .decl named(l: number)\nnamed(l) :- line_of(\"a.py\", l), last(\"a.py\", k), l < k.\n.output named\n\
                .decl shared(p: symbol)\nshared(p) :- line_of(p, l), line_of(q, l), p != q.\n.output shared\n";
    let both = ".decl both(l: number)\nboth(l) :- line_of(\"a.py\", l), line_of(\"b.py\", l).\n.output both\n";
    let split = split_of(&read(&format!("{text}{both}")));
    assert!(!split.local.contains("both"), "two paths: {:?}", split.local);
    assert!(split.causes["both"].starts_with("joins 2 documents"), "{}", split.causes["both"]);
    let v = read(text);
    let split = split_of(&v);
    for rel in ["line_of", "last", "lead"] {
        assert!(split.local.contains(rel), "{rel}: {:?} {:?}", split.local, split.causes);
        assert_eq!(split.locations[rel], Vec::<usize>::new(), "{rel}: a path is no location: {:?}", split.locations);
        assert_eq!(split.path_keys.get(rel).map(Vec::as_slice), Some(&[0][..]), "{rel}: its path is a key: {:?}", split.path_keys);
    }
    assert!(split.constants.contains("same_pkg"), "settings alone: {:?} {:?}", split.constants, split.causes);
    assert!(split.local.contains("named"), "one path written twice: {:?} {:?}", split.local, split.causes);
    // Located: `lead` is derived with its document, found through its path.
    let (out, found) = located(v);
    assert!(found.as_ref().is_ok_and(|f| f.iter().any(|r| r == "lead") && f.iter().any(|r| r == "named")), "{found:?}");
    let made: Vec<String> = out.rules().filter(|r| out.rel_name(r.head().rel()).starts_with("lead_at")).flat_map(|r| reads(&out, r)).collect();
    assert!(made.iter().any(|r| r == "doc"), "the located rule reads its document off its path: {made:?}");
    // `line_of`, read by per-file rules and by the cross-file `shared`, stays, and is relayed: `shared` reads its
    // located form, of documents found through its path.
    assert!(found.as_ref().is_ok_and(|f| f.iter().any(|r| r == "line_of")), "{found:?}");
    assert!(!moved(&out).iter().any(|r| r == "line_of"), "{:?}", moved(&out));
    let shared: Vec<String> = out.rules().filter(|r| out.rel_name(r.head().rel()) == "shared").flat_map(|r| reads(&out, r)).collect();
    assert!(!shared.is_empty() && shared.iter().all(|r| r.starts_with("line_of_at")), "{shared:?}");
    let split = split_of(&out);
    assert!(split.local.iter().any(|r| r.starts_with("lead_at")) && split.local.contains("line_of"), "{:?} {:?}", split.local, split.causes);
    // What the cross part reads back from a located form stays there, though its path would place it in one file.
    assert!(!split.local.contains("lead") && !split.local.contains("named"), "{:?}", split.local);
    // Located again (a later round), nothing is relayed twice.
    let (again, _) = located(out.clone());
    assert_eq!(again.rules().count(), out.rules().count(), "located once");
    let relay = split.local.iter().find(|r| r.starts_with("line_of_at")).expect("the relay is per-file");
    assert_eq!(split.locations[relay], vec![0], "the relay's rows name their document: {:?}", split.locations);
}

/// A name built from a file's path names its document too (decision 0665): a module from the path (a named group of a
/// whole-path pattern, its separators replaced), a symbol the module leads (`cat(m, ".", q)`). A relation keyed by
/// such a symbol, and an aggregate over it (eager-resolve's `er_last_def`: the last def of each symbol), stay
/// per-file; the symbol is a key, no location. A symbol the module does not lead is no file's name: the aggregate
/// over it is cross-file.
#[test]
fn a_symbol_built_from_a_files_module_stays_per_file() {
    let text = ".decl doc(d: number, path: symbol, module: symbol)\n.input doc\n\
                .decl qual(f: node, q: symbol)\n.input qual\n.decl docof(n: node, d: number)\n.input docof\n.decl line(n: node, l: number)\n.input line\n\
                .decl mod_of(d: number, m: symbol)\nmod_of(d, @replace_all(m, \"/\", \".\")) :- doc(d, p, _), match(\"(?<m>.*)\\\\.py\", p).\n\
                .decl sym(f: node, s: symbol)\nsym(f, cat(m, \".\", q)) :- qual(f, q), docof(f, d), mod_of(d, m).\n\
                .decl last_def(s: symbol, f: node)\nlast_def(s, f) :- sym(f, s), line(f, l), l = max a : { sym(g, s), line(g, a) }.\n.output last_def\n\
                .decl tail(f: node, s: symbol)\ntail(f, cat(q, \".\", m)) :- qual(f, q), docof(f, d), mod_of(d, m).\n\
                .decl last_tail(s: symbol, f: node)\nlast_tail(s, f) :- tail(f, s), line(f, l), l = max a : { tail(g, s), line(g, a) }.\n.output last_tail\n";
    let split = split_of(&read(text));
    for rel in ["mod_of", "sym", "last_def"] {
        assert!(split.local.contains(rel), "{rel}: {:?} {:?}", split.local, split.causes);
    }
    assert_eq!(split.path_keys.get("sym").map(Vec::as_slice), Some(&[1][..]), "the symbol is a key: {:?}", split.path_keys);
    assert_eq!(split.locations["sym"], vec![0], "and no location: {:?}", split.locations);
    assert!(!split.local.contains("last_tail"), "a symbol the module does not lead: {:?}", split.local);
}

/// Module resolution handed on by a phase (decision 0665, tsx/names.rerail over the phase `tsx_names`): the phase's
/// export, symbols alone and every file's rows, is read as a setting is, and a file's own targets are its rows by
/// its path, so a rule resolving a file's imports through it stays per-file. The same resolution made in the run
/// (every file's paths joined) is cross-file.
#[test]
fn a_rule_resolving_through_a_phases_names_stays_per_file() {
    let text = ".decl doc(d: number, path: symbol, module: symbol)\n.input doc\n.decl docof(n: node, d: number)\n.input docof\n\
                .decl import_ref(n: node, s: symbol)\n.input import_ref\n\
                .decl tsx_names__target_at(p: symbol, spec: symbol, t: symbol)\n.decl tsx_names__exports(m: symbol, x: symbol, q: symbol)\n\
                .decl target(d: number, spec: symbol, t: symbol)\ntarget(d, s, t) :- doc(d, p, _), tsx_names__target_at(p, s, t).\n\
                .decl sym(n: node, q: symbol)\nsym(n, q) :- import_ref(n, s), docof(n, d), target(d, s, t), tsx_names__exports(t, \"default\", q).\n.output sym\n\
                .decl near(d: number, s: symbol, t: symbol)\nnear(d, s, m) :- docof(n, d), import_ref(n, s), doc(_, s, m).\n.output near\n";
    let split = split_of(&read(text));
    for rel in ["target", "sym"] {
        assert!(split.local.contains(rel), "{rel}: {:?} {:?}", split.local, split.causes);
    }
    assert!(!split.local.contains("near"), "another file's path, joined in the run: {:?}", split.local);
}

/// The project's settings (decision 0665: a package's `package.json` fields, its tsconfig, the run's environment) are
/// read like the run's options: a per-file rule that joins them stays per-file, and a relation derived from them alone
/// is a constant, derived in both parts. A rule that reads another file's rows, or the tree's list of files (no
/// configuration), is cross-file.
#[test]
fn a_rule_reading_a_setting_stays_per_file_and_one_reading_another_files_rows_does_not() {
    let text = ".decl doc(d: number, path: symbol, module: symbol)\n.input doc\n.decl kind(n: node, k: symbol)\n.input kind\n\
                .decl docof(n: node, d: number)\n.input docof\n.decl file(path: symbol)\n.input file\n\
                .decl package_for(path: symbol, dir: symbol)\n.input package_for\n.decl package_field(dir: symbol, key: symbol, v: symbol)\n.input package_field\n\
                .decl env(key: symbol, value: symbol)\n.input env\n\
                .decl module_doc(d: number)\nmodule_doc(d) :- doc(d, p, _), package_for(p, dir), package_field(dir, \"type\", \"module\").\n.output module_doc\n\
                .decl on_linux(n: node)\non_linux(n) :- kind(n, \"if_statement\"), env(\"python.platform\", \"linux\").\n.output on_linux\n\
                .decl pkg_dep(dir: symbol, m: symbol)\npkg_dep(dir, m) :- package_field(dir, \"dependencies/react\", m).\n\
                .decl reactish(d: number)\nreactish(d) :- doc(d, p, _), package_for(p, dir), pkg_dep(dir, _).\n.output reactish\n\
                .decl elsewhere(d: number)\nelsewhere(d) :- doc(d, _, _), kind(n, \"if_statement\"), docof(n, e), e != d.\n.output elsewhere\n\
                .decl listed(d: number)\nlisted(d) :- doc(d, _, _), file(\"setup.py\").\n.output listed\n\
                .decl dep_list(l: symbol)\ndep_list(\"dependencies\"). dep_list(\"peerDependencies\").\n\
                .decl dep_package(m: symbol, r: symbol)\ndep_package(\"react\", \"react\"). dep_package(\"vue\", \"other\").\n\
                .decl package_dep(dir: symbol, l: symbol, m: symbol)\npackage_dep(dir, l, m) :- dep_list(l), dep_package(m, _), package_field(dir, cat(l, \"/\", @replace_all(m, \"/\", \"~1\")), _).\n\
                .decl deps_on(dir: symbol, r: symbol)\ndeps_on(dir, r) :- package_dep(dir, _, m), dep_package(m, r).\n\
                .decl other_jsx(d: number)\nother_jsx(d) :- doc(d, p, _), package_for(p, dir), deps_on(dir, \"other\"), !deps_on(dir, \"react\").\n.output other_jsx\n";
    let split = split_of(&read(text));
    // React's gate (`tsx/react.rerail`): settings and facts joined with a file's own path.
    assert!(split.constants.contains("deps_on"), "{:?} {:?}", split.constants, split.causes);
    for rel in ["module_doc", "on_linux", "reactish", "other_jsx"] {
        assert!(split.local.contains(rel), "{rel}: {:?} {:?}", split.local, split.causes);
    }
    assert!(split.constants.contains("pkg_dep"), "derived from settings alone: {:?}", split.constants);
    assert!(!split.local.contains("elsewhere"), "{:?}", split.local);
    assert!(!split.local.contains("listed"), "the tree's files are no setting: {:?}", split.local);
    assert_eq!(split.causes["listed"], "reads file with no location");
}

/// A nullable location tested set places its row (decision 0672): `sig_param`'s default is a node of the parameter's
/// document or -1, and a rule keeping only `d != -1` (Ruff's PYI011, PYI014: the default node itself) has a node of
/// that document in its head. `locate` names the row's document through it, and nothing moves to the cross-file part.
#[test]
fn a_nullable_location_tested_set_places_its_head() {
    let text = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl child(p: node, c: node, i: number)
.decl docof(n: node, d: number)
.input node, child, docof
.decl param(f: node, d: loc)
param(f, d) :- node(_, f, "param", _, _), child(f, d, 0).
param(f, d) :- node(_, f, "param", _, _), !child(f, _, 0), unset(d).
.decl simple(d: node)
simple(d) :- node(_, d, "number", _, _).
.decl bad(d: node)
bad(d) :- param(f, d), !unset(d), !simple(d).
.output bad
"#;
    let v = read(text);
    let split = split_of(&v);
    assert!(split.local.contains("bad"), "{:?} {:?}", split.local, split.causes);
    assert_eq!(split.locations["bad"], [0], "the default, tested set, is a location: {:?}", split.locations);
    let (v, found) = located(v);
    found.unwrap();
    assert!(moved(&v).is_empty(), "nothing moves: {:?}", moved(&v));
}

/// A pair cache's guard chooses how its readers are evaluated, not their rows (decision 0652's `strategy`): a chunk's
/// guard is the chunk's choice. A cross-file reader of it (`far`, declared) does not make it an export, which `locate`
/// could not place and would move with every reader (Ruff's `local_class`: RUF049, ctypes): the cross-file part
/// derives its own guard, and its own pairs, from the factors it reads (decision 0672).
#[test]
fn a_guard_read_across_files_is_each_parts_own() {
    let text = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl child(p: node, c: node, i: number)
.decl text(n: node, s: symbol)
.input node, child, text
.decl fa(e: node, m: node)
fa(e, m) :- node(_, e, "name", _, _), child(e, m, 0).
.decl fb(k: node, m: node)
fb(k, m) :- node(_, k, "class", _, _), child(k, m, 1).
.decl small(u: number)
small(1) :- a = count : { fa(_, _) }, b = count : { fb(_, _) }, a * b < 100.
.decl pairs(e: node, k: node)
pairs(e, k) :- small(1), fa(e, m), fb(k, m).
.decl lc(e: node, k: node)
lc(e, k) :- fa(e, m), fb(k, m), !small(1).
lc(e, k) :- pairs(e, k).
.decl used(e: node)
used(e) :- lc(e, _).
.decl far(f: node, k: node) cross_file
far(f, k) :- node(_, f, "call", _, _), text(f, x), node(_, k, "class", _, _), text(k, x), fa(f, m), fb(k, m), !small(1).
far(f, k) :- node(_, f, "call", _, _), text(f, x), node(_, k, "class", _, _), text(k, x), pairs(f, k).
.output used, far
"#;
    let v = qualified(read(text), &[("small", STRATEGY)]);
    let (v, found) = located(v);
    found.unwrap();
    let moved_by_locate: Vec<String> = ProgramFields::qualifiers(&*v).into_iter().filter(|(_, w)| w.contains(MOVED_CROSS_FILE)).map(|(r, _)| r).collect();
    assert!(moved_by_locate.is_empty(), "the guard is not moved: {moved_by_locate:?}");
    let split = split_of(&v);
    for rel in ["small", "lc", "used"] {
        assert!(split.local.contains(rel), "{rel}: {:?} {:?}", split.local, split.causes);
    }
    let p = programs_of(v).unwrap();
    assert!(!p.exports.iter().any(|x| x == "small"), "the cross-file part's guard is its own: {:?}", p.exports);
    crossings_checked(&p).unwrap();
}

/// A view of values read by a cross-file rule as well as a file-local one is each part's own (decision 0672): its
/// rows name no document, so no part can hand them on, and the cross-file part derives its own over every file's
/// rows, as the whole program does, while each chunk keeps its own for its readers. UP032 is cross-file through
/// Python's resolution (`translated`) and reads the format family (`brace_ok`, `mixed`); its `refers` stays per-file.
#[test]
fn a_view_of_values_a_cross_file_rule_reads_is_each_parts_own() {
    let text = r##"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl text(n: node, s: symbol)
.input node, text
.decl strs(c: node, v: symbol)
strs(c, v) :- node(_, c, "#k20", _, _), text(c, v).
.decl fval(v: symbol)
fval(v) :- strs(_, v).
.decl vok(v: symbol)
vok(v) :- fval(v), v != "#n0".
.decl here(c: node)
here(c) :- strs(c, v), vok(v).
.decl far(c: node, k: node) cross_file
far(c, k) :- strs(c, v), vok(v), node(_, k, "#k21", _, _), text(k, v).
.output here, far
"##;
    let v = branded(read(text));
    let (v, found) = located(v);
    found.unwrap();
    let moved_by_locate: Vec<String> = ProgramFields::qualifiers(&*v).into_iter().filter(|(_, w)| w.contains(MOVED_CROSS_FILE)).map(|(r, _)| r).collect();
    assert!(moved_by_locate.is_empty(), "nothing moves: {moved_by_locate:?}");
    let split = split_of(&v);
    for rel in ["fval", "vok", "here"] {
        assert!(split.local.contains(rel), "{rel}: {:?} {:?}", split.local, split.causes);
    }
    assert!(split.views().contains("vok") && split.views().contains("fval"), "{:?}", split.views());
    let p = programs_of(v).unwrap();
    for rel in ["fval", "vok"] {
        assert!(!p.exports.iter().any(|x| x == rel), "{rel} is not handed on: {:?}", p.exports);
        let cross = &p.versions.cross;
        assert!(cross.rules().any(|r| cross.rel_name(r.head().rel()) == rel), "the cross-file part derives its own {rel}");
    }
    assert!(p.exports.iter().any(|x| x == "strs"), "what the view reads is handed on: {:?}", p.exports);
    crossings_checked(&p).unwrap();
}

/// A view of values gathered from every file and read by a rule holding the value in its own document is per-file
/// (decision 0672): each of `parts`' rules reads one row of `item` and tests or splits its value, and its reader reads
/// that row of its own document, so derived per chunk it answers the reader as over every file (ESLint's globals
/// comment, `item_parts`). A view whose rules differ only in the node kinds they read and test the value alike
/// (`zero`: an integer's or a float's text) is answered by its reader's own node through the rule of its kind
/// (Ruff's SIM113 `zero_number`), its node's id range bounded as the lowering bounds it. A value built from the row's
/// key (`spelled`'s `cat("'", v)`) is the key's. A family of views over one value (a format string's fields,
/// pyflakes-format's `brace_…`: `farg` reads `fname`, which reads `ffield`, which reads `frest`, which reads `fvalue`
/// and itself) is answered at the values its outer reader holds, each view passed through at those values, plain,
/// negated or counted, also where the reader holds the value through a rule of its own (`named` reads `braced`, every
/// row of which is a `call_str` row). The constants are branded as a run's are (decision 0270: `#n0` is "", `#k12` a kind).
#[test]
fn a_view_of_a_value_its_reader_holds_is_per_file() {
    let text = r##"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl text(n: node, s: symbol)
.decl line(n: node, l: number)
.input node, text, line
.decl list(c: node, s: symbol, k: number)
list(c, B, 0) :- node(_, c, "#k11", _, _), text(c, v), match("globals?(?<B>\\s[\\s\\S]*)", v).
list(c, T, k + @count_matches("\n", W)) :- list(c, s, k), match("(?<W>[\\s,]*)[^\\s,]+(?<T>[\\s\\S]*)", s).
.decl item(c: node, i: symbol, k: number)
item(c, I, k + @count_matches("\n", W)) :- list(c, s, k), match("(?<W>[\\s,]*)(?<I>[^\\s,]+)[\\s\\S]*", s).
.decl parts(i: symbol, x: symbol, v: symbol)
parts(i, N, V) :- item(_, i, _), match("(?<N>[^:]*):(?<V>[\\s\\S]*)", i).
parts(i, i, "#n0") :- item(_, i, _), !contains(":", i).
.decl global(c: node, x: symbol, v: symbol, l: number)
global(c, x, v, l + k) :- item(c, i, k), parts(i, x, v), line(c, l).
.output global
.decl zero(t: symbol)
zero(t) :- node(_, n, "#k12", _, _), text(n, t), match("0+", t), n < 6917529027641081856.
zero(t) :- node(_, n, "#k13", _, _), text(n, t), match("0+", t), n < 6917529027641081856.
.decl start(z: node)
start(z) :- node(_, z, "#k12", _, _), text(z, t), zero(t).
.output start
.decl str(s: node, v: symbol)
str(s, v) :- node(_, s, "#k15", _, _), text(s, v).
.decl spelled(v: symbol, k: symbol)
spelled(v, v) :- str(_, v), !match("#.*", v).
spelled(v, cat("'", v)) :- str(_, v), match("#.*", v).
.decl key(at: node, k: symbol)
key(at, k) :- node(_, at, "#k15", _, _), str(at, v), spelled(v, k).
.output key
.decl call_str(c: node, v: symbol)
call_str(c, v) :- node(_, c, "#k20", _, _), text(c, v).
.decl fvalue(v: symbol)
fvalue(v) :- call_str(_, v).
.decl frest(v: symbol, r: symbol, i: number)
frest(v, v, 0) :- fvalue(v).
frest(v, rest, i + 1) :- ffield(v, i, _, rest).
.decl ffield(v: symbol, i: number, f: symbol, rest: symbol)
ffield(v, i, f, rest) :- frest(v, r, i), match("[^{}]*\\{(?<f>[^{}]*)\\}(?<rest>[\\s\\S]*)", r).
.decl fname(v: symbol, k: number, f: symbol)
fname(v, i * 1000, f) :- ffield(v, i, f, _).
.decl farg(v: symbol, k: number, a: symbol)
farg(v, k, a) :- fname(v, k, f), match("(?<a>[^.\\[]*)[\\s\\S]*", f).
.decl fbad(v: symbol)
fbad(v) :- fvalue(v), !ffield(v, 0, _, _).
.decl refers(c: node, k: number)
refers(c, k) :- call_str(c, v), farg(v, k, ""), !fbad(v), r = count : { farg(v, k2, ""), k2 < k }, r >= 0.
.output refers
.decl braced(c: node, v: symbol)
braced(c, v) :- call_str(c, v), contains("{", v).
.decl named(c: node, k: number)
named(c, k) :- node(_, c, "#k20", _, _), braced(c, v), farg(v, k, _).
.output named
"##;
    let v = branded(read(text));
    let split = split_of(&v);
    for rel in ["parts", "global", "zero", "start", "spelled", "key", "fvalue", "frest", "ffield", "fname", "farg", "fbad", "refers", "named"] {
        assert!(split.local.contains(rel), "{rel}: {:?} {:?}", split.local, split.causes);
    }
    let p = programs_of(v.clone()).unwrap();
    crossings_checked(&p).unwrap();
    let (v, found) = located(v);
    found.unwrap();
    assert!(moved(&v).is_empty(), "{:?}", moved(&v));
    // A reader holding a value of another kind than any rule reads is not answered by its own file.
    let other = text.replace(r##"start(z) :- node(_, z, "#k12", _, _)"##, r##"start(z) :- node(_, z, "#k14", _, _)"##);
    assert_ne!(other, text);
    assert!(!split_of(&branded(read(&other))).local.contains("start"), "a string's text is answered by another file's number");
    // A count of a view's rows at a column its reader leaves open counts every file's: a format string's pairs with
    // any file's strings are not one file's.
    let pairs = text.replace(".output refers\n", ".output refers\n.decl pairs(v: symbol, w: symbol)\npairs(v, w) :- call_str(_, v), str(_, w).\n\
                                                   .decl many(c: node, n: number)\nmany(c, n) :- call_str(c, v), n = count : { pairs(v, _) }.\n.output many\n");
    assert_ne!(pairs, text);
    let split = split_of(&branded(read(&pairs)));
    assert!(split.local.contains("refers") && !split.local.contains("many"), "{:?} {:?}", split.local, split.causes);
}

/// A family of views over an identifier's text, decoded piece by piece (ESLint's `id_value`: `unescape` joins the
/// pieces `escape_piece` splits, each decoded by `hex_piece`), is per-file (decision 0672): its reader holds the text
/// in its own node, and each view is passed through at it, its pieces counted (the join) at the text the reader
/// holds. The text's view is bounded by its node's id range as the lowering writes it.
#[test]
fn a_decoded_name_is_per_file() {
    let text = r##"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl text(n: node, s: symbol)
.input node, text
.decl id_kind(n: node)
id_kind(n) :- node(_, n, "#k30", _, _).
id_kind(n) :- node(_, n, "#k31", _, _).
.decl escaped_id(s: symbol)
escaped_id(s) :- id_kind(n), text(n, s), contains("\\", s), n < 6917529027641081856.
.decl escape_piece(s: symbol, o: number, p: symbol)
escape_piece(s, o, p) :- escaped_id(s), match_all("(?<p>\\\\u[0-9a-fA-F]{4}|[^\\\\]+|\\\\)", s, o, _, _).
.decl hex_char(h: symbol, c: symbol)
hex_char("0061", "a"). hex_char("0062", "b").
.decl hex_piece(p: symbol, d: symbol)
hex_piece(p, d) :- escape_piece(_, _, p), match("\\\\u(?<H>[0-9a-fA-F]{4})", p), hex_char(@lower(H), d).
.decl hex_decodes(p: symbol)
hex_decodes(p) :- hex_piece(p, _).
.decl unescape_piece(s: symbol, o: number, d: symbol)
unescape_piece(s, o, p) :- escape_piece(s, o, p), !contains("\\", p).
unescape_piece(s, o, d) :- escape_piece(s, o, p), hex_piece(p, d).
unescape_piece(s, o, p) :- escape_piece(s, o, p), contains("\\", p), !hex_decodes(p).
.decl unescape(s: symbol, t: symbol)
unescape(s, t) :- escaped_id(s), t = join d by o : { unescape_piece(s, o, d) }.
.decl id_value(n: node, x: symbol)
id_value(n, x) :- id_kind(n), text(n, x), !contains("\\", x).
id_value(n, x) :- id_kind(n), text(n, s), contains("\\", s), unescape(s, x), !contains("\\", x).
.output id_value
"##;
    let v = branded(read(text));
    let split = split_of(&v);
    for rel in ["escaped_id", "escape_piece", "hex_piece", "hex_decodes", "unescape_piece", "unescape", "id_value"] {
        assert!(split.local.contains(rel), "{rel}: {:?} {:?}", split.local, split.causes);
    }
    let p = programs_of(v.clone()).unwrap();
    crossings_checked(&p).unwrap();
    let (v, found) = located(v);
    found.unwrap();
    assert!(moved(&v).is_empty(), "{:?}", moved(&v));
}

/// pyflakes-format's `str.format` family as the library writes it (decision 0672): a format string's tokens, fields,
/// nested fields, names, arguments, its faults and whether it is well formed, each a view of the string's values,
/// read by a call holding the string in its own node, plain (F522's keywords), negated (F521), counted (the
/// automatic numbering) and ranked (UP032's `refers`: a field's position among the automatic ones, a rank over the
/// string's fields joined to the call's arguments), is per-file.
#[test]
fn a_format_strings_fields_are_per_file() {
    let text = r##"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl text(n: node, s: symbol)
.input node, text
.decl call_str(c: node, v: symbol)
call_str(c, v) :- node(_, c, "#k20", _, _), text(c, v).
.decl kw(c: node, x: symbol)
kw(c, x) :- node(d, c, "#k20", _, _), node(d, a, "#k21", _, _), text(a, x), a > c.
.decl brace_value(v: symbol)
brace_value(v) :- call_str(_, v).
// brace_rest(v, r, i): after i tokens (literal braces and fields), the rest of v is r.
.decl brace_rest(v: symbol, r: symbol, i: number)
brace_rest(v, v, 0) :- brace_value(v).
brace_rest(v, rest, i + 1) :- brace_rest(v, r, i), match("[^{}]*(\\{\\{|\\}\\})(?<rest>[\\s\\S]*)", r).
brace_rest(v, rest, i + 1) :- brace_field(v, i, _, _, _, rest).
// brace_field(v, i, name, conv, spec, rest): token i is a field. conv keeps its `!`, spec its `:`.
.decl brace_field(v: symbol, i: number, name: symbol, conv: symbol, spec: symbol, rest: symbol)
brace_field(v, i, f, cv, sp, rest) :- brace_rest(v, r, i),
    match("[^{}]*\\{(?<f>[^{}!:]*)(?<cv>(![^{}:]*)?)(?<sp>(:([^{}]|\\{[^{}]*\\})*)?)\\}(?<rest>[\\s\\S]*)", r).
// brace_nested(v, i, j, name, conv, rest): nested field j in the spec of field i (`{:{width}}`); rest follows it.
.decl spec_rest(v: symbol, i: number, r: symbol, j: number)
spec_rest(v, i, sp, 0) :- brace_field(v, i, _, _, sp, _).
spec_rest(v, i, rest, j + 1) :- brace_nested(v, i, j, _, _, rest).
.decl brace_nested(v: symbol, i: number, j: number, name: symbol, conv: symbol, rest: symbol)
brace_nested(v, i, j, f, cv, rest) :- spec_rest(v, i, r, j),
    match("[^{}]*\\{(?<f>[^{}!:]*)(?<cv>(![^{}:]*)?)(:[^{}]*)?\\}(?<rest>[\\s\\S]*)", r).
// brace_name(v, k, name, conv): every field, top-level and nested; k orders them (Ruff numbers `{}` in this order).
.decl brace_name(v: symbol, k: number, name: symbol, conv: symbol)
brace_name(v, i * 1000, f, cv) :- brace_field(v, i, f, cv, _, _).
brace_name(v, i * 1000 + j + 1, f, cv) :- brace_nested(v, i, j, f, cv, _).
// brace_arg(v, k, a): field k's argument part (before any `.attr` or `[key]`).
.decl brace_arg(v: symbol, k: number, a: symbol)
brace_arg(v, k, a) :- brace_name(v, k, f, _), match("(?<a>[^.\\[]*)[\\s\\S]*", f).
.decl brace_bad(v: symbol)
// A brace that starts no token: a lone `}`, an unclosed `{`, a spec nested two deep.
brace_bad(v) :- brace_rest(v, r, i), (contains("{", r) ; contains("}", r)), !match("[^{}]*(\\{\\{|\\}\\})[\\s\\S]*", r),
    !brace_field(v, i, _, _, _, _).
// A field whose name has an empty `.` part or an unclosed or empty `[ ]`, or a conversion other than r, s, a.
brace_bad(v) :- brace_name(v, _, f, _), !match("[^.\\[]*([.][^.\\[\\]]+|\\[[^\\]]+\\])*", f).
brace_bad(v) :- brace_name(v, _, _, cv), cv != "", cv != "!r", cv != "!s", cv != "!a".
.decl brace_ok(v: symbol)
brace_ok(v) :- brace_value(v), !brace_bad(v).
.decl brace_auto(v: symbol, n: number)
brace_auto(v, n) :- brace_ok(v), n = count : { brace_arg(v, k, "") }.
.decl brace_index(v: symbol, i: number)
brace_index(v, to_number(a)) :- brace_ok(v), brace_arg(v, _, a), match("[0-9]+", a).
.decl brace_keyword(v: symbol, x: symbol)
brace_keyword(v, a) :- brace_ok(v), brace_arg(v, _, a), a != "", !match("[0-9]+", a).

.decl f521(c: node)
f521(c) :- call_str(c, v), !brace_ok(v).
.output f521
.decl f522(c: node, x: symbol)
f522(c, x) :- call_str(c, v), brace_ok(v), kw(c, x), !brace_keyword(v, x).
.output f522
.decl autos(c: node, n: number)
autos(c, n) :- call_str(c, v), brace_auto(v, n).
.output autos
.decl indexed(c: node, i: number)
indexed(c, i) :- call_str(c, v), brace_index(v, i).
.output indexed
.decl call_pos(c: node, i: number, e: node)
call_pos(c, i, e) :- node(d, c, "#k20", _, _), node(d, e, "#k22", i, _).
.decl refers(c: node, k: number, e: node)
refers(c, k, e) :- call_str(c, v), brace_arg(v, k, "#n0"), r = count : { brace_arg(v, k2, "#n0"), k2 < k }, call_pos(c, r, e).
// A batch's scope tests (decision 0091) on each document a rule reads, and a demand carrying them, read through a view
// by a reader carrying them too (the rank's query of UP032's `refers` in a pack of several scopes).
.decl doc(d: number, p: symbol, x: number)
.input doc
.decl scope_in(d: number)
scope_in(d) :- doc(d, p, _), p != "#n1".
.decl scope_not(d: number)
scope_not(d) :- doc(d, _, _), !scope_in(d).
.decl want(k: number, v: symbol)
want(k, v) :- call_str(c, v), s0 = @doc(c), !scope_not(s0), brace_arg(v, k, "#n0"), call_pos(c, _, e), s1 = @doc(e), !scope_not(s1).
.decl wanted(v: symbol)
wanted(v) :- want(_, v).
.decl scoped(c: node, k: number, e: node)
scoped(c, k, e) :- call_str(c, v), s0 = @doc(c), !scope_not(s0), brace_arg(v, k, "#n0"), wanted(v), call_pos(c, r, e), s1 = @doc(e), !scope_not(s1), r = k.
.output scoped
.output refers
"##;
    let v = branded(read(text));
    let split = split_of(&v);
    for rel in ["brace_value", "brace_rest", "brace_field", "spec_rest", "brace_nested", "brace_name", "brace_arg", "brace_bad", "brace_ok", "brace_auto", "brace_keyword", "brace_index", "f521", "f522", "autos", "indexed", "refers", "want", "wanted", "scoped"] {
        assert!(split.local.contains(rel), "{rel}: {:?} {:?}", split.local, split.causes);
    }
    let (v, found) = located(v);
    found.unwrap();
    assert!(moved(&v).is_empty(), "{:?}", moved(&v));
}

/// A view of one document's text read by every file (plan-optimizer-v2's `pv2_doc_text`: the design doc's codes,
/// tested by each call site) is cross-file with its readers, and located (decision 0672): the row a function of strings
/// reads (`match_all`'s demand, `rr_ctx…`) is the view's own, never a ground for finding it in the reader's file.
#[test]
fn one_documents_text_read_by_every_file_is_cross_file() {
    let text = r##"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl text(n: node, s: symbol)
.input node, text
.decl call_str(c: node, v: symbol)
call_str(c, v) :- node(_, c, "#k20", _, _), text(c, v).
.decl design(s: symbol)
design(s) :- node(_, n, "#k40", _, _), text(n, s).
.decl codes(v: symbol)
codes(v) :- design(s), match_all("`(?<v>[a-z]+)`", s).
.decl missing(c: node, v: symbol)
missing(c, v) :- call_str(c, v), design(_), !codes(v).
.output missing
"##;
    let v = branded(read(text));
    let split = split_of(&v);
    for rel in ["codes", "missing"] {
        assert!(!split.local.contains(rel), "{rel}: {:?} {:?}", split.local, split.causes);
    }
    let (v, found) = located(v);
    found.unwrap();
    assert!(moved(&v).is_empty(), "{:?}", moved(&v));
    crossings_checked(&programs_of(v).unwrap()).unwrap();
}

/// Two trees in one chunk (kinds 1 = module, 2 = function, 3 = other):
///
/// ```text
/// 0 module            5 module
/// ├ 1 function        └ 6 other
/// │ └ 2 other           └ 7 function
/// │   └ 3 function
/// └ 4 other
/// ```
fn chunk() -> (Vec<u32>, Vec<i64>) {
    (vec![NONE, 0, 1, 2, 0, NONE, 5, 6], vec![1, 2, 3, 2, 3, 1, 3, 2])
}

#[test]
fn nearest_is_each_nodes_closest_ancestor_in_the_set() {
    let (parent, kind) = chunk();
    let trees = Trees { parent: &parent, kind: &kind };
    let sets = [vec![1], vec![2], vec![1, 2]];
    let got = nearest(&trees, &sets, Binding::Cpu).unwrap();
    assert_eq!(got[0], [NONE, 0, 0, 0, 0, NONE, 5, 5]);
    assert_eq!(got[1], [NONE, NONE, 1, 1, NONE, NONE, NONE, NONE]);
    assert_eq!(got[2], [NONE, 0, 1, 1, 0, NONE, 5, 5]);
}

/// The device's work queue solves the same system to the same values (where there is a device).
#[test]
fn the_device_finds_the_cpus_nearest() {
    // A deep tree and a wide one, over three sets.
    let mut parent = vec![NONE];
    let mut kind = vec![1];
    for i in 1..20_000_u32 {
        parent.push(if i % 7 == 0 { i / 2 } else { i - 1 });
        kind.push(i64::from(i % 5));
    }
    let trees = Trees { parent: &parent, kind: &kind };
    let sets = [vec![1], vec![2, 3], vec![4]];
    let cpu = nearest(&trees, &sets, Binding::Cpu).unwrap();
    let metal = nearest(&trees, &sets, Binding::Metal).unwrap();
    assert_eq!(cpu, metal);
    // Climbing parents gives the same answer.
    for (k, set) in sets.iter().enumerate() {
        for node in 0..parent.len() {
            let mut at = parent[node];
            while at != NONE && !set.contains(&kind[at as usize]) {
                at = parent[at as usize];
            }
            assert_eq!(cpu[k][node], at, "set {k} node {node}");
        }
    }
}

/// Where the device's nearest time goes (ignored; `--ignored --nocapture`): one chunk-sized forest (about
/// 172K nodes, as a 1-in-8 data-layer chunk), solved by the work queue, by the CPU worklist, and by one pass in
/// preorder (a parent precedes its children, so each node reads its parent's answer once).
#[test]
#[ignore]
fn nearest_costs_by_solver() {
    // Depth capped as real syntax trees are (`RERAIL_NEAREST_DEPTH`, default 40), and uncapped.
    for cap in [std::env::var("RERAIL_NEAREST_DEPTH").ok().and_then(|d| d.parse().ok()).unwrap_or(40usize), usize::MAX] {
        nearest_costs(cap);
    }
}

fn nearest_costs(cap: usize) {
    let n = 172_000usize;
    // Preorder forest: 80 documents; a node's parent is the open node on top of a stack that grows and shrinks.
    let mut parent = Vec::with_capacity(n);
    let mut kind = Vec::with_capacity(n);
    let mut stack: Vec<u32> = Vec::new();
    let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut rand = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    for i in 0..n {
        if i % (n / 80) == 0 {
            stack.clear();
        }
        // Close some open nodes (keeps depth around 20-40), then this node's parent is the top.
        while stack.len() > 1 && (rand() % 3 == 0 || stack.len() >= cap) {
            stack.pop();
        }
        parent.push(stack.last().copied().unwrap_or(NONE));
        kind.push((rand() % 300) as i64);
        stack.push(i as u32);
    }
    let depth = (0..n).map(|mut v| { let mut d = 0; while parent[v] != NONE { v = parent[v] as usize; d += 1; } d }).max().unwrap();
    let sets = vec![vec![7]];
    let trees = Trees { parent: &parent, kind: &kind };
    let sys = system(&trees, &sets).unwrap();
    let t = std::time::Instant::now();
    let cpu = solve_cpu(&sys);
    let cpu_worklist = t.elapsed();
    // Read by Metal's comparisons below (macOS); kept elsewhere so the worklist's time is the solve's.
    std::hint::black_box(&cpu);
    // One pass in preorder.
    let t = std::time::Instant::now();
    let mut one = vec![NONE; n];
    for v in 0..n {
        let p = parent[v];
        if p != NONE {
            one[v] = if sets[0].contains(&kind[p as usize]) { p } else { one[p as usize] };
        }
    }
    let one_pass = t.elapsed();
    // Both timed results held (the device's runs, macOS only, are compared with `cpu`).
    std::hint::black_box((&cpu, &one));
    println!("NEAREST {n} nodes, max depth {depth}, {} copies; CPU worklist {:.2} ms, one preorder pass {:.3} ms", sys.copies.len(), cpu_worklist.as_secs_f64() * 1e3, one_pass.as_secs_f64() * 1e3);
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    use crate::code_rule_dsl::transducer::packed::metal_nearest;
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    for how in [metal_nearest::Buffers::Fresh, metal_nearest::Buffers::Blocks, metal_nearest::Buffers::Fresh, metal_nearest::Buffers::Blocks] {
        let target: Vec<u32> = parent.clone();
        let done: Vec<u8> = parent.iter().map(|&p| u8::from(p == NONE || sets[0].contains(&kind[p as usize]))).collect();
        for round in 0..4 {
            // `RERAIL_NEAREST_IDLE_MS`: the GPU left idle before each call, as between a run's chunks.
            if let Some(ms) = std::env::var("RERAIL_NEAREST_IDLE_MS").ok().and_then(|v| v.parse().ok()) {
                std::thread::sleep(std::time::Duration::from_millis(ms));
            }
            let t = std::time::Instant::now();
            let solved = metal_nearest::solve_with(n, &target, &done, how).unwrap();
            let p: Vec<String> = solved.phases.iter().map(|s| format!("{:.3}", s * 1e3)).collect();
            println!(
                "JUMP {how:?} round {round}: host {:.2} ms, GPU {:.3} ms, {} steps at most; phases ms [made, queue, encode, submit, schedule, GPU, wake] = [{}]; {} KB made",
                t.elapsed().as_secs_f64() * 1e3,
                solved.gpu * 1e3,
                solved.rounds,
                p.join(", "),
                solved.made >> 10
            );
            assert_eq!(solved.targets, one);
        }
    }
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    for round in 0..3 {
        use crate::code_rule_dsl::transducer::packed::metal_lattice;
        let device = metal_lattice::System { seeds: &sys.seeds, copies: &sys.copies, bins: &[] };
        let t = std::time::Instant::now();
        let solved = metal_lattice::solve(&device).unwrap();
        println!(
            "METAL round {round}: host {:.2} ms, GPU {:.2} ms, wait {:.2} ms, {} launches, {} visits, {} retries",
            t.elapsed().as_secs_f64() * 1e3,
            solved.gpu * 1e3,
            solved.wall * 1e3,
            solved.launches,
            solved.visits,
            solved.retries
        );
        assert_eq!(solved.values, cpu);
    }
}

/// A helper several rules read, of one content and so one name, that the split leaves outside the chunks undoes
/// every reader's push at once. Undone at its first reader only, it was made again by the next and failed again:
/// ep2's `option(k, v), @rx(v, …)` helper, read by 352 `violation` rules, took 91 rounds of the pass, each a split of
/// half the program (4 s of a cold run's prepare).
#[test]
fn a_helper_the_split_leaves_outside_undoes_all_its_readers_at_once() {
    let mut text = String::from(r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl text(n: node, s: symbol)
.decl option(k: symbol, v: symbol)
.input node, text, option
// Any file's names: no location column, so cross-file, and so is each rule that negates it.
.decl defined(s: symbol)
defined(s) :- node(_, n, "function_definition", _, _), text(n, s).
.decl finding(n: node, v: symbol)
.output finding
"#);
    let kinds = ["call", "name", "attribute", "subscript", "lambda", "tuple"];
    for k in kinds {
        text.push_str(&format!("finding(n, v) :- node(_, n, \"{k}\", _, _), option(\"target\", v), v != \"none\", !defined(v).\n"));
    }
    let v = read(&text);
    let mut pass = PushLocalJoins::default();
    let mut p = Pipeline::new("test", v);
    p.program(&mut pass);
    let v = p.finish().unwrap().0;
    let split = split_of(&v);
    assert!(pass.made.iter().all(|m| split.local.contains(m)), "what is left made is file-local: {:?}", pass.made);
    assert!(!pass.made.iter().any(|m| rules_of(&v, m).iter().any(|r| reads(&v, *r).iter().any(|x| x == "option"))), "no helper over `option` is left");
    // Each push: a round with the helper, and one with it a level less, whatever its readers; the pass pushes twice.
    assert!(pass.rounds <= 2 * 2, "{} rounds for {} readers", pass.rounds, kinds.len());
}

/// A cross-file rule's file-local part is made per chunk: the rule reads one exported join, not its base relations.
#[test]
fn a_cross_file_rules_local_join_is_pushed_into_the_chunks() {
    let text = r#"
.decl doc(d: number, path: symbol, module: symbol)
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl field(p: node, f: symbol, c: node)
.decl text(n: node, s: symbol)
.decl op(n: node, s: symbol)
.input doc, node, field, text, op
// Any file's names: no location column, so cross-file.
.decl defined(s: symbol)
defined(s) :- node(_, n, "function_definition", _, _), field(n, "name", c), text(c, s).
.decl finding(n: node, s: symbol)
.output finding
finding(n, s) :- node(_, n, "call", _, _), field(n, "function", f), text(f, s), !op(n, "await"), s != "print", !defined(s).
"#;
    let v = read(text);
    assert!(!split_of(&v).local.contains("finding"), "finding negates a cross-file relation");
    let (v, made) = pushed(v);
    assert_eq!(made.len(), 1, "{made:?}");
    let split = split_of(&v);
    assert!(split.local.contains(&made[0]), "the made join is file-local");
    let finding = rules_of(&v, "finding");
    assert_eq!(finding.len(), 1);
    // What is left of the rule: the join and the cross-file negation.
    assert_eq!(reads(&v, finding[0]), [made[0].as_str(), "defined"]);
    let parts = programs_of(Arc::clone(&v)).unwrap();
    assert!(parts.exports.contains(&made[0]));
    assert!(!parts.base.iter().any(|b| b == "field" || b == "text" || b == "op"), "{:?}", parts.base);
    // The join takes the comparison and the negated base read along.
    let join = rules_of(&v, &made[0])[0];
    assert_eq!(join.body().count(), 5, "{}", v.show_rule(join));
    assert_eq!(join.head().args().count(), 2);
    // Its rows are attributable to their file: the node column is a location. The interface's order is the join's
    // canonical one (its literals by their text, decision 0575), so the node's column is found by its variable.
    let shown = v.show_rule(join);
    let (head, body) = shown.split_once(" :- ").expect("a rule");
    let head_vars: Vec<&str> = head[head.find('(').unwrap() + 1..head.rfind(')').unwrap()].split(", ").collect();
    let node_at = body.find("node(").expect("the join reads node");
    let node_var = body[node_at + 5..].split(", ").nth(1).expect("node's node column");
    let column = head_vars.iter().position(|x| *x == node_var).expect("the node is in the interface");
    assert_eq!(split.locations[&made[0]], [column], "{shown}: {:?}", split.locations);
}

/// A cross-file join by a value (two files' identifiers of one text, `text(a, t), text(b, t)`) is each file's part,
/// pushed apart, joined in the cross part by the value: so an edit makes the edited file's rows of it alone, and the
/// join reads the others' kept rows, no document of theirs (Q332). A test of the value alone goes with each.
#[test]
fn a_cross_file_join_by_a_value_pushes_each_files_part() {
    let text = r#"
.decl doc(d: number, path: symbol, module: symbol)
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl text(n: node, s: symbol)
.decl docof(n: node, d: number)
.input doc, node, text, docof
.decl named(t: symbol)
named(t) :- node(_, a, "identifier", _, _), text(a, t), docof(a, d), node(_, b, "identifier", _, _), text(b, t), docof(b, e), d < e, t != "self".
.output named
"#;
    let v = read(text);
    assert!(!split_of(&v).local.contains("named"), "named joins two files");
    let (v, made) = pushed(v);
    assert_eq!(made.len(), 2, "one join each file's identifiers: {made:?}");
    let split = split_of(&v);
    assert!(made.iter().all(|m| split.local.contains(m)), "each made join is file-local: {made:?}");
    let named = rules_of(&v, "named");
    assert_eq!(named.len(), 1);
    let mut read = reads(&v, named[0]);
    read.sort();
    let mut want: Vec<String> = made.clone();
    want.sort();
    assert_eq!(read, want, "the rule reads the two joins, no base relation: {}", v.show_rule(named[0]));
    for m in &made {
        let join = rules_of(&v, m)[0];
        assert_eq!(join.head().args().count(), 2, "its value and its document: {}", v.show_rule(join));
        assert!(v.show_rule(join).contains("self"), "the value's test goes with each: {}", v.show_rule(join));
    }
    let parts = programs_of(Arc::clone(&v)).unwrap();
    assert!(!parts.base.iter().any(|b| b == "text" || b == "docof"), "the cross part reads no base relation: {:?}", parts.base);
}

/// Position comparisons must retain the semantic candidate filters. Document equality alone
/// does not make their Cartesian product a bounded per-file producer.
#[test]
fn cross_file_filters_keep_independent_node_positions_out_of_local_products() {
    let text = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl field(p: node, f: symbol, c: node)
.decl text(n: node, s: symbol)
.decl docof(n: node, d: number)
.decl visit_index(n: node, i: number)
.input node, field, text, docof, visit_index
.decl defined(s: symbol)
defined(s) :- node(_, n, "function_definition", _, _), field(n, "name", c), text(c, s).
.decl chosen(n: node)
chosen(n) :- node(_, n, "call", _, _), field(n, "function", f), text(f, s), !defined(s).
.decl triangle(a: node, b: node, c: node)
triangle(a, b, c) :- chosen(a), chosen(b), chosen(c),
    docof(a, d), docof(b, d), docof(c, d), visit_index(a, ai), visit_index(b, bi), visit_index(c, ci), ai < bi, bi < ci.
.output triangle
"#;
    let v = read(text);
    assert!(!split_of(&v).local.contains("chosen"));
    let original = v.show_rule(rules_of(&v, "triangle")[0]);
    let (v, _) = pushed(v);
    let actual = v.show_rule(rules_of(&v, "triangle")[0]);
    assert_eq!(actual, original, "do not materialize all document-local node triples before chosen() filters them");
}

/// `v = v0` carries a location: a disjunction's branch (`(kind(v0, "as_pattern"), child(v0, v, 0) ; v = v0)`, as
/// lowering leaves it) keeps the head's node column a location, so its readers stay file-local. And a relation made
/// from constants alone (`side(x) :- table("k", x)`) is a constant to the split: reading it joins no document, and
/// both parts derive it.
#[test]
fn equalities_and_constant_relations_keep_rules_file_local() {
    let v = read(r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl child(p: node, c: node, i: number)
.decl text(n: node, s: symbol)
.input node, child, text
.decl table(k: symbol, x: symbol)
table("k", "a"). table("k", "b").
.decl side(x: symbol)
side(x) :- table("k", x).
.decl root(s: node, v: node)
root(s, v) :- node(_, s, "with", _, _), child(s, v0, 0), (node(_, v0, "as", _, _), child(v0, v, 0) ; !node(_, v0, "as", _, _), v = v0).
.decl under(s: node, n: node)
under(s, n) :- root(s, r), child(r, n, _).
.decl named(n: node)
named(n) :- node(_, n, "identifier", _, _), text(n, x), side(x).
.output under, named
"#);
    let split = split_of(&v);
    assert_eq!(split.locations["root"], [0, 1], "{:?}", split.locations);
    assert!(split.local.contains("under") && split.local.contains("named"), "{:?}", split.local);
    assert!(split.constants.contains("side"), "{:?}", split.constants);
    let parts = programs_of(Arc::clone(&v)).unwrap();
    let lv = &parts.versions.local;
    assert!(lv.rules().any(|r| lv.rel_name(r.head().rel()) == "side"), "the chunks derive side for named");
    assert!(!split.causes.contains_key("under"), "{:?}", split.causes);
}

/// A column filled by a node in one rule and by `unset` (-1) in another is a nullable location (decision 0649): a rule
/// that joins through it, its variable placed by a node column too (`child(a, e, 0)`), stays on one document, as
/// `conditional-architecture`'s `ca_alias_at` reads `ca_param`'s annotation. Two rows joined by that column alone may
/// both be -1, of two documents: that rule is cross-file. The column is no location, so no row's document is read
/// off it.
#[test]
fn a_location_or_unset_column_keeps_a_rule_through_it_on_one_document() {
    let v = read(r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl child(p: node, c: node, i: number)
.decl field(p: node, f: symbol, c: node)
.input node, child, field
.decl ann_of(f: node, a: loc)
ann_of(f, a) :- node(_, f, "function_definition", _, _), field(f, "return_type", a).
ann_of(f, a) :- node(_, f, "function_definition", _, _), !field(f, "return_type", _), unset(a).
.decl ann_head(f: node, e: node)
ann_head(f, e) :- ann_of(f, a), child(a, e, 0).
.decl same_ann(f: node, g: node)
same_ann(f, g) :- ann_of(f, a), ann_of(g, a), f != g.
.output ann_head, same_ann
"#);
    let split = split_of(&v);
    assert_eq!(split.locations["ann_of"], [0], "{:?}", split.locations);
    assert_eq!(split.nullable.get("ann_of").map(Vec::as_slice), Some(&[1][..]), "{:?}", split.nullable);
    assert!(split.local.contains("ann_head"), "{:?} {:?}", split.local, split.causes);
    assert!(!split.local.contains("same_ann"), "two -1s are of any documents: {:?}", split.local);
    assert_eq!(split.same_doc.get("ann_of").map(Vec::as_slice), Some(&[0][..]), "{:?}", split.same_doc);
}

/// An undeclared crossing is refused (decision 0652): a rule joining two documents by a value names its relation,
/// cause and rule. Declared `cross_file`, it is accepted, and its reader follows it with no declaration. A relation
/// the lowering made for a rule is named with that rule's relation.
#[test]
fn an_undeclared_crossing_is_refused_and_a_declared_one_is_not() {
    let base = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl text(n: node, s: symbol)
.input node, text
.decl named(n: node, s: symbol)
named(n, s) :- node(_, n, "identifier", _, _), text(n, s).
.decl twin(a: node, b: node)DECL
twin(a, b) :- named(a, s), named(b, s), a != b.
.decl has_twin(a: node)
has_twin(a) :- twin(a, _).
.output has_twin
"#;
    let refused = crossings_checked(&programs_of(read(&base.replace("DECL", ""))).unwrap()).unwrap_err();
    assert!(refused.contains("twin: joins 2 documents"), "{refused}");
    assert!(refused.contains("decision 0652") && refused.contains("cross_file"), "{refused}");
    assert!(!refused.contains("has_twin"), "a reader of a crossing is not one: {refused}");
    let declared = programs_of(read(&base.replace("DECL", " cross_file"))).unwrap();
    crossings_checked(&declared).unwrap();
    assert!(!declared.file_local.contains("twin") && !declared.file_local.contains("has_twin"), "{:?}", declared.file_local);
    let helper = read(r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl text(n: node, s: symbol)
.input node, text
.decl alone(a: node)
alone(a) :- node(_, a, "identifier", _, _), text(a, s), !(text(b, s), b != a).
.output alone
"#);
    let refused = crossings_checked(&programs_of(helper).unwrap()).unwrap_err();
    assert!(refused.contains("made by the lowering of alone") || refused.contains("  alone:"), "{refused}");
}

/// A view of values its readers cover is file-local (decision 0652): `clash(x, v, w)` is built from every file's
/// attributes, but each reader holds the two attributes it is built from, in its own document, so derived per chunk
/// it answers each reader as over every file, positive or negated. `defined(x)`, read by a rule holding only a
/// reference, is not covered: another file's definition answers it, so it is a crossing.
#[test]
fn a_view_its_readers_cover_is_file_local() {
    let base = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl attr(e: node, a: node, x: symbol, v: symbol)
.decl text(n: node, s: symbol)
.input node, attr, text
.decl solo(x: symbol)
solo("id").
.decl clash(x: symbol, v: symbol, w: symbol)
clash(x, v, w) :- attr(_, _, x, v), attr(_, _, x, w), v = w.
clash(x, v, w) :- attr(_, _, x, v), attr(_, _, x, w), solo(x).
.decl duplicate(a: node, x: symbol)
duplicate(a, x) :- attr(e, a, x, w), attr(e, b, x, v), a != b, clash(x, v, w).
.decl distinct(a: node, x: symbol)
distinct(a, x) :- attr(e, a, x, w), attr(e, b, x, v), a != b, !clash(x, v, w).
.output duplicate, distinct
"#;
    let split = split_of(&read(base));
    for rel in ["clash", "duplicate", "distinct"] {
        assert!(split.local.contains(rel), "{rel}: {:?} {:?}", split.local, split.causes);
    }
    assert!(split.filters.contains("clash"), "{:?}", split.filters);
    crossings_checked(&programs_of(read(base)).unwrap()).unwrap();
    let refused = crossings_checked(&programs_of(read(r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl text(n: node, s: symbol)
.input node, text
.decl defined(x: symbol)
defined(x) :- node(_, n, "def", _, _), text(n, x).
.decl undefined_ref(n: node)
undefined_ref(n) :- node(_, n, "ref", _, _), text(n, x), !defined(x).
.output undefined_ref
"#)).unwrap()).unwrap_err();
    assert!(refused.contains("undefined_ref"), "{refused}");
    // A located view joining two documents by a name (`pc_modattr`): each rule's reads are tied to a head location
    // column, and its reader holds both columns in its own document, so it reads only rows of its document.
    let located = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl child(p: node, c: node, i: number)
.decl text(n: node, s: symbol)
.decl imp(m: node, x: symbol)
.input node, child, text, imp
.decl modattr(m: node, o: node)
modattr(m, o) :- node(_, o, "identifier", _, _), text(o, x), imp(m, x).
.decl bare(e: node)
bare(e) :- node(d, e, "attribute", _, _), child(e, o, _), node(d, m, "module", _, _), !modattr(m, o).
.output bare
"#;
    let split = split_of(&read(located));
    assert!(split.local.contains("modattr") && split.covered.contains("modattr"), "{:?} {:?}", split.local, split.causes);
    assert!(!split.filters.contains("modattr"), "a located view is read by its locations: {:?}", split.filters);
    crossings_checked(&programs_of(read(located)).unwrap()).unwrap();
    // A reader leaving a location column to the view (`o` placed by nothing else) asks of every document's rows.
    let open = located.replace("child(e, o, _), node(d, m, \"module\", _, _), !modattr(m, o)", "node(d, m, \"module\", _, _), modattr(m, o), e = m");
    assert_ne!(open, located);
    let refused = crossings_checked(&programs_of(read(&open)).unwrap()).unwrap_err();
    assert!(refused.contains("modattr"), "{refused}");
}

/// A covered view is placed by its readers, which its own rules' digests do not hold (decision 0652): `alt` joins any
/// file's name with a parameter by text, and is covered through `param_eq`'s head, which `user` places in one function.
/// An edit removing `user` leaves `param_eq` read by no one, so `alt` is no longer covered: the split maintained across
/// it asks `alt` again (`cover_deps`), and is the whole split.
#[test]
fn a_covered_view_is_asked_again_when_a_reader_that_placed_it_goes() {
    use crate::code_rule_dsl::logic::program_image::manager::{Cx, ProgramPass, ProgramW};
    let mut text = String::from(
        r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl child(p: node, c: node, i: number)
.decl text(n: node, s: symbol)
.input node, child, text
.decl alt(n: node, k: node)
alt(n, k) :- node(_, k, "name", _, _), node(_, n, "identifier", _, _), text(k, t), text(n, t).
.decl param_eq(n: node, k: node)
param_eq(n, k) :- alt(n, k).
.decl user(f: node)
user(f) :- node(_, f, "def", _, _), child(f, n, 0), child(f, k, 1), param_eq(n, k).
.output user
"#,
    );
    // Relations the edit does not reach: the cone stays under half of them.
    for k in 0..8 {
        text.push_str(&format!(".decl other{k}(n: node)\nother{k}(n) :- node(_, n, \"k{k}\", _, _).\n.output other{k}\n"));
    }
    let mut p = Pipeline::new("test", read(&text));
    let before = p.get::<FileLocal>().unwrap();
    assert!(before.covered.contains("alt") && before.local.contains("alt"), "{:?} {:?}", before.covered, before.causes);
    struct DropUser;
    impl ProgramPass for DropUser {
        const NAME: &'static str = "drop user";
        fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
            let user = cx.v().rel("user").unwrap();
            w.keep(|r| r.head().rel() != user)
        }
    }
    p.program(&mut DropUser);
    let after = p.get::<FileLocal>().unwrap();
    assert!(!after.covered.contains("alt") && !after.local.contains("alt"), "alt has no reader placing it: {:?}", after.covered);
    let (out, _) = p.finish().unwrap();
    let fresh = Version::open(out.fold().unwrap().base_bytes().unwrap()).unwrap();
    assert_eq!(*after, *split_of(&fresh), "the maintained split is a whole split's");
}

/// A covered view of values is in `filters`, read as a test, but it is no string filter: a maintained split whose cone
/// leaves it out takes it as it was, and reads it as the whole split does (React's `jsx-no-target-blank`: `noreferrer`,
/// a test over its reader's own attribute values, made `agg`'s rows each one document's in one split and not the
/// other's).
#[test]
fn a_covered_view_of_values_outside_the_cone_is_read_as_the_whole_split_reads_it() {
    use crate::code_rule_dsl::logic::program_image::manager::{Cx, ProgramPass, ProgramW};
    let mut text = String::from(
        r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl child(p: node, c: node, i: number)
.decl text(n: node, s: symbol)
.input node, child, text
.decl val(o: node, r: node, s: symbol)
val(o, r, s) :- node(_, o, "attr", _, _), child(o, r, 0), text(r, s).
.decl nr(s: symbol)
nr(s) :- val(_, _, s), @rx(@lower(s), "^(.* )?noreferrer( .*)?$") = 1.
.decl dem(o: node, r: node)
dem(o, r) :- node(_, o, "attr", _, _), child(o, r, 0).
.decl agg(o: node, r: node)
agg(o, r) :- dem(o, r), val(o, r, s), !nr(s).
.output agg
"#,
    );
    for k in 0..8 {
        text.push_str(&format!(".decl other{k}(n: node)\nother{k}(n) :- node(_, n, \"k{k}\", _, _).\n.output other{k}\n"));
    }
    let mut p = Pipeline::new("test", read(&text));
    let before = p.get::<FileLocal>().unwrap();
    assert!(before.covered.contains("nr") && before.filters.contains("nr"), "{:?} {:?}", before.covered, before.causes);
    assert_eq!(before.row_documents.get("agg"), Some(&vec![0, 1]), "{:?}", before.row_documents);
    struct DropOther;
    impl ProgramPass for DropOther {
        const NAME: &'static str = "drop other0";
        fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
            let other = cx.v().rel("other0").unwrap();
            w.keep(|r| r.head().rel() != other)
        }
    }
    p.program(&mut DropOther);
    let after = p.get::<FileLocal>().unwrap();
    let (out, _) = p.finish().unwrap();
    let fresh = Version::open(out.fold().unwrap().base_bytes().unwrap()).unwrap();
    assert_eq!(*after, *split_of(&fresh), "the maintained split is a whole split's");
}

/// `row_documents` (decision 0652): a relation whose rules place its location columns in one document through reads
/// whose rows are each one document's, file-local or not. `pair` reads `named`, cross-file (a call's name meets any
/// file's definition) but of one location column, so `pair`'s rows are each one call's: listed, though cross-file.
/// `uses` joins two files' nodes by a name: not listed.
#[test]
fn a_relation_whose_rows_are_each_one_documents_is_listed_cross_file_or_not() {
    let split = split_of(&read(
        r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl child(p: node, c: node, i: number)
.decl text(n: node, s: symbol)
.input node, child, text
.decl named(n: node)
named(n) :- node(_, n, "call", _, _), text(n, x), node(_, m, "def", _, _), text(m, x).
.decl pair(a: node, f: node)
pair(a, f) :- node(_, c, "call", _, _), child(c, a, 0), child(c, f, 1), named(a).
.decl uses(n: node, f: node)
uses(n, f) :- node(_, n, "ref", _, _), text(n, x), node(_, f, "def", _, _), text(f, x).
.output pair, uses
"#,
    ));
    assert!(!split.local.contains("pair") && !split.local.contains("named"), "{:?}", split.causes);
    assert_eq!(split.row_documents.get("pair"), Some(&vec![0, 1]), "{:?}", split.row_documents);
    assert!(!split.row_documents.contains_key("uses"), "{:?}", split.row_documents);
}

/// A cross-file rule reading a node's position (decision 0649) is reported, with the relation it reads; one placing a
/// finding (its head an output) is not, and a file-local rule reading one is not.
#[test]
fn a_cross_file_rule_reading_a_position_is_reported() {
    let v = read(r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl visit_index(n: node, i: number)
.decl line(n: loc, l: number)
.decl text(n: node, s: symbol)
.input node, visit_index, line, text
.decl call(n: node, s: symbol)
call(n, s) :- node(_, n, "call", _, _), text(n, s).
.decl first_of(s: symbol, i: number)
first_of(s, i) :- call(n, s), call(m, s), n != m, visit_index(n, i).
.decl local_at(n: node, l: number)
local_at(n, l) :- call(n, _), line(n, l).
.decl found(n: node, l: number)
found(n, l) :- call(n, s), first_of(s, _), line(n, l).
.output found, local_at
"#);
    let parts = programs_of(Arc::clone(&v)).unwrap();
    assert!(!parts.file_local.contains("first_of"), "{:?}", parts.file_local);
    assert_eq!(parts.positions, [("first_of".to_owned(), "visit_index".to_owned())], "{:?}", parts.positions);
    let refused = positions_checked(&parts.positions, true).unwrap_err();
    assert!(refused.contains("first_of reads visit_index"), "{refused}");
    assert!(positions_checked(&parts.positions, false).is_ok());
    assert!(positions_checked(&[], true).is_ok());
}

/// A relation stated with no rows is a constant, as one with rows is: reading it, or its negation, joins no document
/// (t50 step 1). The `.references` setting's relations, which the lowering states empty when the program has no
/// setting, are such. Declared and not stated, a relation has no location, and its readers are cross-file.
#[test]
fn a_relation_stated_empty_is_a_constant() {
    let v = read(r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl doc(d: number, p: symbol, m: symbol)
.input node, doc
.decl setting_given(k: number)
.decl setting_glob(g: symbol)
.decl pydoc(d: number, p: symbol)
pydoc(d, p) :- doc(d, p, _), !setting_given(1).
pydoc(d, p) :- doc(d, p, _), setting_given(1), setting_glob(g), glob(g, p).
.decl module(d: number, m: symbol)
module(d, m) :- pydoc(d, p), doc(d, p, m).
.decl named(n: node, m: symbol)
named(n, m) :- node(d, n, "identifier", _, _), module(d, m).
.output named
"#);
    let unstated = split_of(&v);
    assert!(unstated.causes.contains_key("pydoc"), "{:?}", unstated.causes);
    let mut b = Builder::on(&v);
    b.fields().set_facts("setting_given", &[]);
    b.fields().set_facts("setting_glob", &[]);
    let (v, _) = b.finish().unwrap();
    let split = split_of(&v);
    assert!(split.local.contains("pydoc") && split.local.contains("module") && split.local.contains("named"), "{:?} {:?}", split.local, split.causes);
    assert!(split.causes.is_empty(), "{:?}", split.causes);
}

#[test]
fn the_cross_part_takes_node_attributes_as_a_semi_join_only_when_each_lookup_is_of_a_held_node() {
    let source = |extra: &str| format!(r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl child(p: node, c: node, i: number)
.decl text(n: node, s: symbol)
.decl line(n: loc, l: number)
.input node, child, text, line
.decl names(x: symbol)
names(x) :- node(_, n, "string", _, _), text(n, x).
.decl ident(n: node, x: symbol)
ident(n, x) :- node(_, n, "identifier", _, _), text(n, x).
.decl pick(n: node)
pick(n) :- ident(n, x), names(x).
.decl hit(n: node, l: number)
hit(n, l) :- pick(n), line(n, l).
{extra}
.output hit
"#);
    // As the corpus run splits: after the file-local joins are pushed into the chunks.
    let split = |text: &str| programs_of(pushed(read(text)).0).unwrap();
    let parts = split(&source(""));
    assert!(parts.base.iter().any(|b| b == "line"), "{:?}", parts.base);
    let held = semijoin_bases(&parts.versions.cross, &parts.base).expect("every lookup is of a node an export holds");
    assert!(held.contains("line"), "{held:?}");
    // `child(p, c, _)` reaches nodes no export holds: their lines would be missing. So would a cross-file rule's
    // own enumeration of nodes (`node(_, n, "identifier", _, _), text(n, x), names(x)`).
    let parts = split(&source(".decl kid(c: node, l: number)\nkid(c, l) :- pick(p), child(p, c, _), line(c, l).\n.output kid"));
    assert!(parts.base.iter().any(|b| b == "child"), "{:?}", parts.base);
    assert!(!semijoin_bases(&parts.versions.cross, &parts.base).unwrap().contains("line"));
    let parts = programs_of(read(&source(".decl own(n: node)\nown(n) :- node(_, n, \"identifier\", _, _), text(n, x), names(x).\n.decl seen(n: node, l: number)\nseen(n, l) :- own(n), line(n, l).\n.output seen"))).unwrap();
    let held = semijoin_bases(&parts.versions.cross, &parts.base).unwrap();
    assert!(!held.contains("line") && !held.contains("node") && !held.contains("text"), "{held:?}");
    // Per relation: a walk of `child` that looks up no attribute leaves `line` a semi-join for the other rules.
    let parts = split(&source(".decl kid(c: node, i: number)\nkid(c, i) :- pick(p), child(p, c, i).\n.decl kidline(p: node, l: number)\nkidline(p, l) :- kid(c, _), pick(p), child(p, c, _), line(p, l).\n.output kid, kidline"));
    assert!(parts.base.iter().any(|b| b == "child"), "{:?}", parts.base);
    let held = semijoin_bases(&parts.versions.cross, &parts.base).unwrap();
    assert!(held.contains("line"), "{held:?}");
    // A tainted output's findings are decoded through `node`: kept whole.
    assert!(!held.contains("node"), "{held:?}");
    // `tape(n, par, ..)`'s parent is a node though a number: a walk up it reaches nodes no export holds.
    let parts = split(&source(".decl tape(n: node, par: number, size: number, depth: number)\n.input tape\n.decl up(n: node, p: number)\nup(n, p) :- hit(n, _), names(_), tape(n, p, _, _).\nup(n, q) :- up(n, p), names(_), tape(p, q, _, _).\n.output up"));
    if parts.base.iter().any(|b| b == "tape") {
        assert!(!semijoin_bases(&parts.versions.cross, &parts.base).unwrap().contains("tape"));
    }
}

/// A climb up the tree (`inside`, `has`, a nearest over a derived set: lower.rs `climb`) reads the tape's parent
/// column, which `need_tape` declares over numbers. A node's parent is in its document, so the climb is file-local:
/// cross-file, every climb of a pack joined the corpus on one worker (O-cold-seed-join).
#[test]
fn a_climb_over_the_tapes_parent_column_is_file_local() {
    let text = ".decl kind(n: node, k: number)\n.input kind\n.decl tape(n: int, par: int, size: int, depth: int)\n.input tape\n\
        .decl stop(n: node)\nstop(n) :- kind(n, 7).\n\
        .decl up(n: node, a: int)\nup(n, p) :- kind(n, 3), tape(n, p, _, _), p >= 0.\nup(n, q) :- up(n, p), !stop(p), tape(p, q, _, _), q >= 0.\n\
        .decl under(n: node)\nunder(n) :- up(n, a), stop(a).\n.output under\n";
    let split = split_of(&read(text));
    assert!(split.local.contains("up") && split.local.contains("under"), "{:?}", split.local);
    assert!(split.locations["up"].contains(&0), "{:?}", split.locations);
}

/// A file-local output the cross-file part also reads (decision 0431 R9: a pack's `rr_found_through` over each
/// policy's findings) is an export, which the file-local part keeps in the engine for the cross-file part, not
/// decoded: the cross-file part writes it. Neither did, and the policy's findings' file was empty.
#[test]
fn an_exported_output_is_the_cross_file_parts_to_write() {
    let text = ".decl kind(n: node, k: number)\n.input kind\n\
        .decl hit(n: node)\nhit(n) :- kind(n, 3).\n\
        .decl hits(c: number)\nhits(c) :- c = count : { hit(_) }.\n.output hit, hits\n";
    let parts = programs_of(read(text)).unwrap();
    assert!(parts.file_local.contains("hit") && parts.exports.iter().any(|e| e == "hit"), "{:?} {:?}", parts.file_local, parts.exports);
    let cross = &parts.versions.cross;
    let outputs: Vec<&str> = cross.outputs().map(|a| cross.text(a)).collect();
    assert!(outputs.contains(&"hit") && outputs.contains(&"hits"), "{outputs:?}");
}

/// `boot-python.rerail`'s `deco_word`: a disjunction's branch `(e0 = d ; …)` leaves `e0 = d` between two per-file pieces.
/// The equality names one value, so the pieces are one join, pushed into the chunks whole; apart, the split pushed each
/// and left their join to the cross-file part (decision 0093 §4, O140).
#[test]
fn an_equality_between_two_pieces_joins_them_in_the_chunks() {
    let text = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl field(p: node, f: symbol, c: node)
.decl text(n: node, s: symbol)
.input node, field, text
.decl defined(s: symbol)
defined(s) :- node(_, n, "function_definition", _, _), field(n, "name", c), text(c, s).
.decl finding(f: node, x: symbol)
.output finding
finding(f, x) :- node(_, f, "decorated_definition", _, _), field(f, "decorator", d), e = d, field(e, "name", c), text(c, x), !defined(x).
"#;
    let (v, made) = pushed(read(text));
    let local = &split_of(&v).local;
    assert!(made.iter().all(|m| local.contains(m)), "{made:?}");
    let finding = rules_of(&v, "finding");
    assert_eq!(finding.len(), 1);
    let equal = finding[0].body().any(|l| matches!(l.view(), L::Cmp { l: TermRef::Var(_), op: CmpOp::Eq, r: TermRef::Var(_) }));
    assert!(!equal, "the equality stays in the cross-file rule: {}", v.show_rule(finding[0]));
    let pushed: Vec<String> = reads(&v, finding[0]).into_iter().filter(|a| made.contains(a)).collect();
    assert_eq!(pushed.len(), 1, "one pushed join, not one per piece: {}", v.show_rule(finding[0]));
}

/// Decision 0099, the factorized store: an export that joins two parts at a variable its head projects away
/// (`outer`'s pushed join `inside(f, p), bound(p, x)`) is handed on as its parts, each reduced by the other's keys,
/// and the cross-file reader joins them by the key where it read the product. One whose second part is a node's
/// attribute (`text(p, s)`: one row a key) is left whole: its product is no larger than its other part.
#[test]
#[ignore = "t31 A1 keeps outer's join in the cross-file part (uses, a cross-file read of the same rule, filters it), so this program has no export for decision 0099 to factorize; open: a case 0099 still factorizes"]
fn an_export_joined_at_a_projected_key_is_handed_on_as_its_parts() {
    let text = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl child(p: node, c: node, i: number)
.decl text(n: node, s: symbol)
.input node, child, text
// Any file's names: no location column, so cross-file.
.decl defined(s: symbol)
defined(s) :- node(_, n, "function_definition", _, _), text(n, s).
.decl uses(f: node, x: symbol)
uses(f, x) :- node(_, f, "identifier", _, _), text(f, x), defined(x).
.decl encl(f: node, p: node)
encl(f, p) :- node(_, p, "block", _, _), child(p, f, _).
.decl bound(p: node, x: symbol)
bound(p, x) :- node(_, p, "block", _, _), child(p, c, _), node(_, c, "assignment", _, _), text(c, x).
.decl outer(f: node, x: symbol)
.output outer
outer(f, x) :- uses(f, x), encl(f, p), bound(p, x).
.decl named(f: node, s: symbol)
.output named
named(f, s) :- uses(f, _), child(p, f, _), text(p, s).
"#;
    let (v, pushed) = pushed(read(text));
    assert_eq!(pushed.len(), 2, "{pushed:?}");
    let mut pass = FactorizeExports::new(true);
    let mut p = Pipeline::new("test", v);
    p.program(&mut pass);
    let (v, _) = p.finish().unwrap();
    let lines = pass.lines;
    assert_eq!(lines.len(), 1, "only outer's join is factorized: {lines:?}");
    let product = pushed.iter().find(|p| p.ends_with("_outer")).unwrap();
    assert!(lines[0].starts_with(&format!("{product}: handed on as parts")), "{lines:?}");
    assert!(rules_of(&v, product).is_empty(), "the product's rule is gone");
    // The reader reads the parts, joined by the key, where it read the product.
    let outer = rules_of(&v, "outer");
    assert_eq!(outer.len(), 1);
    let parts: Vec<String> = reads(&v, outer[0]).into_iter().filter(|r| r.starts_with("fz")).collect();
    assert_eq!(parts.len(), 2, "{}", v.show_rule(outer[0]));
    let keys: Vec<Option<crate::code_rule_dsl::logic::program_image::Slot>> = outer[0].body().filter_map(|l| l.atom()).filter(|a| v.rel_name(a.rel()).starts_with("fz")).map(|a| match a.args().last() { Some(TermRef::Var(s)) => Some(s), _ => None }).collect();
    assert!(keys[0].is_some(), "the key is a variable: {}", v.show_rule(outer[0]));
    assert_eq!(keys[0], keys[1], "the parts meet at the key: {}", v.show_rule(outer[0]));
    // Each part is reduced by the other's keys, and both are file-local exports.
    for part in &parts {
        let rule = rules_of(&v, part)[0];
        assert!(reads(&v, rule).iter().any(|a| a.starts_with("fzk")), "{}", v.show_rule(rule));
    }
    let split = programs_of(Arc::clone(&v)).unwrap();
    for part in &parts {
        assert!(split.exports.iter().any(|e| e == part), "{part} is exported: {:?}", split.exports);
    }
    assert!(!split.exports.iter().any(|e| e == product));
    // The other join (`named`'s) is handed on whole.
    let named = pushed.iter().find(|p| p.ends_with("_named")).unwrap();
    assert!(split.exports.iter().any(|e| e == named), "{:?}", split.exports);
}

/// A composition that a cross-file read of the same rule filters stays in the cross-file part (t31 A1): pushed, the
/// chunks would export every (function, name bound around it) pair, not the few the filter asks about.
#[test]
fn a_composition_filtered_by_a_cross_file_read_is_not_pushed() {
    let text = r#"
.decl doc(d: number, path: symbol, module: symbol)
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl field(p: node, f: symbol, c: node)
.decl text(n: node, s: symbol)
.input doc, node, field, text
// Any file's names: no location column, so cross-file; and the pairs asked about, cross-file through it.
.decl named(x: symbol)
named(x) :- node(_, n, "call", _, _), field(n, "function", c), text(c, x).
.decl wanted(f: node, x: symbol)
wanted(f, x) :- named(x), node(_, f, "function_definition", _, _).
.decl bound_in(p: node, x: symbol)
bound_in(p, x) :- field(p, "name", c), text(c, x).
.decl outer(f: node, x: symbol)
.output outer
outer(f, x) :- wanted(f, x), field(p, "body", f), bound_in(p, x).
"#;
    let (v, made) = pushed(read(text));
    assert!(!made.iter().any(|m| m.ends_with("_outer")), "{made:?}");
    let outer = rules_of(&v, "outer");
    assert_eq!(reads(&v, outer[0]), ["wanted", "field", "bound_in"]);
}

/// The split read from the version (`FileLocal`) is maintained across an edit of a few rules: computed again only
/// for the relations the edit changed and those reading them, not split whole, and the same as a whole split (a debug
/// build checks each carried analysis against a fresh one). Dropping the rule of `q` that joins two documents makes
/// `q`, and `reads_q` with it, file-local.
#[test]
fn the_split_of_a_version_is_maintained_across_an_edit() {
    use crate::code_rule_dsl::logic::program_image::manager::{Cx, ProgramPass, ProgramW};
    let mut text = String::from(
        ".decl node(d: number, n: node, kind: symbol, start: number, end: number)\n.input node\n\
         .decl q(n: node)\nq(n) :- node(_, n, \"a\", _, _).\nq(n) :- node(_, n, \"a\", _, _), node(_, m, \"b\", _, _), m != n.\n\
         .decl reads_q(n: node)\nreads_q(n) :- q(n).\n.output reads_q\n",
    );
    // Relations the edit does not reach: the cone stays under half of them.
    for k in 0..6 {
        text.push_str(&format!(".decl other{k}(n: node)\nother{k}(n) :- node(_, n, \"k{k}\", _, _).\n.output other{k}\n"));
    }
    let mut p = Pipeline::new("test", read(&text));
    let before = p.get::<FileLocal>().unwrap();
    assert!(!before.local.contains("q") && !before.local.contains("reads_q") && before.local.contains("other0"), "{:?}", before.local);
    struct DropJoin;
    impl ProgramPass for DropJoin {
        const NAME: &'static str = "drop the join";
        fn run(&mut self, _: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
            w.keep(|r| r.body().count() < 3)
        }
    }
    p.program(&mut DropJoin);
    let after = p.get::<FileLocal>().unwrap();
    assert!(after.local.contains("q") && after.local.contains("reads_q"), "{:?}", after.local);
    assert_eq!(p.store().computed("file-local split"), 0, "maintained from the input's split, not computed again");
    let (out, _) = p.finish().unwrap();
    // The same program opened afresh, with no analysis carried: its split is computed whole.
    let fresh = Version::open(out.fold().unwrap().base_bytes().unwrap()).unwrap();
    let whole = split_of(&fresh);
    assert_eq!(*after, *whole, "the maintained split is a whole split's");
}

/// Rows located on the version (`Locate`): each export with no location column derived as `<r>_at(d, …)`, its
/// document a column a rule binds (`named2`), one it leaves unnamed (`named`), or one through a node it binds
/// (`named3`, by `node`; not a lone input atom, which the cross-file part derives itself), and read back by the
/// cross-file part.
#[test]
fn rows_are_located_on_a_version() {
    let text = ".decl node(d: number, n: node, kind: symbol, start: number, end: number)\n.decl text(n: node, s: symbol)\n.input node, text\n\
                .decl named(s: symbol)\nnamed(s) :- node(_, n, \"function\", _, _), text(n, s).\n\
                .decl named2(s: symbol)\nnamed2(s) :- node(d, n, \"class\", _, _), text(n, s), d > 0.\n\
                .decl named3(s: symbol)\nnamed3(s) :- text(n, s), s != \"x\".\n\
                .decl all(s: symbol)\nall(s) :- named(s), named2(s), named3(s).\n.output all\n";
    let mut pass = Locate::default();
    let mut p = Pipeline::new("test", read(text));
    p.program(&mut pass);
    let (out, _) = p.finish().unwrap();
    assert_eq!(pass.result(), Ok(vec!["named".to_owned(), "named2".to_owned(), "named3".to_owned()]));
    for rel in ["named", "named2", "named3"] {
        let at = format!("{rel}_at");
        assert_eq!(rules_of(&out, &at).len(), 1, "{at} derived by the chunks:\n{}", out.show());
        let back = rules_of(&out, rel);
        assert!(back.len() == 1 && reads(&out, back[0]) == [at.as_str()], "{rel} read back from {at}:\n{}", out.show());
    }
}

/// A located rule keeps its branded atoms (decision 0270, a lowered program's atoms): `named`'s `node(_, n,
/// "function", _, _)`, its kind branded as lowering brands it, is the atom whose unnamed document column the located
/// rule names, its other terms copied (`passes::t_of`): the kind stays the same branded atom.
#[test]
fn a_located_rule_keeps_its_branded_atoms() {
    use crate::code_rule_dsl::logic::front::{Branded, Class};
    use crate::code_rule_dsl::logic::program_image::T;
    let kind = Branded { class: Class::Kind, atom: 7 };
    let mut b = Builder::base();
    b.fields().declare("node", &[("d", "number"), ("n", "node"), ("kind", "symbol"), ("start", "number"), ("end", "number")]);
    b.fields().declare("text", &[("n", "node"), ("s", "symbol")]);
    b.fields().declare("named", &[("s", "symbol")]);
    b.fields().declare("all", &[("s", "symbol")]);
    b.fields().set_inputs(&["node", "text"]);
    b.fields().set_outputs(&["all"]);
    b.rule().head("named", &[T::Var("s")]).atom("node", &[T::Wild, T::Var("n"), T::Branded(kind), T::Wild, T::Wild]).atom("text", &[T::Var("n"), T::Var("s")]).finish().unwrap();
    b.rule().head("all", &[T::Var("s")]).atom("named", &[T::Var("s")]).finish().unwrap();
    let (v, _) = b.finish().unwrap();
    assert!(!v.show().contains("\"function\""), "branded: {}", v.show());
    let mut pass = Locate::default();
    let mut p = Pipeline::new("test", v);
    p.program(&mut pass);
    let (out, _) = p.finish().unwrap();
    assert_eq!(pass.result(), Ok(vec!["named".to_owned()]));
    let keeps = |r: RuleRef<'_>| r.body().filter_map(|l| l.atom()).any(|a| out.rel_name(a.rel()) == "node" && a.args().any(|t| matches!(t, TermRef::Branded(b) if b == kind)));
    assert!(rules_of(&out, "named_at").into_iter().any(keeps), "the located rule keeps it:\n{}", out.show());
}

/// An export read by a cross-file rule with a demand is cut on a version: the cut's line, and the program made
/// (`xd_fl_later` through `dm_demand_0` and the key `xk_text_0`).
#[test]
fn exports_are_cut_on_a_version() {
    let text = ".decl node(d: number, n: node, kind: symbol, start: number, end: number)\n.decl text(n: node, s: symbol)\n\
                .decl within(c: node, n: node)\n.input node, text, within\n\
                .decl fl_later(n: node, c: node)\nfl_later(n, c) :- within(c, n), node(_, c, \"call\", _, _).\n\
                .decl named(n: node, s: symbol)\nnamed(n, s) :- text(n, s).\n\
                .decl demand(n: node, m: node)\ndemand(n, m) :- named(n, s), named(m, s), node(_, n, \"function\", _, _), n != m.\n\
                .decl later(n: node, c: node)\nlater(n, c) :- demand(n, _), fl_later(n, c), text(c, _).\n\
                later(n, c) :- demand(n, _), fl_later(n, c), node(_, c, \"call\", 1, _).\n.output later\n";
    let mut pass = DemandExports::default();
    let mut p = Pipeline::new("test", read(text));
    p.program(&mut pass);
    let (out, _) = p.finish().unwrap();
    assert!(pass.lines[0].starts_with("1 exports cut"), "{:?}", pass.lines);
    let shown = out.show();
    for made in ["xd_fl_later(", "dm_demand_0(", "xk_text_0("] {
        assert!(shown.contains(made), "{made} in\n{shown}");
    }
}

/// A demand whose location only a `top` superset cuts is `top` itself when what is left of its chain names every node
/// (`unwrap(n, n) :- kind(n, k), k != "paren"`, as `unparen` is): no superset of it is made and stored. The export is
/// still cut by its reader's own key (`xk_text_0`). Left with an atom that names some nodes only (`called`), the
/// superset is made.
#[test]
fn a_demand_cut_only_by_a_top_superset_is_top() {
    let program = |unwrap: &str| {
        format!(
            ".decl node(d: number, n: node, kind: symbol, start: number, end: number)\n.decl text(n: node, s: symbol)\n\
             .decl kind(n: node, k: symbol)\n.decl calls(n: node, c: node)\n\
             .decl within(c: node, n: node)\n.input node, text, kind, calls, within\n\
             .decl fl_later(n: node, c: node)\nfl_later(n, c) :- within(c, n), node(_, c, \"call\", _, _).\n\
             .decl named(n: node, s: symbol)\nnamed(n, s) :- text(n, s).\n\
             .decl unwrap(n: node, m: node)\n{unwrap}\n\
             .decl pair(n: node, m: node)\npair(n, m) :- named(n, s), named(m, s), n != m.\n\
             .decl demand(n: node, m: node)\ndemand(n, m) :- unwrap(n, x), pair(x, m).\n\
             .decl later(n: node, c: node)\nlater(n, c) :- demand(n, _), fl_later(n, c), text(c, _).\n.output later\n"
        )
    };
    let cut = |unwrap: &str| {
        let mut pass = DemandExports::default();
        let mut p = Pipeline::new("test", read(&program(unwrap)));
        p.program(&mut pass);
        let (out, _) = p.finish().unwrap();
        (out.show(), pass.lines)
    };
    let (shown, lines) = cut("unwrap(n, n) :- kind(n, k), k != \"paren\".");
    assert!(lines[0].contains("through 0 supersets (2 top of 2 explored"), "{lines:?}");
    for not in ["dm_demand_0(", "dm_pair_0("] {
        assert!(!shown.contains(not), "{not} in\n{shown}\n{lines:?}");
    }
    assert!(shown.contains("xd_fl_later($n, $c) :- fl_later($n, $c), xk_text_0($c)."), "{shown}");
    let (shown, lines) = cut("unwrap(n, n) :- calls(n, _).");
    assert!(lines[0].contains("through 1 supersets (1 top of 2 explored"), "{lines:?}");
    assert!(shown.contains("dm_demand_0("), "{shown}");
}

/// An export joining two parts at a key its head projects away is handed on as its parts on a version: one line, the
/// product gone, and the parts and their keys named from the part's content.
#[test]
fn exports_are_factorized_on_a_version() {
    let text = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl child(p: node, c: node, i: number)
.decl text(n: node, s: symbol)
.input node, child, text
.decl defined(s: symbol)
defined(s) :- node(_, n, "function_definition", _, _), text(n, s).
.decl uses(f: node, x: symbol)
uses(f, x) :- node(_, f, "identifier", _, _), text(f, x), defined(x).
.decl encl(f: node, p: node)
encl(f, p) :- node(_, p, "block", _, _), child(p, f, _).
.decl bound(p: node, x: symbol)
bound(p, x) :- node(_, p, "block", _, _), child(p, c, _), node(_, c, "assignment", _, _), text(c, x).
.decl fl_outer(f: node, x: symbol)
fl_outer(f, x) :- encl(f, p), bound(p, x).
.decl outer(f: node, x: symbol)
.output outer
outer(f, x) :- uses(f, x), fl_outer(f, x).
.decl outer2(f: node)
.output outer2
outer2(f) :- uses(f, _), fl_outer(f, y), y != "self".
"#;
    let mut pass = FactorizeExports::new(true);
    let mut p = Pipeline::new("test", read(text));
    p.program(&mut pass);
    let (out, _) = p.finish().unwrap();
    assert_eq!(pass.lines.len(), 1, "{:?}", pass.lines);
    assert!(pass.lines[0].starts_with("fl_outer: handed on as parts"), "{:?}", pass.lines);
    assert!(rules_of(&out, "fl_outer").is_empty(), "the product is gone:\n{}", out.show());
    // Each part and its key are named from the part's content: `fz<h>_` and `fzk<h>_`, one hash. The first part is
    // one atom (`encl`), so it is its own key, probed by the other part, and stores none (decision 0488): two parts,
    // one key, and the key's rule reads `encl`.
    let shown = out.show();
    let parts: std::collections::BTreeSet<&str> = shown.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|w| w.starts_with("fz") && w.ends_with("_fl_outer")).collect();
    assert_eq!(parts.len(), 3, "{parts:?}");
    let mut keyed = Vec::new();
    for p in parts.iter().filter(|p| !p.starts_with("fzk")) {
        assert!(p["fz".len()..p.len() - "_fl_outer".len()].bytes().all(|b| b.is_ascii_digit()), "{p}");
        let key = format!("fzk{}", &p["fz".len()..]);
        if parts.contains(key.as_str()) {
            keyed.push(key);
        }
    }
    assert_eq!(keyed.len(), 1, "{parts:?}");
    let key_rules: Vec<_> = shown.lines().filter(|l| l.starts_with(&format!("{}(", keyed[0]))).collect();
    assert!(!key_rules.is_empty() && key_rules.iter().all(|l| l.contains("encl(")), "{key_rules:?}\n{shown}");
}

/// The file-local joins of cross-file rules are pushed on a version: helpers named from their content
/// (`fl<h>_<head>`), each derived by the chunks and read in place of the join.
#[test]
fn local_joins_are_pushed_on_a_version() {
    let text = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl child(p: node, c: node, i: number)
.decl text(n: node, s: symbol)
.decl docof(n: node, d: number)
.decl line(n: node, l: number)
.input node, child, text, docof, line
.decl defined(s: symbol)
defined(s) :- node(_, n, "function_definition", _, _), text(n, s).
.decl uses(f: node, x: symbol)
uses(f, x) :- node(_, f, "identifier", _, _), text(f, x), defined(x).
.decl encl(f: node, p: node)
encl(f, p) :- node(_, p, "block", _, _), child(p, f, _).
.decl bound(p: node, x: symbol)
bound(p, x) :- node(_, p, "block", _, _), child(p, c, _), node(_, c, "assignment", _, _), text(c, x).
.decl outer(f: node, x: symbol)
.output outer
outer(f, x) :- uses(f, x), encl(f, p), bound(p, x).
.decl named(f: node, s: symbol)
.output named
named(f, s) :- uses(f, _), child(p, f, _), text(p, s), !bound(p, s).
.decl at(f: node, l: number)
.output at
at(f, l) :- uses(f, _), docof(f, d), line(f, l), d > 0.
"#;
    let (out, made) = pushed(read(text));
    assert!(!made.is_empty(), "{made:?}");
    let local = &split_of(&out).local;
    for m in &made {
        assert!(m.starts_with("fl") && local.contains(m), "{m}: a file-local helper, {made:?}");
        assert!(out.rules().any(|r| reads(&out, r).contains(m)), "{m} is read:\n{}", out.show());
    }
}

#[test]
fn a_local_join_can_export_a_node_bound_by_equality() {
    let text = r#"
        .decl node(d: number, n: node, k: symbol, start: number, end: number)
        .decl child(p: node, c: node, i: number)
        .decl text(n: node, x: symbol)
        .input node, child, text
        .decl excluded(n: node)
        excluded(n) :- text(n, x), text(m, x), n != m.
        .decl observed(n: node)
        .output observed
        observed(answer) :- node(_, p, "some-kind", _, _), child(p, g, 0), mid = g, answer = mid, !excluded(g).
    "#;
    // The version pass alone (0276 step 6: the tree twin it was compared with is gone). `push_equal_columns`'s
    // switch, read as it reads it (the test is compiled in crates that see only `rerail-logic`'s public items).
    let equal_columns = push_equal_columns();
    let (result, made) = pushed(read(text));
    for m in &made {
        assert!(result.rules().any(|r| reads(&result, r).contains(m)), "{m} is read:\n{}", result.show());
    }
    let helpers: Vec<_> = made.iter().filter(|name| name.ends_with("_observed")).collect();
    assert_eq!(helpers.len(), usize::from(equal_columns), "{}", result.show());
    if let [helper] = helpers.as_slice() {
        assert!(result.show().contains(&format!("observed($answer) :- {helper}(")));
    }
}

/// A helper is named from its content: an edit of one rule renames its own helper, and no other rule's (a counter
/// renumbered every helper after the edited rule's).
#[test]
fn an_edit_renames_only_its_own_rules_helpers() {
    let program = |c: i64| {
        format!(
            r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl child(p: node, c: node, i: number)
.decl text(n: node, s: symbol)
.input node, child, text
.decl defined(s: symbol)
defined(s) :- node(_, n, "function_definition", _, _), text(n, s).
.decl uses(f: node, x: symbol)
uses(f, x) :- node(_, f, "identifier", _, _), text(f, x), defined(x).
.decl first(f: node, s: symbol)
.output first
first(f, s) :- uses(f, _), child(p, f, {c}), text(p, s).
.decl named(f: node, s: symbol)
.output named
named(f, s) :- uses(f, _), child(p, f, _), node(_, p, "call", _, _), text(p, s).
"#
        )
    };
    let made = |c: i64| pushed(read(&program(c))).1;
    let (before, after) = (made(1), made(2));
    let of = |m: &[String], head: &str| m.iter().find(|r| r.ends_with(&format!("_{head}"))).cloned().unwrap_or_else(|| panic!("{head} in {m:?}"));
    assert_ne!(of(&before, "first"), of(&after, "first"), "the edited rule's helper is renamed");
    assert_eq!(of(&before, "named"), of(&after, "named"), "another rule's is not");
}

/// A base relation read before it is declared (a tree axis, `rr_inside_<set>`, which lowering declares) has no
/// location columns of its own, so its reader is cross-file for reading it: the version's split asked for its
/// columns' types and stopped, on any program with `inside(n, a, "kind")` (ep2's combine).
#[test]
fn the_split_of_a_version_reads_an_undeclared_axis() {
    let text = ".decl node(d: number, n: node, kind: symbol, start: number, end: number)\n.input node\n\
                .decl out(n: node)\n.output out\nout(n) :- node(_, n, \"call\", _, _), inside(n, a, \"assignment\").\n";
    let v = read(text);
    // The axis as the combine's link hands it on: read, and not declared.
    let axes: Vec<String> = v.rules().flat_map(|r| reads(&v, r)).filter(|a| crate::code_rule_dsl::logic::front::is_axis(a)).collect();
    assert!(!axes.is_empty(), "the program reads an axis:\n{}", v.show());
    let mut b = Builder::on(&v);
    for a in &axes {
        b.fields().undeclare(a);
    }
    let (v, _) = b.finish().unwrap();
    let split = split_of(&v);
    assert!(!split.local.contains("out"), "{:?}", split.local);
    assert!(axes.iter().any(|a| split.causes.get("out").is_some_and(|why| why.contains(a.as_str()))), "{:?}", split.causes);
}

/// A pushed join the split does not take holds back every rule that reads it in one round, not one reader a round
/// (`passes::push::Readers`). Held against its first reader only, each other reader made it again the next round: on
/// main's Python pack the two pushes (14,361 and 17,289 helpers) never ended; held against all, 3 rounds each.
#[test]
fn a_refused_join_holds_back_all_its_readers_in_one_round() {
    let mut made = Readers::default();
    // Six rules read one helper (one component's content); rule 9 reads another, taken; rule 2 reads both.
    for rule in 0..6 {
        made.read(rule, "fl1_violation".to_owned());
    }
    made.read(9, "fl2_violation".to_owned());
    made.read(2, "fl2_violation".to_owned());
    made.read(2, "fl1_violation".to_owned());
    let taken = std::collections::BTreeSet::from(["fl2_violation".to_owned()]);
    assert_eq!(made.refused(&taken), (0..6).collect(), "every reader of the refused helper, each once");
    assert!(made.refused(&std::collections::BTreeSet::from(["fl1_violation".to_owned(), "fl2_violation".to_owned()])).is_empty());
    assert_eq!(made.names(), ["fl1_violation", "fl2_violation"], "each helper once, in the order made");
}
