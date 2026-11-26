//! The compiled grammar against the lean parser: the same tape, node for node, on every text it
//! reads; a refusal (to the lean parser) where it does not.

use super::*;

/// The tests' grammar: the crate's, or `RERAIL_PYTHON_GRAMMAR`'s file (a variant tried without a
/// rebuild). It shadows the crate's `compiled`.
fn compiled() -> Result<&'static Compiled, &'static str> {
    static OTHER: std::sync::OnceLock<Result<Compiled, String>> = std::sync::OnceLock::new();
    match std::env::var("RERAIL_PYTHON_GRAMMAR") {
        Ok(p) => OTHER.get_or_init(|| compile_answered(&std::fs::read_to_string(p).expect("the grammar file"))).as_ref().map_err(String::as_str),
        Err(_) => super::compiled(),
    }
}

/// Each of `texts` read by the compiled grammar as the lean parser reads it: the JIT's tape, and
/// the interpreter's over the interpreter's tier 1.
fn same(texts: &[&str]) {
    let c = compiled().unwrap_or_else(|e| panic!("the grammar compiles: {e}"));
    for &text in texts {
        let want = lean_text(text).unwrap_or_else(|| panic!("the lean parser refuses {text:?}"));
        let image = c.parser.image(text.as_bytes()).unwrap_or_else(|why| panic!("the grammar refuses {text:?}: {why:?}\n{}", c.explain(text)));
        let got = image_text(&image).expect("a tape image");
        assert_eq!(got, want, "{text:?}");
        let run = c.program.block_programs[0].run(text.as_bytes());
        let nodes = rerail_compiler::stream::run(&c.program, text.as_bytes(), &run).unwrap_or_else(|why| panic!("the interpreter refuses {text:?}: {why:?}"));
        let interp = rerail_compiler::stream::image(&nodes, &c.program.readings.extras, TAG, text.as_bytes());
        assert_eq!(image_text(&interp).expect("a tape image"), want, "the interpreter, {text:?}");
    }
}

/// Each of `texts` refused by the compiled grammar (to the lean parser).
fn refused(texts: &[&str]) {
    let c = compiled().expect("compiles");
    for &text in texts {
        assert!(c.image(text).is_none(), "accepted {text:?}");
    }
}

#[test]
fn simple_statements_are_the_lean_parsers() {
    same(&["x\n", "x\ny\n", "pass\n", "1\n", "x; y\n", "x;\n", "x", "", "\n\nx\n\n", "# c\nx  # d\n# e\n", "x\n# e"]);
    // The module: from its first token or comment, to past the blanks after its last, but no line break.
    same(&["x\n\n\n", "x  \n", "x\n  \n", "\nx", "  \nx\n", "x  # c\n\n"]);
}

#[test]
fn what_the_grammar_does_not_read_is_refused() {
    refused(&["'a'\n", "x.5\n", "0x1f\n", "x\\\n", "\tx\n"]);
}

/// The corpus: a folder of `.py` files, each read by both. `same`: the compiled grammar's tape is
/// the lean parser's; `refused`: it leaves the file to the lean parser; `differ`: a wrong tape,
/// which must stay 0.
#[test]
#[ignore = "reads RERAIL_PYTHON_CORPUS, a folder of .py files"]
fn corpus() {
    let dir = std::env::var("RERAIL_PYTHON_CORPUS").expect("set RERAIL_PYTHON_CORPUS");
    let c = compiled().expect("compiles");
    let (mut same, mut refused, mut differ, mut lean_refused) = (0, 0, 0, 0);
    let mut why: std::collections::BTreeMap<String, usize> = Default::default();
    let mut paths = Vec::new();
    let mut dirs = vec![std::path::PathBuf::from(&dir)];
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                dirs.push(p);
            } else if p.extension().is_some_and(|x| x == "py") {
                paths.push(p);
            }
        }
    }
    paths.sort();
    for p in &paths {
        let Ok(text) = std::fs::read_to_string(p) else { continue };
        let Some(want) = lean_text(&text) else {
            lean_refused += 1;
            continue;
        };
        match c.image(&text) {
            None => {
                refused += 1;
                if std::env::var_os("RERAIL_PYTHON_WHY").is_some() {
                    let stmts = ::bitlex::syntax::parse(crate::GRAMMAR).expect("parses");
                    let tables = Default::default();
                    let w = match ::bitlex::eval::Ev::run(&stmts, text.as_bytes(), &tables) {
                        Ok(out) => match out.errors.first() {
                            Some(e) => e.split(" (byte").next().unwrap_or(e).to_string(),
                            None => match c.refused_at(&text) {
                                Some(a) => {
                                    let a = a as usize;
                                    let near: String = text[a..].chars().take(12).collect::<String>().replace('\n', "⏎");
                                    format!("the grammar at `{near}`")
                                }
                                None => "the grammar".into(),
                            },
                        },
                        Err(e) => format!("eval: {e:?}"),
                    };
                    *why.entry(w).or_default() += 1;
                }
            }
            Some(image) if image_text(&image).as_deref() == Ok(&want) => same += 1,
            Some(image) => {
                differ += 1;
                let got = image_text(&image).unwrap_or_default();
                let (have, wanted): (Vec<&str>, Vec<&str>) = (got.lines().collect(), want.lines().collect());
                let at = have.iter().zip(&wanted).position(|(a, b)| a != b).unwrap_or(have.len().min(wanted.len()));
                let from = at.saturating_sub(4);
                eprintln!("differs: {}\n  context:\n    {}\n  compiled:\n    {}\n  lean:\n    {}", p.display(), wanted[from..at].join("\n    "), have[at..(at + 3).min(have.len())].join("\n    "), wanted[at..(at + 3).min(wanted.len())].join("\n    "));
            }
        }
    }
    let mut why: Vec<(usize, String)> = why.into_iter().map(|(w, n)| (n, w)).collect();
    why.sort_unstable_by(|a, b| b.cmp(a));
    for (n, w) in why.iter().take(40) {
        eprintln!("refused {n:5}: {w}");
    }
    eprintln!("python corpus: {same} the same, {refused} refused, {differ} differ ({lean_refused} the lean parser refuses)");
    assert_eq!(differ, 0);
}
