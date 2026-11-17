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

#[test]
fn what_step_2_does_not_read_is_refused() {
    refused(&[
        "package a; import b\n",
        "import a.b import c\n",
        "val s = \"x\"\n",
        "/* /* nested */ */\npackage a\n",
        "/* open\n",
        "fun f() { a.forEach { it } }\n",
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
            None => refused += 1,
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
                let words: String = c.explain(&text).lines().filter(|l| !l.contains("token ")).map(|l| format!("{l}\n")).collect();
                eprintln!("== {p}: refused {why:?}\n-- lean\n{lean}-- words\n{words}");
            }
        }
    }
}
