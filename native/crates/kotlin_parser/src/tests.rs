//! The compiled grammar against the lean parser: the same tape, node for node, on every text it
//! reads; a refusal (to the lean parser) where it does not.

use super::*;

/// Each of `texts` read by the compiled grammar as the lean parser reads it.
fn same(texts: &[&str]) {
    let c = compiled().unwrap_or_else(|e| panic!("the grammar compiles: {e}"));
    for &text in texts {
        let want = lean_text(text).unwrap_or_else(|| panic!("the lean parser refuses {text:?}"));
        let image = c.parser.image(text.as_bytes()).unwrap_or_else(|why| panic!("the grammar refuses {text:?}: {why:?}\n{}\n{}", c.explain(text), c.words));
        let got = image_text(&image).expect("a tape image");
        assert_eq!(got, want, "{text:?}");
        // The interpreter's, over the interpreter's tier 1: the same tape.
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
fn headers_are_the_lean_parsers() {
    same(&[
        "package a\n",
        "package a.b.c\n",
        "package a.b\n\nimport c.d.E\nimport c.d.*\nimport c.d.E as F\n",
        "// lead\npackage a\n/* b */\nimport c.D\n// after\n",
        "import a.B\nimport c.*",
        "package a\n/** doc\n */\nimport b.C\n",
    ]);
}

/// A comment after a header on its line is the header's in tree-sitter's tree: the scanner's
/// semicolon is the line break after it (`mark line …, settles`). One on the next line is not.
#[test]
fn a_comment_on_a_headers_line_is_the_headers() {
    same(&[
        "package a // after\nimport c.D\n",
        "package a /* x */ // y\n// z\nimport c.D /* w */\n",
        "import c.D // at the end",
        "import c.D\n// after the last\n",
        "package a /* spans\nlines */\nimport c.D\n",
    ]);
}

/// Step 2: declarations, types, statements and the core of expressions.
#[test]
fn declarations_are_the_lean_parsers() {
    same(&[
        "fun f() {}\n",
        "fun g(a: Int, b: String): Int = a\n",
        "class A\n",
        "class B(val x: Int) : C(), D {\n    fun h() { return }\n}\n",
        "val x = 1\n",
        "private var y: Int? = null\n",
        "object O\n",
        "interface I\n",
        "data class P(private val a: List<String>, var b: Map<K, V>?)\n",
        "abstract class Q<T : Any> : R<T>() {\n    abstract fun f(x: T): T\n    override fun g() {}\n}\n",
        "@Suppress(x) @A.B fun f() {}\n",
        "fun f(vararg xs: Int, g: (Int) -> Unit) {}\n",
        "class C {\n    val a = 1 // one\n\n    fun b() = a\n}\n",
        "val data = 1\nfun get(value: Int) = value.inner\n",
        "class A(val value: Data<value>) : open()\n",
        "fun f() {\n    a.get(0)\n    data = 1\n    x.set += 2\n    enum[3] = where\n}\n",
    ]);
}

#[test]
fn statements_and_expressions_are_the_lean_parsers() {
    same(&[
        "fun f() {\n    val a = 1 + 2 * 3\n    b.c(d, e)\n    f[g] = h\n    if (a) b else c\n    x?.y ?: z\n    !a && b || c\n    for (i in 0..10) {}\n    while (true) {}\n}\n",
        "fun f() {\n    x = 1\n    x += y\n    a.b.c = d\n    a[1][2] = 3\n    a.b().c = 4\n}\n",
        "fun f() {\n    if (a < b && c >= d) return\n    else throw e\n}\n",
        "fun f() {\n    a\n        .b()\n        ?.c\n    -d\n    e !in f\n    g is H\n    i++\n    j!!\n}\n",
        "fun f() = g(a = 1, b)\n",
        "fun f() {\n    val l = listOf<Int>(1, 2)\n    val m: Map<String, List<Int>> = emptyMap<String, List<Int>>()\n}\n",
        "fun f() {\n    do { x-- } while (x > 0)\n    a to b\n    break\n    continue\n    this.x = null\n}\n",
        "fun f() {\n    // lead\n    a() // after a\n    /* b */ b()\n    c(/* in */ 1) // after c\n}\n",
    ]);
}

/// Step 3: strings, with `$name` templates.
#[test]
fn strings_are_the_lean_parsers() {
    same(&[
        "val a = \"x\"\n",
        "val b = \"\"\n",
        "val c = \"a\\nb $d and $e_1!\"\n",
        "val k = \"$\"\n",
        "val l = \"a$\"\n",
        "val m = \"\\$x \\\" y\\\\\"\n",
        "val n = \"$$x $ y\"\n",
        "fun f() {\n    g(\"a // b /* c\", \"$h.i\")\n    \"j\".k()\n}\n",
        "val o = \"h\u{e9}llo\" // \u{fc}\n",
    ]);
}

/// Step 4: characters and backtick names.
#[test]
fn characters_and_backtick_names_are_the_lean_parsers() {
    same(&[
        "val a = 'c'\n",
        "val b = '\\n'\n",
        "val c = '\\u0041'\n",
        "val d = '\\''\n",
        "val e = '\"'\n",
        "val f = '$' // '\n",
        "val g = '\\\\'\n",
        "fun `a test`() {}\n",
        "val h = `in`.`x y`\n",
        "val i = \"a'b`c\" + 'd'\n",
        "class `A B` : `C D`()\n",
    ]);
}

/// Step 5: callable references.
#[test]
fn callable_references_are_the_lean_parsers() {
    same(&[
        "val a = String::class\n",
        "val b = ::foo\n",
        "val c = A::b\n",
        "val d = a.b::c\n",
        "val f = x::class\n",
        "val g = foo(::bar, A::c)\n",
        "val h = this::foo\n",
        "val i = a::class.java\n",
        "val j = foo()::bar\n",
    ]);
}

/// Step 5: a trailing comma keeps its `,` and the list's closer.
#[test]
fn trailing_commas_are_the_lean_parsers() {
    same(&[
        "val a = f(1,)\n",
        "fun g(a: Int,) {}\n",
        "class C(val a: Int,)\n",
        "val b: Map<A, B,> = f(x[1, 2,],)\n",
        "val c = f(\n  1,\n  2,\n)\n",
        "val d = f<T>() + 1\n",
        "val e = a == f<T>()\n",
    ]);
}

/// Step 6: lambdas.
#[test]
fn lambdas_are_the_lean_parsers() {
    same(&[
        "val a = { x }\n",
        "val b = { x: Int, y -> x + y }\n",
        "val c = f { it }\n",
        "val d = f(1) { a ->\n    g(a)\n    h()\n}\n",
        "val e = { }\n",
        "val g = { -> 1 }\n",
        "fun f() {\n    if (x) { g() }\n    while (y) {\n        h()\n    }\n    list.forEach { println(it) }\n}\n",
        "val h = a.map { it * 2 }.filter { it > 1 }\n",
        "val i = f<T> { x }\n",
        "val j = f()\n    { x }\n",
        "fun f() { a.forEach { it } }\n",
    ]);
}

/// Step 7: annotations on lines of their own, use-site targets, file annotations.
#[test]
fn annotations_are_the_lean_parsers() {
    same(&[
        "@file:JvmName(\"A\")\npackage a\n",
        "@file:Suppress(\"x\")\n\nfun f() {}\n",
        "class A {\n    @Throws(X::class)\n    private fun f() {}\n    @get:Input\n    val x: Int = 1\n    @Inject lateinit var y: Y\n}\n",
        "fun g() {\n    @Suppress(\"x\")\n    val z = 1\n}\n",
        "@Deprecated(\"d\")\n@JvmStatic\nfun h() {}\n",
        "val file = 1\nfun file(file: File) = file\n",
    ]);
}

/// Step 8: `${…}` templates.
#[test]
fn templates_are_the_lean_parsers() {
    same(&[
        "val a = \"x${y}z\"\n",
        "val b = \"${f(1)} and ${a.b}\"\n",
        "val c = \"$a ${b + 1}$ $\"\n",
        "val d = \"${m[k]}!\"\n",
        "val e = \"a // b ${c} /* d */\"\n",
        "val f = \"\\${x} \\$y\"\n",
    ]);
}

#[test]
fn what_step_8_does_not_read_is_refused() {
    refused(&[
        "val a = b to f<T>()\n",
        "val a = x + f<T>(1)\n",
        "val a = x < f<T>(1)\n",
        "package a; import b\n",
        "import a.b import c\n",
        "val s = 'x\n",
        "val s = \"a\nb\"\n",
        "val s = \"${\"x\"}\"\n",
        "val s = \"${ {x} }\"\n",
        "val s = \"${}\"\n",
        "val s = \"\"\"raw\"\"\"\n",
        "val s = \"open\n",
        "/* /* nested */ */\npackage a\n",
        "/* open\n",
        "val f = { (a, b) -> a }\n",
        "val f = { a, -> a }\n",
        "val x = when (y) { else -> 1 }\n",
        "val x = 1L\n",
        "val x = y as Z\n",
        "class A {\n    companion object\n}\n",
        "class A { val x = 1 }\n",
        "val x: Int\n    get() = 1\n",
        "fun f() {\n    if (a<b.c) d\n}\n",
        "class A\n@Inject constructor(x: Int)\n",
        "fun f() {\n    a\n    /* c */ .b()\n}\n",
    ]);
}

/// The corpus: the lean parser's corpus files, each read by both. `same`: the compiled
/// grammar's tape is the lean parser's; `refused`: it leaves the file to the lean parser;
/// `differ`: a wrong tape, which must stay 0.
#[test]
#[ignore = "reads RERAIL_KOTLIN_CORPUS, a folder of .kt files"]
fn corpus() {
    let dir = std::env::var("RERAIL_KOTLIN_CORPUS").expect("set RERAIL_KOTLIN_CORPUS");
    let c = compiled().expect("compiles");
    let (mut same, mut refused, mut differ, mut lean_refused) = (0, 0, 0, 0);
    let mut paths: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
    paths.sort();
    for p in &paths {
        let Ok(text) = std::fs::read_to_string(p) else { continue };
        let Some(want) = lean_text(&text) else {
            lean_refused += 1;
            assert!(c.image(&text).is_none(), "{}: accepted a file the lean parser refuses", p.display());
            continue;
        };
        match c.image(&text) {
            None => {
                refused += 1;
                if std::env::var_os("RERAIL_KOTLIN_WHY").is_some() {
                    // The reference evaluator's first escape or error; else the grammar refused.
                    let stmts = ::bitlex::syntax::parse(crate::GRAMMAR).expect("parses");
                    let tables = Default::default();
                    let why = match ::bitlex::eval::Ev::run(&stmts, text.as_bytes(), &tables) {
                        Ok(out) => match out.errors.first() {
                            Some(e) => e.split(" (byte").next().unwrap_or(e).to_string(),
                            // The interpreter's: the text at the token it refused.
                            None => match c.refused_at(&text) {
                                Some(a) => {
                                    let a = a as usize;
                                    let near = text[a..].chars().take(24).collect::<String>().replace('\n', "⏎");
                                    let before = text[..a].chars().rev().take(24).collect::<Vec<_>>().into_iter().rev().collect::<String>().replace('\n', "⏎");
                                    format!("the grammar: `{near}` after `{before}`")
                                }
                                None => "the grammar".into(),
                            },
                        },
                        Err(e) => format!("eval: {e:?}"),
                    };
                    eprintln!("refused: {why} | {}", p.display());
                }
            }
            Some(image) if image_text(&image).as_deref() == Ok(&want) => same += 1,
            Some(_) => {
                differ += 1;
                eprintln!("differs: {}", p.display());
            }
        }
    }
    eprintln!("kotlin corpus: {same} the same, {refused} refused, {differ} differ ({lean_refused} the lean parser refuses)");
    assert_eq!(differ, 0);
}

/// The lean parser's tape for each file in `RERAIL_KOTLIN_SHOW` (paths, `:`-separated), and the
/// grammar's or why it refuses: the reference a ladder step is written against.
#[test]
#[ignore = "a tool: set RERAIL_KOTLIN_SHOW"]
fn show() {
    let c = compiled().expect("compiles");
    for p in std::env::var("RERAIL_KOTLIN_SHOW").expect("set RERAIL_KOTLIN_SHOW").split(':') {
        let text = std::fs::read_to_string(p).expect("a file");
        let lean = lean_text(&text).unwrap_or_else(|| "refused".into());
        if std::env::var_os("RERAIL_KOTLIN_TOKENS").is_some() {
            let stmts = ::bitlex::syntax::parse(crate::GRAMMAR).expect("parses");
            match ::bitlex::eval::Ev::run(&stmts, text.as_bytes(), &Default::default()) {
                Ok(out) => {
                    for t in &out.tokens {
                        eprintln!("reference {} {} {}..{}", if t.mark { "mark" } else { "token" }, t.kind, t.start, t.end);
                    }
                    eprintln!("reference errors {:?}", out.errors);
                    if let Some(tree) = &out.tree {
                        eprintln!("reference tree: {:?}", ::bitlex::tree::nodes(tree, &out.tokens, text.len()).map(|n| n.len()));
                    }
                    if let Some(r) = &c.program.block_programs[0].records {
                        // `RERAIL_KOTLIN_MASK=K`: mask K's output, from either tier 1.
                        if let Ok(m) = std::env::var("RERAIL_KOTLIN_MASK") {
                            let b = &c.program.block_programs[0];
                            let o = b.mask_outputs()[m.parse::<usize>().unwrap()];
                            let run = b.run(text.as_bytes());
                            let cpu = c.parser.tier1.extract(text.as_bytes());
                            let w = |v: &Vec<Vec<u64>>| v.get(o).and_then(|x| x.first()).map(|x| format!("{:064b}", x.reverse_bits()));
                            eprintln!("mask {m} output {o}: interp {:?}\n  cpu {:?} (cpu outputs {})", w(&run.outputs), w(&cpu.outputs), cpu.outputs.len());
                        }
                        for rd in r.readers.iter().filter(|rd| !rd.parts.is_empty()) {
                            eprintln!("reader {:?}", rd);
                        }
                        let run = c.program.block_programs[0].run(text.as_bytes());
                        for rd in r.readers.iter().filter(|rd| !rd.parts.is_empty()) {
                            for o in rd.parts.iter().flat_map(|p| std::iter::once(p.start).chain(p.last)).chain([rd.arms[0].closer]).chain(rd.arms[0].last) {
                                eprintln!("output {o}: {:064b}", run.outputs[usize::from(o)].first().copied().unwrap_or(0).reverse_bits());
                            }
                        }
                    }
                }
                Err(e) => eprintln!("reference: {e:?}"),
            }
        }
        match c.parser.image(text.as_bytes()) {
            Ok(image) => {
                let got = image_text(&image).unwrap_or_default();
                if got == lean {
                    eprintln!("== {p}: the same");
                } else {
                    eprintln!("== {p}: DIFFERS\n-- lean\n{lean}-- grammar\n{got}");
                }
            }
            Err(why) => {
                let words: String = c.explain(&text).lines().filter(|l| std::env::var_os("RERAIL_KOTLIN_TOKENS").is_some() || !l.contains("token ")).map(|l| format!("{l}\n")).collect();
                eprintln!("== {p}: refused {why:?}\n-- lean\n{lean}-- words\n{words}");
            }
        }
    }
}

/// The tier 2 program's size, by its words' opcodes (`RERAIL_KOTLIN_GRAMMAR`: another grammar
/// file than the ladder's last).
#[test]
#[ignore = "a measurement: run by hand"]
fn program_size() {
    let text = match std::env::var("RERAIL_KOTLIN_GRAMMAR") {
        Ok(p) => std::fs::read_to_string(p).expect("the grammar file"),
        Err(_) => crate::GRAMMAR.to_owned(),
    };
    let c = crate::compile_text(&text).expect("the grammar compiles");
    let lines: Vec<&str> = c.words.lines().collect();
    let mut by_op: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for l in &lines {
        if let Some(op) = l.split_whitespace().find(|w| w.chars().all(|c| c.is_ascii_uppercase() || c == '_') && w.len() > 1) {
            *by_op.entry(op).or_default() += 1;
        }
    }
    let mut ops: Vec<_> = by_op.into_iter().collect();
    ops.sort_by(|a, b| b.1.cmp(&a.1));
    eprintln!("{} lines; by opcode: {:?}", lines.len(), &ops[..ops.len().min(12)]);
    if let Ok(out) = std::env::var("RERAIL_KOTLIN_WORDS") {
        std::fs::write(out, &c.words).expect("the words written");
    }
    if let Ok(out) = std::env::var("RERAIL_KOTLIN_MACHINE") {
        std::fs::write(out, format!("{}", c.machine)).expect("the machine written");
    }
}
