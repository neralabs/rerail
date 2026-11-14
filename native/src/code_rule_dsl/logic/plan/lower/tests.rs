use super::*;
use crate::code_rule_dsl::language::dialect_by_label;
use crate::code_rule_dsl::logic::rr::parse;

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
    let d = domains(&p).unwrap();
    assert_eq!(d["kind"], [Domain::Node, Domain::Kind]);
    assert_eq!(d["lit"], [Domain::Kind, Domain::Name]);
    assert_eq!(d["typed"], [Domain::Node, Domain::Name]);
    let python = dialect_by_label("python").unwrap();
    let mut next = 100;
    let mut interned = Vec::new();
    let l = lower(&p, python, &mut |s| {
        interned.extend(s.iter().map(|s| (*s).to_owned()));
        Ok(s.iter().map(|_| {
            next += 1;
            next
        })
        .collect())
    })
    .unwrap();
    let tag = tape_tag(python).unwrap();
    let integer = i64::from(kind_atom(tag, python.kinds().kind_id("integer")));
    assert_eq!(l.program.facts["lit"][0][0], Value::Int(integer));
    assert!(matches!(l.program.facts["lit"][0][1], Value::Int(n) if n > 100));
    let body = &l.program.rules.iter().find(|r| r.head.rel == "typed").unwrap().body;
    let Literal::Atom(field) = &body[2] else { panic!() };
    let left = i64::from(field_atom(tag, python.kinds().field_id("left").unwrap()));
    assert_eq!(field.args[1], Term::Int(left));
    assert!(interned.contains(&"x".to_owned()) && interned.contains(&"+".to_owned()));
}

/// A variable that is a kind in one atom and a name in another cannot be lowered.
#[test]
fn a_variable_in_two_symbol_domains_is_refused() {
    let p = parse(&format!("{BASE}.decl bad(n: node)\n.output bad\nbad(n) :- node(_, n, k, _, _), text(n, k).\n")).unwrap();
    let e = domains(&p).err().expect("a domain conflict").to_string();
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
    let l = lower(&p, python, &mut |s| {
        Ok(s.iter()
            .map(|_| {
                next += 1;
                next
            })
            .collect())
    })
    .unwrap();
    let tag = tape_tag(python).unwrap();
    let atom = |k: &str| Term::Int(i64::from(kind_atom(tag, python.kinds().kind_id(k))));
    let cmp = |op, k: &str| Literal::Cmp { l: Term::Var("k".into()), op, r: atom(k) };
    let bodies = |rel: &str| l.program.rules.iter().filter(|r| r.head.rel == rel).map(|r| r.body[1..].to_vec()).collect::<Vec<_>>();
    assert_eq!(bodies("r"), [vec![cmp("=", "call")], vec![cmp("=", "attribute")]]);
    assert_eq!(bodies("s"), [vec![cmp("!=", "call"), cmp("!=", "attribute")]]);
}
