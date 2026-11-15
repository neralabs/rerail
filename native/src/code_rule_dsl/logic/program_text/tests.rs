use super::{print, read};
use crate::code_rule_dsl::logic::front::{AggOp, Atom, Literal, Native, Program, Rule, Term, Value};

/// Program has no PartialEq: its Debug form is the comparison, field by field.
fn same(a: &Program, b: &Program) {
    assert_eq!(format!("{a:?}"), format!("{b:?}"));
}

fn atom(rel: &str, args: Vec<Term>) -> Atom {
    Atom { rel: rel.into(), args: args.into(), neg: false, gated: false }
}

fn v(x: &str) -> Term {
    Term::Var(x.to_owned().into())
}

/// Every part of a program reads back as written: escaped names, the least number, strings that need escapes, each
/// literal and aggregate, gated negations, facts of a relation with no rows, and what has no syntax (`.about`).
#[test]
fn a_program_reads_back_from_its_text() {
    let mut prog = Program::default();
    prog.decls.insert("r".into(), [("c".into(), "int".into()), ("d".into(), "sym".into())].into());
    prog.decls.insert("nearest".into(), [("x".into(), "int".into())].into());
    prog.inputs = vec!["a".into(), "b".into()];
    prog.outputs = vec!["r".into()];
    prog.facts.insert("k".into(), vec![[Value::Int(i64::MIN), Value::Str("q\"\\\n\u{1}é".into())].into(), [Value::Int(-3), Value::Str(String::new().into())].into()]);
    prog.facts.insert("empty".into(), vec![]);
    let mut gated = atom("done_x", vec![v("x")]);
    gated.neg = true;
    gated.gated = true;
    let mut neg = atom("b", vec![Term::Wild, Term::Top]);
    neg.neg = true;
    prog.rules = vec![
        Rule { head: atom("r", vec![v("x"), Term::Call("cat".into(), [v("top"), Term::Str("s".into())].into())]), body: vec![
            Literal::Atom(atom("a", vec![v("x"), v("count_"), v("min")])),
            Literal::Atom(neg),
            Literal::Atom(gated),
            Literal::Cmp { l: v("x"), op: "<=", r: Term::Int(i64::MIN) },
            Literal::Cmp { l: Term::Call("rx".into(), [v("x"), Term::Str("^a$".into())].into()), op: "=", r: Term::Int(1) },
        ].into() },
        Rule { head: atom("nearest", vec![v("n")]), body: vec![
            Literal::Agg { var: "n".into(), op: AggOp::Count, over: None, atom: atom("a", vec![v("x"), Term::Wild]) },
            Literal::Agg { var: "m".into(), op: AggOp::Min, over: Some("y".into()), atom: atom("a", vec![v("y")]) },
            Literal::Agg { var: "s".into(), op: AggOp::Sum, over: None, atom: atom("a", vec![v("y")]) },
            Literal::Agg { var: "j".into(), op: AggOp::Join { by: vec!["k".into(), "sep".into()], sep: ", ".into() }, over: Some("y".into()), atom: atom("ast", vec![v("y"), v("k"), v("sep")]) },
            Literal::Agg { var: "j2".into(), op: AggOp::Join { by: vec![], sep: "".into() }, over: Some("y".into()), atom: atom("a", vec![v("y")]) },
            Literal::Nearest { n: "x".into(), a: "p".into(), kindset: "Scope".into() },
            Literal::Ast { document: "d".into(), node: "n".into(), matcher: "{\"kind\": \"call\"}".into(), captures: vec![("NAME".into(), "v".into())] },
        ].into() },
        Rule { head: atom("unit", vec![]), body: vec![].into() },
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

/// Two programs read through one run's lists share each list they have in common: an argument list, a body, a
/// declaration's columns and a fact's row are each the one list in both (decision 0172).
#[test]
fn a_runs_programs_share_their_lists() {
    let program = |more: bool| {
        let mut prog = Program::default();
        prog.decls.insert("e".into(), [("x".into(), "int".into()), ("y".into(), "sym".into())].into());
        prog.decls.insert("p".into(), [("x".into(), "int".into()), ("y".into(), "sym".into())].into());
        prog.outputs = vec!["p".into()];
        prog.facts.insert("e".into(), vec![[Value::Int(1), Value::Str("a".into())].into()]);
        prog.rules.push(Rule { head: atom("p", vec![v("x"), v("y")]), body: vec![Literal::Atom(atom("e", vec![v("x"), v("y")])), Literal::Cmp { l: v("x"), op: "<", r: Term::Int(3) }].into() });
        if more {
            prog.rules.push(Rule { head: atom("p", vec![v("x"), v("y")]), body: vec![Literal::Atom(atom("e", vec![v("y"), v("x")]))].into() });
        }
        print(&prog).unwrap()
    };
    let text = |more: bool| program(more);
    let lists = crate::code_rule_dsl::logic::front::Lists::default();
    let one = super::read_in(&text(false), &lists).unwrap();
    let two = super::read_in(&text(true), &lists).unwrap();
    assert!(one.decls["e"].same(&two.decls["e"]) && one.decls["e"].same(&one.decls["p"]), "columns");
    assert!(one.facts["e"][0].same(&two.facts["e"][0]), "a fact's row");
    assert!(one.rules[0].body.same(&two.rules[0].body), "a body");
    assert!(one.rules[0].head.args.same(&two.rules[0].head.args), "arguments");
    assert!(!two.rules[1].head.args.same(&two.rules[1].body[0].atom().unwrap().args), "other arguments, another list");
    // And read alone, the same program.
    same(&one, &read(&text(false)).unwrap());
}

/// A stored program is kept as its text and its image (decision 0173), and read from its image; one kept as text
/// alone reads from its text.
#[test]
fn a_stored_program_is_read_from_its_image() {
    use crate::content_store::{ContentStore, HashKey};
    let dir = std::env::temp_dir().join(format!("rerail-program-image-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = ContentStore::open(&dir.join("store.sqlite")).unwrap();
    let mut prog = Program::default();
    prog.decls.insert("e".into(), [("x".into(), "int".into())].into());
    prog.decls.insert("p".into(), [("x".into(), "int".into())].into());
    prog.outputs = vec!["p".into()];
    prog.facts.insert("e".into(), vec![[Value::Int(1)].into()]);
    prog.rules.push(Rule { head: atom("p", vec![v("x")]), body: vec![Literal::Atom(atom("e", vec![v("x")])), Literal::Cmp { l: v("x"), op: "<", r: Term::Int(3) }].into() });
    let (digest, sources) = super::store(&store, &prog).unwrap().expect("kept");
    assert_eq!(sources.len(), 2, "its text and its image");
    assert!(store.source_by_alias(super::image_key(&digest)).unwrap().is_some());
    let found = super::texts(&store, &[digest]).unwrap();
    let (stored, ids) = &found[&digest];
    assert!(matches!(stored, super::Stored::Image(_)), "read from its image");
    assert_eq!(ids.len(), 2);
    same(&super::read_stored(stored, &Default::default()).unwrap(), &prog);
    same(&super::load(&store, &digest, &Default::default()).unwrap().expect("loads").0, &prog);
    // Kept as text alone (a store written before images): read from its text.
    let mut other = prog.clone();
    other.outputs.push("e".into());
    let text = print(&other).unwrap();
    store.put_source_hashed(text.as_bytes(), HashKey::sha256(text.as_bytes()), &[]).unwrap();
    let d2: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(text.as_bytes()).into();
    let found = super::texts(&store, &[d2]).unwrap();
    assert!(matches!(found[&d2].0, super::Stored::Text(_)));
    same(&super::load(&store, &d2, &Default::default()).unwrap().expect("loads").0, &other);
    drop(store);
    let _ = std::fs::remove_dir_all(&dir);
}
