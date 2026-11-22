//! Contexts' tests (decision 0597, step 1): a context's instance of its rules, the reads between passes, and what is
//! refused.
use crate::code_rule_dsl::logic::program_image::Version;
use crate::code_rule_dsl::logic::rr::parse_version;

/// A host that derives names and the roots of its annotations, and an annotation context whose names are the host's
/// bound ones: `name_node` is the library's, held by both.
const PROGRAM: &str = "\
.language python
.decl kind(n: node, k: symbol)
.decl text(n: node, s: symbol)
.input kind, text
.decl name_node(n: node)
name_node(n) :- kind(n, \"identifier\").
.decl bound(x: symbol)
bound(x) :- name_node(n), text(n, x).
.decl ann_root(r: node)
ann_root(r) :- kind(r, \"annotation\").
.decl ann_names(x: symbol)
ann_names(x) :- name_node(n), text(n, x), bound(x).
.decl used(x: symbol)
used(x) :- ann_names(x).
.output used, bound
.context annotation of python roots ann_root exports ann_names
.from python bound for annotation
.from annotation ann_names
";

fn rules_of(v: &Version, rel: &str) -> Vec<String> {
    v.rules().filter(|r| v.rel_name(r.head().rel()) == rel).map(|r| v.show_rule(r)).collect()
}

fn refusal(text: &str) -> String {
    match parse_version(text) {
        Ok(v) => panic!("refused, but read as {}", v.show()),
        Err(e) => e.to_string(),
    }
}

/// A context holds its own instance of each relation its exports reach (`rr_cx_annotation_name_node`, the library
/// rule as written), reads the host's relation it names `.from` (`bound`), and the host reads the context's export
/// through its `.from` (`used`). The host keeps the relations it reaches itself; one only the context reaches
/// (`ann_names`) is the context's alone.
#[test]
fn a_context_holds_its_own_instance_of_the_rules_its_exports_reach() {
    let v = parse_version(PROGRAM).unwrap();
    let all = v.show();
    let name = rules_of(&v, "rr_cx_annotation_name_node");
    assert!(name.len() == 1 && name[0].contains("kind($n, \"identifier\")"), "{all}");
    let names = rules_of(&v, "rr_cx_annotation_ann_names");
    assert!(names.len() == 1 && names[0].contains("rr_cx_annotation_name_node(") && names[0].contains(" bound(") && !names[0].contains("rr_cx_annotation_bound"), "{all}");
    assert!(rules_of(&v, "name_node").len() == 1 && rules_of(&v, "bound").iter().all(|r| r.contains(" name_node(")), "the host's own: {all}");
    assert!(rules_of(&v, "used").iter().all(|r| r.contains("rr_cx_annotation_ann_names(")), "{all}");
    assert!(rules_of(&v, "ann_names").is_empty() && !v.declares("ann_names"), "the context's alone: {all}");
    assert!(v.declares("rr_cx_annotation_ann_names") && v.declares("rr_cx_annotation_name_node"), "{all}");
    assert!(rules_of(&v, "rr_cx_annotation_bound").is_empty(), "read from the host, not made again: {all}");
}

/// A read under a name of the reader's own (`as`).
#[test]
fn a_pass_reads_another_under_a_name_of_its_own() {
    let text = PROGRAM.replace("used(x) :- ann_names(x).", "used(x) :- quoted(x).").replace(".from annotation ann_names", ".from annotation ann_names as quoted");
    let v = parse_version(&text).unwrap();
    assert!(rules_of(&v, "used").iter().all(|r| r.contains("rr_cx_annotation_ann_names(")), "{}", v.show());
}

/// A program that declares no pass is read as it is.
#[test]
fn a_program_without_passes_is_itself() {
    let text: String = PROGRAM.lines().filter(|l| !l.starts_with(".context") && !l.starts_with(".from")).map(|l| format!("{l}\n")).collect();
    let v = parse_version(&text).unwrap();
    assert!(rules_of(&v, "ann_names").len() == 1 && v.rules().all(|r| !v.rel_name(r.head().rel()).starts_with(super::PREFIX)), "{}", v.show());
}

/// A pass reads of a context only what it exports.
#[test]
fn a_read_of_what_a_context_does_not_export_is_refused() {
    let e = refusal(&PROGRAM.replace(".from annotation ann_names", ".from annotation ann_names, name_node"));
    assert!(e.contains("annotation does not export name_node"), "{e}");
}

/// A cycle through two passes is refused, naming both and its relations: the host's `used` reads the annotations'
/// names, which read `used`.
#[test]
fn a_cycle_through_two_passes_is_refused_naming_it() {
    let text = PROGRAM.replace(".from python bound for annotation", ".from python bound, used for annotation").replace("ann_names(x) :- name_node(n), text(n, x), bound(x).", "ann_names(x) :- name_node(n), text(n, x), bound(x).\nann_names(x) :- used(x).");
    let e = refusal(&text);
    assert!(e.contains("a cycle through the passes annotation and python") && e.contains("rr_cx_annotation_ann_names") && e.contains("used"), "{e}");
}

/// What a declaration names must be there: a context's host and roots, a read's passes.
#[test]
fn a_context_or_read_naming_what_is_not_there_is_refused() {
    let e = refusal(&PROGRAM.replace("roots ann_root", "roots no_roots"));
    assert!(e.contains("its roots no_roots are not declared"), "{e}");
    let e = refusal(&PROGRAM.replace(".context annotation of python", ".context annotation of vue"));
    assert!(e.contains("its host vue is no pass"), "{e}");
    let e = refusal(&PROGRAM.replace(".from annotation ann_names", ".from format_spec ann_names"));
    assert!(e.contains("format_spec is no pass"), "{e}");
    let e = refusal(&PROGRAM.replace("exports ann_names", "exports ann_names, nothing"));
    assert!(e.contains("nothing: no rule derives it"), "{e}");
}

/// 0597 step 2, rung 2.1: the passes are made once. Their program, printed and read again, is itself: what is made
/// holds `.pass` settings (each context, its host and its roots), and no `.context` or `.from` to make again.
#[test]
fn the_passes_printed_and_read_again_are_themselves() {
    let v = parse_version(PROGRAM).unwrap();
    let settings = crate::code_rule_dsl::logic::program_image::fields::ProgramFields::settings(&*v);
    assert!(!settings.iter().any(|(k, _)| k == "context" || k == "from"), "{settings:?}");
    assert!(settings.iter().any(|(k, t)| k == "pass" && t.split_whitespace().collect::<Vec<_>>() == ["annotation", "of", "python", "roots", "ann_root", "exports", "ann_names", "reads", "bound"]), "{settings:?}");
    let printed = crate::code_rule_dsl::logic::rr::print_version(&v).unwrap();
    let again = parse_version(&printed).unwrap();
    assert!(crate::code_rule_dsl::logic::program_image::same_program(&v, &again), "{}", crate::code_rule_dsl::logic::program_image::differences(&v, &again));
    assert_eq!(super::made(&again), super::made(&v));
    assert_eq!(super::made(&v).iter().map(|p| (p.name.as_str(), p.host.as_str(), p.roots.as_str())).collect::<Vec<_>>(), [("annotation", "python", "ann_root")]);
}

/// A read of a pass made already resolves to its instance: a pack's policy reads the library's context, made when
/// the library was (`.pass`), and two policies may read the same relation under the same name.
#[test]
fn a_read_of_a_pass_made_already_reads_its_instance() {
    let made = crate::code_rule_dsl::logic::rr::print_version(&parse_version(PROGRAM).unwrap()).unwrap();
    let text = format!("{made}\n.decl late(x: symbol)\nlate(x) :- names(x).\n.output late\n.from annotation ann_names as names\n.from annotation ann_names as names\n");
    let v = parse_version(&text).unwrap();
    assert!(rules_of(&v, "late").iter().all(|r| r.contains("rr_cx_annotation_ann_names(")), "{}", v.show());
    let e = refusal(&format!("{made}\n.from python bound for annotation\n"));
    assert!(e.contains("annotation is made already"), "{e}");
}

/// 0597 step 3, rung 3.1: a context is declared where its exports are written (the library's files each add theirs):
/// two declarations of one context, of the same host and roots, are one context exporting both's. One of another
/// host or roots is refused.
#[test]
fn a_context_declared_twice_exports_both_declarations() {
    let text = PROGRAM.replace(
        ".context annotation of python roots ann_root exports ann_names",
        ".decl ann_bound(x: symbol)\nann_bound(x) :- bound(x).\n.context annotation of python roots ann_root exports ann_names\n.context annotation of python roots ann_root exports ann_bound",
    ) + ".from annotation ann_bound as late\n.decl later(x: symbol)\nlater(x) :- late(x).\n.output later\n";
    let v = parse_version(&text).unwrap();
    assert!(rules_of(&v, "later").iter().all(|r| r.contains("rr_cx_annotation_ann_bound(")), "{}", v.show());
    let settings = crate::code_rule_dsl::logic::program_image::fields::ProgramFields::settings(&*v);
    assert_eq!(settings.iter().filter(|(k, _)| k == "pass").count(), 1, "{settings:?}");
    let e = refusal(&PROGRAM.replace(".context annotation of python roots ann_root exports ann_names", ".context annotation of python roots ann_root exports ann_names\n.context annotation of python roots name_node exports ann_names"));
    assert!(e.contains("annotation: declared twice, of other roots"), "{e}");
}

/// 0597 step 3, rung 3.2: a guest's view of its host (`host_node(n, h)`, `guest_of(n, s)`) names a host's node in its
/// second column: a context's rule reading it binds its own node to its nodes, not the host's.
#[test]
fn a_contexts_read_of_its_hosts_node_binds_only_its_own() {
    let text = PROGRAM.replace(".input kind, text", ".decl host_node(n: node, h: node)\n.input kind, text, host_node").replace("ann_names(x) :- name_node(n), text(n, x), bound(x).", "ann_names(x) :- name_node(n), host_node(n, h), text(n, x), bound(x).");
    let v = parse_version(&text).unwrap();
    let names = rules_of(&v, "rr_cx_annotation_ann_names");
    assert!(names.len() == 1 && names[0].contains("rr_cx_annotation_rr_nodes($n)") && !names[0].contains("rr_cx_annotation_rr_nodes($h)"), "{}", v.show());
}

/// 0597 step 3, rung 3.3: a host relation that counts the context's rows says so in a rule of its own (rule 5), reading
/// the context's export of the same relation: `name_node` holds the host's names and the annotations'. That rule is
/// the host's: the context's instance of `name_node` is its own rule over its nodes, not a read of itself.
#[test]
fn a_hosts_rule_reading_a_contexts_export_is_no_rule_of_the_context() {
    let text = PROGRAM.replace("exports ann_names", "exports ann_names, name_node").replace("name_node(n) :- kind(n, \"identifier\").", "name_node(n) :- kind(n, \"identifier\").\nname_node(n) :- guest_names(n).") + ".from annotation name_node as guest_names\n";
    let v = parse_version(&text).unwrap();
    let inner = rules_of(&v, "rr_cx_annotation_name_node");
    assert!(inner.len() == 1 && inner[0].contains("kind($n, \"identifier\")"), "{}", v.show());
    let host = rules_of(&v, "name_node");
    assert!(host.len() == 2 && host.iter().any(|r| r.contains("rr_cx_annotation_name_node($n)")), "{}", v.show());
}

/// Two exports, one read by the other's rule: the host reads both by their own names (`.from annotation ann_in`), so
/// the rule reading one is the context's, and its instance reads the context's own (python/ast's `ann_dotted`).
#[test]
fn a_contexts_rule_reading_an_export_by_its_own_name_is_the_contexts() {
    let text = PROGRAM.replace("exports ann_names", "exports ann_names, ann_short").replace(".output used, bound", ".decl ann_short(x: symbol)\nann_short(x) :- ann_names(x), !bound(x).\n.decl short(x: symbol)\nshort(x) :- ann_short(x).\n.output used, bound, short") + ".from annotation ann_short\n";
    let v = parse_version(&text).unwrap();
    let inner = rules_of(&v, "rr_cx_annotation_ann_short");
    assert!(inner.len() == 1 && inner[0].contains("rr_cx_annotation_ann_names("), "{}", v.show());
    assert!(rules_of(&v, "short").iter().all(|r| r.contains("rr_cx_annotation_ann_short(")), "{}", v.show());
}

/// A program of no `.language` (a test's, a typing phase's) is of the language its library is written for: the host
/// its contexts name is its pass, and its reads are that pass's.
#[test]
fn a_program_of_no_language_is_its_librarys_host() {
    let v = parse_version(&PROGRAM.replace(".language python\n", "")).unwrap();
    assert!(rules_of(&v, "used").iter().all(|r| r.contains("rr_cx_annotation_ann_names(")), "{}", v.show());
}
