use rerail_blocks::vector::Vector;
use super::*;
use crate::code_rule_dsl::language::dialect_by_label;
use crate::code_rule_dsl::logic::program_image::fields::ProgramFields as _;
use crate::code_rule_dsl::logic::program_image::{Lit, TermRef};
use crate::code_rule_dsl::logic::rr::parse_version as parse;
use std::sync::Arc;

/// `v` lowered for `dialect`, `intern` giving the name atoms (`lower_version`).
fn lowered(v: &Arc<Version>, dialect: GrammarDialect, intern: &mut dyn FnMut(&[&str]) -> Result<Vector<u32>>) -> Arc<Version> {
    lower_version(v.clone(), dialect, intern).unwrap().0
}

/// The rules of `v`, as `show_rule` writes them.
fn shown(v: &Version) -> Vec<String> {
    v.rules().map(|r| v.show_rule(r)).collect()
}

const BASE: &str = ".decl node(d: number, n: node, kind: symbol, start: number, end: number)\n.decl field(p: node, f: symbol, c: node)\n\
                    .decl text(n: node, s: symbol)\n.input node, field, text\n";

/// A kind flows from `node.kind` through a derived relation into facts joined with it; text stays a name.
#[test]
fn kinds_flow_through_derived_columns_and_facts() {
    let p = parse(&format!(
        "{BASE}.decl kind(n: node, k: symbol)\n.decl lit(k: symbol, t: symbol)\n.decl typed(n: node, t: symbol)\n.output typed\n\
         kind(n, k) :- node(_, n, k, _, _).\nlit(\"integer\", \"int\").\n\
         typed(n, t) :- kind(n, k), lit(k, t), field(p, \"left\", n), text(p, \"x\").\n"
    ))
    .unwrap();
    let d = domains_of(&p).unwrap();
    assert_eq!(d["kind"], [Domain::Node, Domain::Kind]);
    assert_eq!(d["lit"], [Domain::Kind, Domain::Name]);
    assert_eq!(d["typed"], [Domain::Node, Domain::Name]);
    let python = dialect_by_label("python").unwrap();
    let mut next = 100;
    let mut interned = Vec::new();
    let l = lowered(&p, python, &mut |s| {
        interned.extend(s.iter().map(|s| (*s).to_owned()));
        Ok(s.iter().map(|_| {
            next += 1;
            next
        })
        .collect())
    });
    let tag = tape_tag(python).unwrap();
    let integer = kind_atom(tag, python.kinds().kind_id("integer"));
    let lit = l.fact_rows("lit");
    assert_eq!(lit[0][0], Value::Branded(Branded { class: Class::Kind, atom: integer }));
    assert!(matches!(lit[0][1], Value::Branded(Branded { class: Class::Name, atom }) if atom > 100));
    let typed = l.rules().find(|r| l.rel_name(r.head().rel()) == "typed").unwrap();
    let Lit::Atom(field) = typed.body().nth(2).unwrap().view() else { panic!() };
    let left = field_atom(tag, python.kinds().field_id("left").unwrap());
    assert!(matches!(field.args().nth(1), Some(TermRef::Branded(Branded { class: Class::Field, atom })) if atom == left));
    assert!(interned.contains(&"x".to_owned()) && interned.contains(&"+".to_owned()));
}

/// A variable that is a kind in one atom and a name in another cannot be lowered.
#[test]
fn a_variable_in_two_symbol_domains_is_refused() {
    let p = parse(&format!("{BASE}.decl bad(n: node)\n.output bad\nbad(n) :- node(_, n, k, _, _), text(n, k).\n")).unwrap();
    let e = domains_of(&p).expect_err("a domain conflict").to_string();
    assert!(e.contains("$k is a Kind and a Name") || e.contains("$k is a Name and a Kind"), "{e}");
}

