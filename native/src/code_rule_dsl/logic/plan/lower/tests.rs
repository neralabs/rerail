use super::*;
use crate::code_rule_dsl::language::dialect_by_label;
use crate::code_rule_dsl::logic::front::parse;

const BASE: &str = ".decl node(d: int, n: node, kind: sym, start: int, end: int)\n.decl field(p: node, f: sym, c: node)\n\
                    .decl text(n: node, s: sym)\n.input node, field, text\n";

/// A kind flows from `node.kind` through a derived relation into facts joined with it; text stays a name.
#[test]
fn kinds_flow_through_derived_columns_and_facts() {
    let p = parse(&format!(
        "{BASE}.decl kind(n: node, k: sym)\n.decl lit(k: sym, t: sym)\n.decl typed(n: node, t: sym)\n.output typed\n\
         kind($n, $k) :- node(_, $n, $k, _, _).\nlit(\"integer\", \"int\").\n\
         typed($n, $t) :- kind($n, $k), lit($k, $t), field($p, \"left\", $n), text($p, \"x\").\n"
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
    let p = parse(&format!("{BASE}.decl bad(n: node)\n.output bad\nbad($n) :- node(_, $n, $k, _, _), text($n, $k).\n")).unwrap();
    let e = domains(&p).err().expect("a domain conflict").to_string();
    assert!(e.contains("$k is a Kind and a Name") || e.contains("$k is a Name and a Kind"), "{e}");
}
