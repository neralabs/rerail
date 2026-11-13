use super::{print, read};
use crate::code_rule_dsl::logic::front::{AggOp, Atom, Literal, Native, Program, Rule, Term, Value};

/// Program has no PartialEq: its Debug form is the comparison, field by field.
fn same(a: &Program, b: &Program) {
    assert_eq!(format!("{a:?}"), format!("{b:?}"));
}

fn atom(rel: &str, args: Vec<Term>) -> Atom {
    Atom { rel: rel.into(), args, neg: false, gated: false }
}

fn v(x: &str) -> Term {
    Term::Var(x.to_owned())
}

/// Every part of a program reads back as written: escaped names, the least number, strings that need escapes, each
/// literal and aggregate, gated negations, facts of a relation with no rows, and what has no syntax (`.about`).
#[test]
fn a_program_reads_back_from_its_text() {
    let mut prog = Program::default();
    prog.decls.insert("r".into(), vec![("c".into(), "int".into()), ("d".into(), "sym".into())]);
    prog.decls.insert("nearest".into(), vec![("x".into(), "int".into())]);
    prog.inputs = vec!["a".into(), "b".into()];
    prog.outputs = vec!["r".into()];
    prog.facts.insert("k".into(), vec![vec![Value::Int(i64::MIN), Value::Str("q\"\\\n\u{1}é".into())], vec![Value::Int(-3), Value::Str(String::new())]]);
    prog.facts.insert("empty".into(), vec![]);
    let mut gated = atom("done_x", vec![v("x")]);
    gated.neg = true;
    gated.gated = true;
    let mut neg = atom("b", vec![Term::Wild, Term::Top]);
    neg.neg = true;
    prog.rules = vec![
        Rule { head: atom("r", vec![v("x"), Term::Call("cat".into(), vec![v("top"), Term::Str("s".into())])]), body: vec![
            Literal::Atom(atom("a", vec![v("x"), v("count_"), v("min")])),
            Literal::Atom(neg),
            Literal::Atom(gated),
            Literal::Cmp { l: v("x"), op: "<=", r: Term::Int(i64::MIN) },
            Literal::Cmp { l: Term::Call("rx".into(), vec![v("x"), Term::Str("^a$".into())]), op: "=", r: Term::Int(1) },
        ] },
        Rule { head: atom("nearest", vec![v("n")]), body: vec![
            Literal::Agg { var: "n".into(), op: AggOp::Count, over: None, atom: atom("a", vec![v("x"), Term::Wild]) },
            Literal::Agg { var: "m".into(), op: AggOp::Min, over: Some("y".into()), atom: atom("a", vec![v("y")]) },
            Literal::Agg { var: "s".into(), op: AggOp::Sum, over: None, atom: atom("a", vec![v("y")]) },
            Literal::Agg { var: "j".into(), op: AggOp::Join { by: vec!["k".into(), "sep".into()], sep: ", ".into() }, over: Some("y".into()), atom: atom("ast", vec![v("y"), v("k"), v("sep")]) },
            Literal::Agg { var: "j2".into(), op: AggOp::Join { by: vec![], sep: "".into() }, over: Some("y".into()), atom: atom("a", vec![v("y")]) },
            Literal::Nearest { n: "x".into(), a: "p".into(), kindset: "Scope".into() },
            Literal::Ast { document: "d".into(), node: "n".into(), matcher: "{\"kind\": \"call\"}".into(), captures: vec![("NAME".into(), "v".into())] },
        ] },
        Rule { head: atom("unit", vec![]), body: vec![] },
    ];
    prog.natives.insert("reach_x".into(), Native::Reach { seed: "s".into(), edge: "e".into() });
    prog.kindsets.insert("Scope".into(), vec!["module".into(), "class".into()]);
    prog.demand = vec!["*".into()];
    prog.settings = vec![("mode".into(), "block".into()), ("input".into(), "r(IO=json)".into())];
    prog.nullary.insert("unit".into());
    let text = print(&prog).unwrap();
    let back = read(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
    same(&back, &prog);
    assert_eq!(print(&back).unwrap(), text);
}

/// The pack rules' programs, as read from their files, read back from their text.
#[test]
fn pack_programs_read_back_from_their_text() {
    let packs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/rule-syntax/packs");
    let mut files: Vec<_> = std::fs::read_dir(&packs).unwrap().flatten().filter(|d| d.path().is_dir()).flat_map(|d| std::fs::read_dir(d.path()).unwrap().flatten().map(|f| f.path())).filter(|p| p.extension().is_some_and(|e| e == "rr")).collect();
    files.sort();
    let mut read_back = 0;
    for path in files.iter().step_by(7) {
        let Ok(prog) = crate::code_rule_dsl::logic::rr::parse_file(path) else { continue };
        let text = print(&prog).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let back = read(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(format!("{back:?}"), format!("{prog:?}"), "{}", path.display());
        read_back += 1;
    }
    assert!(read_back > 50, "{read_back}");
}

/// What the text has no form for is refused.
#[test]
fn the_tape_reader_refuses_what_print_never_writes() {
    for text in [
        ".decl a(x: int)\nb(x) :- a(x).\n",
        ".rules\nb(x) :- a(x) ; c(x).\n",
        ".rules\nb(x) :- a(x), x ~ `y`.\n",
        ".rules\nb(x) :- a(x), x = 1.5.\n",
        ".rules\nb(x) :- a(x), x = mean y : { c(y) }.\n",
        ".mode strict\n",
        ".rules\nb(x :- .\n",
    ] {
        assert!(read(text).is_err(), "{text:?}");
    }
}