/// A disjunction over a kind variable, `(k = "call" ; k = "attribute")`, a membership test, compares kinds: a rule
/// per kind, each `$k` equal to its kind atom; negated, `$k` differs from each (it was "$k is a Kind and a Name").
#[test]
fn a_kind_membership_compares_kind_atoms() {
    let p = parse(&format!(
        "{BASE}.decl r(n: node)\n.decl s(n: node)\n.output r, s\n\
         r(n) :- node(_, n, k, _, _), (k = \"call\" ; k = \"attribute\").\n\
         s(n) :- node(_, n, k, _, _), !(k = \"call\" ; k = \"attribute\").\n"
    ))
    .unwrap();
    let python = dialect_by_label("python").unwrap();
    let mut next = 100;
    let l = lowered(&p, python, &mut |s| {
        Ok(s.iter()
            .map(|_| {
                next += 1;
                next
            })
            .collect())
    });
    let tag = tape_tag(python).unwrap();
    let atom = |k: &str| kind_atom(tag, python.kinds().kind_id(k));
    let cmp = |op: &str, k: &str| (op.to_owned(), atom(k));
    // Each rule's comparisons after its `node` atom: the operator and the kind atom `$k` is compared with.
    let bodies = |rel: &str| {
        l.rules()
            .filter(|r| l.rel_name(r.head().rel()) == rel)
            .map(|r| {
                r.body()
                    .skip(1)
                    .map(|x| match x.view() {
                        Lit::Cmp { l: TermRef::Var(s), op, r: TermRef::Branded(b) } if l.text(r.var(s)) == "k" && b.class == Class::Kind => (op.as_str().to_owned(), b.atom),
                        _ => panic!("a comparison of $k with a kind: {}", l.show_lit(r, x)),
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(bodies("r"), [vec![cmp("=", "call")], vec![cmp("=", "attribute")]]);
    assert_eq!(bodies("s"), [vec![cmp("!=", "call"), cmp("!=", "attribute")]]);
}

/// Lowered for two stores that number their atoms apart, a program differs only in its branded names, and moving the
/// first's to the second's atoms (`relocate`, decision 0270) gives the second: every name is found, kinds, fields and
/// integers stay. Its image keeps each atom's brand.
#[test]
fn a_program_lowered_for_one_store_moves_to_another() {
    let p = parse(&format!(
        "{BASE}.decl kind(n: node, k: symbol)\n.decl lit(k: symbol, t: symbol)\n.decl typed(n: node, t: symbol, i: number)\n.output typed\n\
         kind(n, k) :- node(_, n, k, _, _).\nlit(\"integer\", \"int\").\n\
         typed(n, t, 3) :- kind(n, k), lit(k, t), field(p, \"left\", n), text(p, \"x\"), t != \"str\".\n"
    ))
    .unwrap();
    let python = dialect_by_label("python").unwrap();
    let lower = |base: u32| {
        let mut names = Vec::new();
        let l = lowered(&p, python, &mut |s| {
            names.extend(s.iter().map(|s| (*s).to_owned()));
            Ok((0..s.len() as u32).map(|i| base + 3 * i).collect())
        });
        (l, names)
    };
    let (here, names) = lower(7);
    let (there, again) = lower(1000);
    assert_eq!(names, again);
    assert_ne!(shown(&here), shown(&there));
    let to: HashMap<u32, u32> = (0..names.len() as u32).map(|i| (7 + 3 * i, 1000 + 3 * i)).collect();
    let moved = relocate(&here, &to).unwrap().expect("every name has an atom there");
    assert_eq!(shown(&moved), shown(&there));
    assert_eq!(moved.fact_rows("lit"), there.fact_rows("lit"));
    // A name no intern call made is not moved: the caller prepares again.
    assert!(relocate(&here, &HashMap::default()).unwrap().is_none());
    // The image keeps the brands: opened again, the program is the one written.
    let folded = moved.fold().unwrap();
    let back = Version::open(folded.base_bytes().unwrap()).unwrap();
    assert_eq!(shown(&back), shown(&there));
    assert_eq!(back.fact_rows("lit"), there.fact_rows("lit"));
}

/// Outside the run (a memo entry, a session's frames) a column of kind or field atoms is no store atom: they are
/// grammar codes (`field.f` is `tag << 16 | code`), and mapped as atoms between processes a field atom past the
/// store's end was read as another process's atom, inside the waves.
#[test]
fn kind_and_field_columns_are_not_stored_as_atoms() {
    let p = parse(&format!(
        "{BASE}.decl kind(n: node, k: symbol)\n.decl named(n: node, f: symbol, s: symbol)\n.output named\n\
         kind(n, k) :- node(_, n, k, _, _).\nnamed(c, f, s) :- field(p, f, c), kind(p, \"call\"), text(c, s).\n"
    ))
    .unwrap();
    let d = domains_of(&p).unwrap();
    let atom_columns = |p: &Version, d: &Domains, rel: &str| {
        let cols = p.columns_of(rel).unwrap_or_default();
        let types: Vec<&str> = cols.iter().map(|(_, t)| t.as_str()).collect();
        super::atom_columns(&types, d, rel)
    };
    assert_eq!(atom_columns(&p, &d, "field"), [false, false, false]);
    assert_eq!(atom_columns(&p, &d, "node"), [false; 5]);
    assert_eq!(atom_columns(&p, &d, "text"), [false, true]);
    assert_eq!(atom_columns(&p, &d, "kind"), [false, false], "a derived kind column");
    assert_eq!(atom_columns(&p, &d, "named"), [false, false, true], "a derived field column, and a name");
}
