//! The compiled grammar against the lean parser: the same tape, node for node, on every text it
//! reads; a refusal (to the lean parser) where it does not.

use super::*;

/// The tests' grammar: the crate's, or `RERAIL_KOTLIN_GRAMMAR`'s file (a variant tried without a
/// rebuild). It shadows the crate's `compiled`.
fn compiled() -> Result<&'static Compiled, &'static str> {
    static OTHER: std::sync::OnceLock<Result<Compiled, String>> = std::sync::OnceLock::new();
    match std::env::var("RERAIL_KOTLIN_GRAMMAR") {
        Ok(p) => OTHER.get_or_init(|| compile_answered(&std::fs::read_to_string(p).expect("the grammar file"))).as_ref().map_err(String::as_str),
        Err(_) => super::compiled(),
    }
}

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
        // Tier 1 as Rust: the interpreter's records; tier 2 as Rust over them: the same tape.
        if let Some(why) = tier1_differs(c, text) {
            panic!("tier 1 as Rust, {text:?}: {why}");
        }
        let aot = aot_text(text).unwrap_or_else(|at| panic!("the parser as Rust refuses {text:?} at byte {at}"));
        assert_eq!(aot, want, "the parser as Rust, {text:?}");
    }
}

/// Each of `texts` refused by the compiled grammar (to the lean parser), by tier 1 as Rust where
/// the interpreter's tier 1 refuses it.
fn refused(texts: &[&str]) {
    let c = compiled().expect("compiles");
    for &text in texts {
        assert!(c.image(text).is_none(), "accepted {text:?}");
        if let Some(why) = tier1_differs(c, text) {
            panic!("tier 1 as Rust, {text:?}: {why}");
        }
    }
}

/// Each of `texts`: tier 1 as Rust is the interpreter's tier 1, refused or not.
fn tier1_same(texts: &[&str]) {
    let c = compiled().expect("compiles");
    for &text in texts {
        if let Some(why) = tier1_differs(c, text) {
            panic!("tier 1 as Rust, {text:?}: {why}");
        }
    }
}

/// The template spans' escapes, each tested in the rows it may hold in (decision 0595): a raw
/// string in a template's code, a string, a character or a backtick name in a raw string's
/// template; and a template's span after its end. A `}` in a token of the code is no template's
/// end (decision 0600): a string after it that never closes is refused.
#[test]
fn the_template_spans_are_the_interpreters() {
    refused(&[
        "val a = \"${\"}\"}\"\"\n",
        "val a = \"${'}'}\"\"\n",
    ]);
    tier1_same(&[
        "val a = \"${b}\"\n",
        "val a = \"${b + c}\" + \"${d}\"\n",
        "val a = \"${b}\"\nval c = \"\"\"x\"\"\"\n",
        "val a = \"\"\"${b}\"\"\"\nval c = \"x\" + 'y' + `z`\n",
        "val a = \"${b}\" + c.d { e }\n",
        "val a = \"${b}\n",
        "val a = \"${\n",
    ]);
}

/// "A `0x` or `0b` with no digit", tested at a number's first byte and, where a `0` may start a
/// number inside one (`1e+0x`), over its bytes after (decision 0587). There the `x` is also a name
/// touching a number, refused either way.
#[test]
fn the_0x_escape_is_the_interpreters() {
    refused(&["val a = 0x\n", "val a = 0xg\n", "val a = 0b\n", "val a = 0b2\n", "val a = 1e+0x\n"]);
    tier1_same(&["val a = 0x", "val a = 0x1F\n", "val a = 0b10\n", "val a = x0x\n", "val a = \"0x\"\n", "val a = 0\nval x = 1\n", "val a = 10x\n"]);
}

/// The masks of a type after `as` the lean parser may split (`as_run`, `as_lt`), as the
/// interpreter's: after a token, over a gap, over a token's bytes after its first (`..<`), and
/// never from an `as` in a region or across one.
#[test]
fn the_as_split_masks_are_the_interpreters() {
    tier1_same(&[
        "val a = b as java.util.List<Int>\n",
        "val a = b as  x.y<Int>\n",
        "val a = b as x?.y<Int>\n",
        "val r = x as a.B..<c\n",
        "val a = b as x.y<Int>\n  c()\n",
        "val a = b is x.y<Int>\n  val c = 1\n",
        "val a = b as x.y<Int>  \n  c()\n",
        "val a = b as x.y<Int> // c\n  c()\n",
        "val a = b as x.y<Int>\r\n  c()\n",
        "val a = b as x.y // c\n  c()\n",
        "val a = b as List<Int>\n",
        "val a = b // as\nval c = d.e<Int>()\n",
        "val a = \"as\" + d.e<Int>()\n",
        "val a = b /* as */ .e<Int>()\n",
        "val a = b as x /* c */ .y<Int>()\n",
        "val a = b as x\n.y<Int>()\n",
        "val a = b as x.y\nval c = d<e>()\n",
        "val a = b as(x.y<Int>)\n",
        "val r = x as a..<c\n",
        "val a = `as` .y<Int>()\n",
    ]);
}

/// The parser as Rust (`rerail_kotlin_aot`, its `tier1.rs` and `tier2.rs`) is the Bitlex
/// compiler's output for the grammar (`tiers_rust`); `RERAIL_BLESS=1` writes it again.
#[test]
fn rust_is_current() {
    let (tier1, tier2) = crate::tiers_rust(crate::GRAMMAR).unwrap_or_else(|e| panic!("the parser as Rust: {e}"));
    for (file, rust) in [("tier1.rs", tier1), ("tier2.rs", tier2)] {
        let path = format!("{}/../kotlin_aot/src/{file}", env!("CARGO_MANIFEST_DIR"));
        if std::env::var_os("RERAIL_BLESS").is_some() {
            std::fs::write(&path, &rust).expect("writes");
        }
        assert!(std::fs::read_to_string(&path).is_ok_and(|have| have == rust), "kotlin_aot's {file} is not the compiler's: RERAIL_BLESS=1");
    }
}

/// `text`'s tape from the parser as Rust (`rerail_kotlin_aot`: tier 1, then tier 2), as text.
fn aot_text(text: &str) -> Result<String, u32> {
    let tape = rerail_kotlin_aot::parse(text.as_bytes())?;
    Ok(lean::tape_text(&tape))
}

/// Where tier 1 as Rust (`rerail_kotlin_aot::lex`) is not the interpreter's tier 1: whether it
/// sends the file to tree-sitter, and where neither does, each token's first and last byte and
/// code.
fn tier1_differs(c: &Compiled, text: &str) -> Option<String> {
    let run = c.program.block_programs[0].run(text.as_bytes());
    let lx = rerail_kotlin_aot::lex(text.as_bytes());
    let refused = run.totals.first().is_some_and(|&t| t != 0);
    // An escape of `A & ~L` the lowering makes a local check of tier 2's at `A`'s token
    // (`pending_checks`): the interpreter refuses the file there, tier 1 as Rust where it reads
    // the escape. Either way the file is refused.
    if lx.refused && !refused && c.parser.image(text.as_bytes()).is_err() {
        return None;
    }
    if refused != lx.refused {
        return Some(format!("refused: the interpreter {refused}, tier 1 as Rust {}", lx.refused));
    }
    if refused {
        return None;
    }
    let (starts, lasts) = (&run.positions[0], &run.positions[1]);
    let n = starts.len().max(lx.starts.len());
    for k in 0..n {
        let theirs = (starts.get(k), lasts.get(k), run.codes.get(k));
        let ours = (lx.starts.get(k), lx.lasts.get(k), lx.codes.get(k));
        if theirs != ours {
            let at = |s: Option<&u32>| s.map_or(String::new(), |&s| text[s as usize..].chars().take(16).collect::<String>().replace('\n', "⏎"));
            return Some(format!("record {k}: the interpreter {theirs:x?} `{}`, tier 1 as Rust {ours:x?} `{}`", at(theirs.0), at(ours.0)));
        }
    }
    None
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
        // `/=` at the comments' reader's byte: the pair its other rule reads (the JIT read `/` alone).
        "fun f() {\n    w /= scale\n    h = a / b // c\n    d /= 2 /* e */\n}\n",
        "fun f() {\n    if (a < b && c >= d) return\n    else throw e\n}\n",
        "fun f() {\n    a\n        .b()\n        ?.c\n    -d\n    e !in f\n    g is H\n    i++\n    j!!\n}\n",
        "fun f() = g(a = 1, b)\n",
        "fun f() {\n    val l = listOf<Int>(1, 2)\n    val m: Map<String, List<Int>> = emptyMap<String, List<Int>>()\n}\n",
        "fun f() {\n    do { x-- } while (x > 0)\n    a to b\n    break\n    continue\n    this.x = null\n}\n",
        "fun f() {\n    // lead\n    a() // after a\n    /* b */ b()\n    c(/* in */ 1) // after c\n}\n",
    ]);
}

/// A block comment that starts a line before a token that goes on the line before (`.`, `?:`,
/// `as`, `+`…): where the lean parser asks for a semicolon there (`asi`, at a statement's level),
/// the comment is lost, of no width after the token; inside brackets it is any comment
/// (`semi`'s `loses`, decision 0451).
#[test]
fn lost_comments_are_the_lean_parsers() {
    let texts = crate::lost_comment_texts();
    same(&texts.iter().map(String::as_str).collect::<Vec<_>>());
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

/// Step 9: numbers.
#[test]
fn numbers_are_the_lean_parsers() {
    same(&[
        "val a = 1L + 2u + 3UL + 0xFF + 0b10 + 1.5 + 2f + 1e5 + .5 + 1_000 + 0xFFL + 1.0e-3f\n",
        "val b = 1..2\n",
        "val c = 1.toString() + 2.5.toInt()\n",
        "val d = x[0] + -1 + 1E+2 + 0XAbu\n",
        "val e = a / 2.0 + 255.0 + 1.5.toInt()\n",
    ]);
}

/// A number's suffix is the name touching it (`sfx`), as the lean parser's `integer` reads the
/// token after; elsewhere `L`, `u`, `U`, `uL` and `UL` are names.
#[test]
fn suffixes_are_the_names_touching_a_number() {
    same(&[
        "val a = 0b1u + 0x1uL + 7U + 7UL + 7uL\n",
        "val L = 1\nval u = L + U\nfun uL(UL: Int) = UL\n",
        "val a = L.b + 1L.c\n",
        "val b = 1L\nval L = 2\n",
    ]);
    refused(&["val a = 1x\n", "val a = 0xFFg\n", "val a = 0b1z\n", "val a = 1Lx\n", "val a = 1.5x\n", "val a = 1lu\n"]);
}

/// Step 10: a class's members.
#[test]
fn members_are_the_lean_parsers() {
    same(&[
        "class A : B() {\n    companion object {\n        const val X = 1\n    }\n    init {\n        super.f()\n    }\n    val y: Int\n        get() = 1\n    var z = 0\n        private set\n}\n",
        "class A {\n    companion object F : X {\n    }\n    var z = 0\n        set(v) { field = v }\n    val w get() = 2\n    init { }\n}\nfun f(vararg x: Int) {}\n",
    ]);
}

/// Step 11: `when`.
#[test]
fn when_is_the_lean_parsers() {
    same(&[
        "fun f(x: Int) = when (x) {\n    1, 2 -> a()\n    in r -> b\n    is T -> { c() }\n    else -> d\n}\n",
        "fun g() {\n    when {\n        x > 1 -> a()\n        !y -> b()\n    }\n    val v = when (val y = h()) {\n        else -> 0\n    }\n}\n",
        "fun g() {\n    when (x) {\n        // c\n        1 -> {\n        }\n\n        else -> {}\n    }\n}\n",
        "fun g() {\n    when (x) {\n        1 -> a // one\n        else -> 0 // two\n    }\n}\n",
    ]);
}

/// Step 12: `;`.
#[test]
fn semicolons_are_the_lean_parsers() {
    same(&[
        "package a;\nimport b.C;\nfun f() {\n    a(); b()\n    c(); // x\n    d();\n}\nclass A { val x = 1; }\n",
        "fun f() { a(); }\nval x = 1;\nfun g() = 2\n",
        "package a; import b\n",
        "class B {\n    fun f() {}; fun g() {}\n}\nclass C { fun f(); }\n",
    ]);
}

/// Step 13: `try`, and `as`.
#[test]
fn try_and_as_are_the_lean_parsers() {
    same(&[
        "fun f() {\n    try {\n        a()\n    } catch (e: E) {\n        b()\n    } finally {\n        c()\n    }\n    val x = try { 1 } catch (_: Exception) { 2 }\n    val y = z as T\n    val w = (q as List<Int>).size\n}\n",
        "fun f() {\n    try {\n    }\n    catch (e: E) {}\n    val a = -b as C * d as E + f\n}\n",
    ]);
}

/// Step 14: a lambda that opens a lambda.
#[test]
fn nested_lambdas_are_the_lean_parsers() {
    same(&[
        "fun f() {\n    Row {\n        xs.forEach { x ->\n            g(x)\n        }\n    }\n    a { b.c { d, e -> 1 } }\n}\n",
    ]);
}

/// Step 15: extension receivers.
#[test]
fn receivers_are_the_lean_parsers() {
    same(&[
        "fun String.faint(): String = this\nfun List<T>.g() {}\nval A.B.c: Int get() = 2\n",
        "class A {\n    val String.x\n        get() = 1\n    fun f() {\n        val y = a.b\n        var z: A.B = c\n    }\n}\n",
        // Nested type arguments in the type parameters: `receiver_end` reads past them.
        "fun <T : Comparable<T>> B.f() {}\n",
    ]);
}

/// Step 16: generic functions.
#[test]
fn generic_functions_are_the_lean_parsers() {
    same(&[
        "fun <T> List<T>.g() {}\nfun <T> f(x: T): T = x\nprivate fun <A, B> h() {}\n",
        "class C {\n    fun <T> g(): T? = null\n    val x = foo<Int>(1)\n}\n",
    ]);
}

/// Step 17: delegation, and the three-byte operators.
#[test]
fn delegates_are_the_lean_parsers() {
    same(&[
        "class A(b: B) : I by b, J {\n    val x by lazy { 1 }\n    private val y: Int by mapOf()\n}\nval z by lazy {\n    2\n}\n",
        "val a = b === c\nval d = e !== f\nval g = 0..<n\nval h = 1..2\n",
    ]);
}

/// Step 18: object literals, secondary constructors, destructuring.
#[test]
fn objects_are_the_lean_parsers() {
    same(&[
        "val h = object : Handler(), I {\n    override fun f() {}\n}\nclass A @Inject constructor(\n    private val b: B,\n) : C() {\n    constructor(x: Int) : this(B()) {\n        g()\n    }\n    constructor() : super()\n}\nfun g() {\n    val (a, b) = p\n    val (c: Int, _) = q\n}\n",
        "class A {\n    companion object {\n    }\n    val o = f(object : R {\n        override fun run() {}\n    })\n}\n",
    ]);
}

/// Step 19: type arguments where the lean parser reads them.
#[test]
fn type_arguments_are_the_lean_parsers() {
    same(&[
        "fun f() {\n    val d = a.b<T>(x)\n    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) {}\n    val e = listOf<Map<String, Int>>()\n    g<A> { }\n    val n = x < y\n    val m = i < n && j > (k)\n}\nclass C<T>(val t: T) : D<T>() {\n    val l: List<T>? = null\n}\n",
        "fun f() {\n    if (a<b.c) d\n}\n",
    ]);
}

/// Runs that count their depths (Q339), as `type_args_end` and `lambda_params_end` do: type
/// arguments to the `>` that closes the `<`, a function type inside them, a lambda's parameters
/// with brackets and an inner `->`; `a<b>c>(d)`, whose first `>` closes, is no call.
#[test]
fn depths_are_the_lean_parsers() {
    same(&[
        "val a = f<A<B>, C>(x)\nval b = f<Map<A, List<B>>>(x)\nval c = g<(A) -> B>(y)\n",
        "val d = { a: Map<K, (V) -> W> -> a }\n",
        "val e = { (a, b): Pair<A, B> -> a }\n",
        "val f = a<b>c>(d)\n",
    ]);
}

/// Step 20: enum classes, labels, `as?`, reified type parameters, spread arguments.
#[test]
fn enums_and_labels_are_the_lean_parsers() {
    same(&[
        "enum class E(val v: Int) {\n    A(1),\n    B(2) {\n        override fun f() = 1\n    };\n    fun g() = 0\n}\nenum class F { X, Y }\ninline fun <reified T : Any, R> h(vararg a: T) = g(*a)\nclass P private constructor(x: Int)\nfun k() {\n    xs.map { return@map 1 }\n    for (i in a) break@loop\n    val t = this@P\n}\n",
        "internal enum class G {\n    A,\n    B,\n}\nval h = i as? J\n",
    ]);
}

/// Step 21: raw strings.
#[test]
fn raw_strings_are_the_lean_parsers() {
    same(&[
        "val a = \"\"\"x\n  \"y\" $b \\n\"\"\"\nval d = \"\"\"\"\"\"\nval f = \"\"\"$\"\"\"\nval g = \"\"\"\n    a\"\"b\n\"\"\"\nval i = \"\"\"a\"\n\"\"\"\n",
        "fun q() = \"\"\"\n    SELECT * FROM t WHERE a = $b\n\"\"\".trimIndent()\n",
        // An opener's run of four or five quotes: its `"""`, then content.
        "val h = \"\"\"\"a\"\"\"\n",
        "val h = \"\"\"\"\"a\"\"\"\n",
        "val h = \"\"\"\"\n\"\"\"\n",
        "val h = \"\"\"\"$a\"\"\"\n",
        "val h = \"\"\"\"${a}\"\"\"\n",
        "val h = \"\"\"a\"b\"\"c\"\"\"\n",
        "val h = \"\"\"\"\"\"\nval i = 1\n",
        // A quote one or two bytes before a closer, in a run of its own.
        "val i = \"\"\"a\"\n\"\"\"\n",
        "val i = \"\"\"a\"b\"\"\"\n",
        "val i = \"\"\"a\" \"\"\"\n",
        // A closing run of four quotes or more: the lean parser's closer takes them all, its
        // quotes before the last three no leaves (`r_lead`).
        "val h = \"\"\"a\"\"\"\"\n",
        "val h = \"\"\"a\"\"\"\"\"\n",
        "val h = \"\"\"${a}\"\"\"\"\n",
        "val h = \"\"\"\"\"\"\"\n",
        "val h = \"\"\"\"\"\"\"\"\"\n",
        "val h = \"\"\"a ${b}\"\"\"\"\"\n",
        "val h = \"\"\"a\"\" b\"\"\"\"\n",
        "val h = \"\"\"\"a\"\"\"\"\n",
        "val h = \"\"\"a\"\"\" + \"\"\"b\"\"\"\n",
    ]);
    // An opener in a block's last two bytes: its closer past them, in the next block (the JIT's
    // walk carries the opener's bytes over, as the places it hides).
    let pad = |p: usize, body: &str| format!("//{}\nval s = {body}\n", "x".repeat(p - 3));
    let bodies = ["\"\"\"\"x\"\"\"", "\"\"\"\"\"\"", "\"\"\"\"\"\"\"", "\"\"\"a\"\"\"\"", "\"\"\"\"${a}\"\"\""];
    let edges: Vec<String> = (52..=56).flat_map(|p| bodies.iter().map(move |b| pad(p, b))).collect();
    same(&edges.iter().map(String::as_str).collect::<Vec<_>>());
    // A string whose `"` starts a block (byte 64, 65, 128), the token before in the block
    // before: the refill reads the block before its rows' words are final, and the opener of
    // the two regions is read from the rows (the CPU's tier 1 waits on it).
    let at = |q: usize, line: &str| format!("//{}\n{line}\n", "x".repeat(q - line.find('"').expect("a quote") - 3));
    let lines = ["val v = \"c\"", "val v = \"\"\"c\"\"\"", "val v = \"${a}\"", "val v = \"c\" + \"d\""];
    let texts: Vec<String> = [64, 65, 128].iter().flat_map(|&q| lines.iter().map(move |l| at(q, l))).collect();
    same(&texts.iter().map(String::as_str).collect::<Vec<_>>());
}

#[test]
fn destructured_parameters_are_the_lean_parsers() {
    same(&[
        "val f = m.forEach { (k, v) -> g(k) }\n",
        "val f = m.map { (_, v) -> v }\n",
        "val f = { (a, b), c -> a }\n",
        "fun f() {\n    for ((k, v) in m) g(k)\n}\n",
        "val x = a as B.C\nval y = 1\n",
        "fun f() {\n    val d = x as B.C\n    g(d)\n}\n",
        "fun f() {\n    g(x as B.C)\n}\n",
        "fun f() {\n    (x as java.nio.Buffer).position(1)\n}\n",
        "val x = (a as? B.C)?.d\n",
        "fun f(a: (url: String) -> Unit) {}\n",
        "val f: (a: Int, b: String) -> Unit = {}\n",
        "fun f(a: (Int, b: String) -> Unit) {}\n",
        "fun <T : Any> B.get(): T = x\n",
        "fun <T : Any> T.f() {}\n",
    ]);
}

#[test]
fn types_and_aliases_are_the_lean_parsers() {
    same(&[
        "fun f(s: (() -> Int)?) = 1\n",
        "val c: ((Boolean) -> Unit)? = null\n",
        "val l = mutableListOf<(Int) -> Unit>()\n",
        "typealias O = (Int) -> Unit\n",
        "public typealias W = Int\n",
        "typealias M<T> = Map<String, T>\n",
        "@Entity(primaryKeys = [A, B])\nclass E\n",
        "class A\n@Inject\nconstructor(\n    private val b: B,\n)\n",
        "class A\n@JvmOverloads\nconstructor(context: C) : B(context)\n",
        "class A @Inject constructor(b: B)\n",
        "@Ann\nclass A\n@Ann\nclass B\n",
    ]);
}

#[test]
fn strings_in_templates_are_the_lean_parsers() {
    same(&[
        "val s = \"${\"x\"}\"\n",
        "val v = \"${System.getProperty(\"user.name\")}\" + a\n",
        "val p = \"${name.replace('.', '/')}/a\"\n",
        "val q = \"a ${f(\"b\", 'c')} d $e\"\n",
        "val r = \"${f(\"$x\")}\"\n",
        "// \"${f(\"}\")}\"\nval a = 1\n",
    ]);
}

/// Step 35: templates in raw strings.
#[test]
fn raw_templates_are_the_lean_parsers() {
    same(&[
        "val x = \"\"\"a ${b} \"c\" ${d}\"\"\"\nval y = \"\"\"${e}\"\"\"\n",
        "val x = \"\"\"\n  a $b ${c.d}\n  \"q\" \"\"\n\"\"\"\n",
        "val h = \"\"\"${a}\"\"\"\n",
        "val s = \"a ${f('x')} b ${g(\"y\")} c\" + \"\"\"${h} \"\"\" + \"${i}\"\n",
        "fun q() = \"\"\"\n    SELECT * FROM ${t.name} WHERE a = ${b + 1}\n    AND c = '$d'\n\"\"\".trimIndent()\n",
    ]);
    // A raw string's template at a block's edge (bytes 64, 65, 128): its `$`, its `}` and the
    // raw tail after it each across a block from the string's opener.
    let at = |q: usize, line: &str| format!("//{}\n{line}\n", "x".repeat(q - line.find('$').expect("a dollar") - 3));
    let lines = ["val v = \"\"\"c${a}d\"\"\"", "val v = \"\"\"${a} ${b}\"\"\"", "val v = \"\"\"c${ab}\"\"\""];
    let texts: Vec<String> = [64, 65, 66, 128].iter().flat_map(|&q| lines.iter().map(move |l| at(q, l))).collect();
    same(&texts.iter().map(String::as_str).collect::<Vec<_>>());
}

/// A template's tail goes back to the kind of string the template is in, whatever its code
/// holds (the lean lexer's string reader, back from the template's code): a string, a character
/// or a backtick name in a raw string's template, a raw string in a string's template.
#[test]
fn a_tail_is_the_strings_whose_template_it_ends() {
    same(&[
        "val h = \"\"\"${f(\"a\")}\"\"\"\n",
        "val h = \"\"\"${f('a')}\"\"\"\n",
        "val h = \"\"\"${`a`}\"\"\"\n",
        "val h = \"\"\"a ${f(\"b\")} \"c\" ${'d'}e\"\"\"\n",
        "val h = \"${f(\"\"\"a\"\"\")}\"\n",
        "val h = \"${f(\"\"\"a\"\"\")} b ${c}\" + \"\"\"${d}\"\"\"\n",
    ]);
}

/// A string with templates is one record (decision 0600), with its templates' code's records
/// inside it and none at a template's `}`, as the lean lexer reads it: Kotlin's grammar with its
/// tails resuming their strings, tier 1 and tier 2 by the interpreter, the tape the lean
/// parser's.
#[test]
fn a_string_with_templates_is_one_record() {
    let c = compiled().unwrap_or_else(|e| panic!("the grammar compiles: {e}"));
    let texts = [
        "val a = \"x ${b} y\"\n",
        "val a = \"${b}${c}\" + d\n",
        "val a = \"${f(\"x\", 'y')} $z ${g}\"\nval b = \"\"\"${h(\"i\")} j\"\"\"\n",
        "val h = \"${f(\"\"\"a\"\"\")} b ${c}\" + \"\"\"${d}\"\"\"\n",
        "val h = \"\"\"a ${f(\"b\")} \"c\" ${'d'}e\"\"\"\n",
        "val a = \"${ b }\"\nval c = \"${ d } e\"\n.f()\n",
        "fun f() = \"${ a }\"",
        // A `}` in a string or a character of a template's code is no template's end; a `}` of
        // code after a string ended opens no tail.
        "val a = \"${f(\"}\")} b\"\n",
        "val a = \"${\"}\"}\"\nval b = \"${'}'}\"\nval s = \"${f(\"}\")}\"\n",
        "val a = \"${f('}')}\" + \"\"\"${g(\"}\")}\"\"\"\n",
        "fun f() { val a = \"${b}\" }\nfun g() {\n    h(\"${c} d\") }\n",
        "class A {\n    val s = \"${x}${y}\"\n    fun f() { }\n}\n",
        // A tail read across a block's edge: the loop reads it again where its block is final.
        "fun q() = \"${a} ${b} xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\"\n",
        "fun q() = \"\"\"\n    SELECT * FROM ${t.name} WHERE a = ${b + 1}\n    AND c = '$d'\n\"\"\".trimIndent()\n",
    ];
    // A template at a block's edge (bytes 64, 65, 128): its `$`, its code and its `}` each
    // across a block from the string's opener.
    let at = |q: usize, line: &str| format!("//{}\n{line}\n", "x".repeat(q - line.find('$').expect("a dollar") - 3));
    let lines = ["val v = \"c${a}d\"", "val v = \"\"\"${f(\"a\")} ${b}\"\"\"", "val v = \"c${ab} ${'d'}\""];
    let edges: Vec<String> = [64, 65, 66, 128].iter().flat_map(|&q| lines.iter().map(move |l| at(q, l))).collect();
    for text in texts.iter().copied().chain(edges.iter().map(String::as_str)) {
        let bytes = text.as_bytes();
        let want = lean_text(text).unwrap_or_else(|| panic!("the lean parser refuses {text:?}"));
        let program = &c.program.block_programs[0];
        let run = program.run(bytes);
        let r = program.records.as_ref().expect("records");
        assert!(run.totals.iter().all(|&t| t == 0), "tier 1 refuses {text:?}");
        let (starts, lasts) = (&run.positions[usize::from(r.starts)], &run.positions[usize::from(r.lasts)]);
        // Each string's record: from its quote to its last, outside the templates' code.
        let mut at = 0usize;
        while at < bytes.len() {
            if bytes[at] == b'"' {
                let raw = bytes[at..].starts_with(b"\"\"\"");
                let k = starts.iter().position(|&s| s as usize == at).unwrap_or_else(|| panic!("a record at {at} of {text:?}"));
                let mut e = at + if raw { 3 } else { 1 };
                let mut code = 0usize;
                loop {
                    if code == 0 && bytes[e..].starts_with(if raw { b"\"\"\"" } else { b"\"" }) {
                        e += if raw { 2 } else { 0 };
                        break;
                    }
                    if code == 0 && bytes[e..].starts_with(b"${") {
                        code = 1;
                        e += 2;
                        continue;
                    }
                    // A string or a character in the code: its bytes are no template's `}`.
                    if code > 0 && matches!(bytes[e], b'"' | b'\'') {
                        let q = bytes[e];
                        e += 1;
                        while bytes[e] != q {
                            e += if bytes[e] == b'\\' { 2 } else { 1 };
                        }
                        e += 1;
                        continue;
                    }
                    if code > 0 && bytes[e] == b'}' {
                        assert!(!starts.contains(&(e as u32)), "no record at a template's `}}`, {e} of {text:?}");
                    }
                    code = match (code, bytes[e]) {
                        (0, _) => 0,
                        (_, b'}') => code - 1,
                        (_, b'{') => code + 1,
                        _ => code,
                    };
                    e += 1;
                }
                assert_eq!(lasts[k] as usize, e, "the string's record at {at} of {text:?}");
                at = e + 1;
                continue;
            }
            at += 1;
        }
        // The regions as rounds, Metal's form, settle to the walk's: every output.
        let (walked, spans) = program.all_values(&[bytes]);
        let (rounds, _) = program.rounds_values(&[bytes]);
        for (k, &v) in program.outputs.iter().enumerate() {
            let v = usize::from(v);
            assert_eq!(rounds[v][spans[0].clone()], walked[v][spans[0].clone()], "output {k} as rounds, {text:?}");
        }
        // The records' masks, as a device without the token loop reads them (Metal): the loop's
        // records, a region tier 2 splits its opener alone, and none at a template's `}`.
        let bits = |o: u16| -> Vec<u32> {
            let words = &run.outputs[program.extract_outputs()[usize::from(o)]];
            (0..words.len() * 64).filter(|&b| b < bytes.len() && words[b / 64] >> (b % 64) & 1 == 1).map(|b| b as u32).collect()
        };
        let opened: Vec<u32> = starts.iter().zip(lasts.iter()).map(|(&s, &l)| if matches!(bytes[s as usize], b'"' | b'\'') { s } else { l }).collect();
        assert_eq!((bits(r.starts), bits(r.lasts)), (starts.clone(), opened), "the records' masks, {text:?}");
        // Tier 2 over them, coded by the record pass, as over a device's records.
        let table = rerail_compiler::records::WordTable::new(&r.words).expect("the words fit a table");
        let (ms, ml) = (bits(r.starts), bits(r.lasts));
        let (codes, bad) = rerail_compiler::records::pass(r, &table, bytes, &ms, &ml, &run.outputs);
        assert!(!bad, "the record pass refuses the masks' records, {text:?}");
        let mut device = run.clone();
        device.positions[usize::from(r.starts)] = ms;
        device.positions[usize::from(r.lasts)] = ml;
        device.codes = codes;
        let nodes = c.parser.tier2.run(bytes, &device).unwrap_or_else(|why| panic!("the CPU's tier 2 refuses the masks' records of {text:?}: {why:?}"));
        let over = rerail_compiler::stream::image(&nodes, &c.program.readings.extras, TAG, bytes);
        assert_eq!(image_text(&over).expect("a tape image"), want, "the CPU's tier 2 over the masks' records, {text:?}");
        let nodes = rerail_compiler::stream::run(&c.program, bytes, &run).unwrap_or_else(|why| panic!("the interpreter refuses {text:?}: {why:?}"));
        let interp = rerail_compiler::stream::image(&nodes, &c.program.readings.extras, TAG, bytes);
        assert_eq!(image_text(&interp).expect("a tape image"), want, "the interpreter, {text:?}");
        // The CPU's tier 1: the interpreter's records and codes.
        let cpu = c.parser.tier1.extract(bytes);
        assert_eq!(cpu.totals, run.totals, "the CPU's tier 1 refuses, {text:?}");
        let n = run.codes.len();
        assert_eq!((&cpu.positions[0][..n.min(cpu.positions[0].len())], &cpu.positions[1][..n.min(cpu.positions[1].len())], &cpu.codes[..]), (&starts[..], &lasts[..], &run.codes[..]), "the CPU's tier 1, {text:?}\n{}", c.explain(text));
        // The CPU's tier 2, over the interpreter's tier 1 and over its own: the lean parser's.
        let nodes = c.parser.tier2.run(bytes, &run).unwrap_or_else(|why| panic!("the CPU's tier 2 refuses {text:?}: {why:?}\n{}", c.explain(text)));
        let over = rerail_compiler::stream::image(&nodes, &c.program.readings.extras, TAG, bytes);
        assert_eq!(image_text(&over).expect("a tape image"), want, "the CPU's tier 2 over the interpreter's tier 1, {text:?}");
        let image = c.parser.image(bytes).unwrap_or_else(|why| panic!("the CPU refuses {text:?}: {why:?}\n{}", c.explain(text)));
        assert_eq!(image_text(&image).expect("a tape image"), want, "the CPU, {text:?}");
        // Tier 1 and tier 2 as Rust.
        assert_eq!(tier1_differs(c, text), None, "tier 1 as Rust, {text:?}");
        assert_eq!(aot_text(text).ok().as_ref(), Some(&want), "the parser as Rust, {text:?}");
    }
}

/// Texts the grammar refused and the lean parser reads: a `when` entry ended by nothing or by
/// `;`, a parameter named by a modifier's word (`vararg`, `noinline`, `crossinline`, `reified`),
/// a lambda's trailing comma, a header ended by an `import` on its line, a string over a line,
/// and a `\$` in a raw string.
#[test]
fn small_readings_are_the_lean_parsers() {
    same(&[
        "val x = when (y) { else -> 1 }\n",
        "val x = when (y) { 1 -> a 2 -> b }\n",
        "val x = when (y) { 1 -> a; 2 -> b }\n",
        "val x = when (y) { 1 -> a; }\n",
        "val x = when (y) {\n    1 -> a; // c\n    2 -> b\n}\n",
        "val x = when { a -> { b } else -> c }\n",
        "fun f(noinline: Int) {}\n",
        "fun f(vararg: Int, noinline b: () -> Unit) {}\n",
        "fun f(vararg a: Int, vararg @A b: Int) {}\n",
        "val vararg = 1\n",
        "fun f() { noinline(1) }\n",
        "class A(val vararg: Int)\n",
        "class A(vararg val a: Int)\n",
        "val reified = 1\n",
        "inline fun <reified T : Any> g(reified: T) = reified\n",
        "fun <reified> f() {}\n",
        "class A<reified out T>\n",
        "val f = { a, -> a }\n",
        "val f = { a, b, -> a }\n",
        "val f = { (a, b), -> a }\n",
        "import a.b import c\n",
        "package a import b\n",
        "import a.b as c import d.*\n",
        "import a.b; import c\n",
        "val s = \"a\nb\"\n",
        "val s = \"a ${b}\nc\"\n",
        "val s = \"a $b\nc\"\n",
        "val s = \"\n\"\nval t = 1\n",
        "val h = \"\"\"\\$a\"\"\"\n",
        "val h = \"\"\"a\\$b c\"\"\"\n",
        "val h = \"\"\"\\\\$a\"\"\"\n",
        "val h = \"\"\"\\${a}\"\"\"\n",
        // A line break in a template's code.
        "val s = \"a ${\n    b\n} c\"\n",
        "val s = \"a ${\n    f(\n        x\n    )\n} | c\"\n",
        "val s = \"a ${b\n}\"\n",
        "val s = \"a ${\nb}\"\n",
        "val s = \"a ${ b\n    .c }\"\n",
        "val s = \"\"\"a ${\n    b\n} c\"\"\"\n",
        "val s = \"a ${b +\n c}\"\n",
        "val s = \"a ${b\n + c}\"\n",
        "val s = \"a ${b\n?: c}\"\n",
        "val s = \"a ${if (x)\n 1 else 2}\"\n",
        "val s = \"a ${b\n as C}\"\n",
        "fun f() {\n    g(\"a ${\n        b\n    }\")\n    h()\n}\n",
    ]);
    refused(&["val h = \"\"\"a\\$$b\"\"\"\n", "val h = \"\"\"a\\$\"\"\"\n", "val s = `a\nb`\n"]);
}

/// Braces in a template's code (decision 0621): the tail opens at the `}` that closes the
/// template, as the lean lexer's `interpolation_end` counts them; a block of more than 4 levels
/// is refused.
#[test]
fn braces_in_templates_are_the_lean_parsers() {
    same(&[
        "val s = \"a ${x.f { it.name }} b\"\n",
        "val s = \"${if (p) { \"granted\"} else { \"denied\" }}\"\n",
        "val s = \"${if (a) { 1 } else 2}\"\n",
        "val s = \"${ {x} }\"\n",
        "val s = \"${x.joinToString(\",\") { it.name }}\"\n",
        "val s = \"\"\"${a.map { b -> { c } }}\"\"\"\n",
        "fun f() { val s = \"${g { 1 }}\"; h { 2 } }\n",
        "val s = \"${a}\" + \"${b {}}\"\n",
        "val s = \"${a}\"; val t = { \"${b}\" }\n",
        "val s = \"${a { b { c { d } } }} e\"\n",
        "val s = \"${a { \"}\" }} e\"\n",
        "val s = \"${a { '}' }} e\"\n",
        "val s = \"${a {\n    b\n}} e\"\n",
        "fun f() {\n    log(\"x ${g.joinToString(\",\") { it.name }}\")\n}\n",
    ]);
    refused(&["val s = \"${a {{{{ b }}}} }\"\n"]);
}

/// Templates and comments in a template's code (decision 0621): each string's record open over
/// its template's code, innermost last, as the lean lexer's `interpolation_end` recurses into a
/// string and a comment; across a block's edge too.
#[test]
fn templates_and_comments_in_templates_are_the_lean_parsers() {
    let texts = [
        "val s = \"${f(\"${x}\")}\"\n",
        "val s = \"a ${f(\"b ${x} c\")} d\"\n",
        "val s = \"${\"${\"${x}\"}\"}\"\n",
        "val s = \"\"\"${f(\"${x}\")} y\"\"\"\n",
        "val s = \"${f(\"\"\"${x}\"\"\")} y\"\n",
        "val s = \"${a { \"${b}\" }} c\"\n",
        "val s = \"${f(\"${x}\") + g(\"${y}\")} z\"\n",
        "val s = \"${f(\"${x}\")}\" + \"${y}\"\n",
        "val s = \"${f(\"${x} $y\")} ${z}\"\n",
        // Comments in a template's code, after a template in it too.
        "val s = \"${f(\"${x}\") /* c */} z\"\n",
        "val s = \"${f(\"${x}\") // c\n} z\"\n",
        "val s = \"${f(x) /* c */} z\"\n",
        "val s = \"${f(x) // c\n} z\"\n",
        "val s = \"${/* c */ x}\"\n",
        "val s = \"${f(x, /* c */ y)} z\"\n",
        "val s = \"${f(\n    // c\n    x\n)} z\"\n",
        "val s = \"${a { /* c */ b }} z\"\n",
        "val s = \"${x /* } */} z\"\n",
        "val s = \"${x // }\n} z\"\n",
        "val s = \"\"\"${x /* c */} y\"\"\"\n",
        "val s = \"${x /* a */ /* b */} z\"\n",
        "val s = \"${x\n// c\n} z\"\n",
        "val s = \"${if (a) b /* c */ else c}\"\n",
    ];
    let line = "val v = \"a ${f(\"b ${x} c\")} d\"";
    let edges: Vec<String> = (58..70).map(|q| format!("//{}\n{line}\n", "x".repeat(q))).collect();
    same(&texts.iter().copied().chain(edges.iter().map(String::as_str)).collect::<Vec<_>>());
}

/// Block comments after a line break, one after another (`semi`'s `loses since`): the first loses
/// its bytes, at the next one's start, as the lean lexer's `asi` reads it through to another
/// comment; the next is the line's, and a line break after it decides at the token first on its
/// line (`own_line`), as after any token; a comment of its own line before the token is the
/// line's, neither dropped nor lost.
#[test]
fn block_comments_one_after_another_are_the_lean_parsers() {
    same(&[
        "val a = 1\n/* c */ /* d */\nexternal fun f()\n",
        "val a = 1\n/* c */ /* d */ /* e */\nval b = 2\n",
        "val a = 1\n/* c */ /* d */\n+ 2\n",
        "val a = 1\n/* c */\n/* d */\nexternal fun f()\n",
        "val a = 1\n/* c */ /* d */\n/* e */\nabstract class A\n",
        "val a = 1\n/* c */ /* d */\n.f()\n",
        "val a = 1\n/* c */ /* d */ // e\nval b = 2\n",
        "val a = 1\n/* c */\n/* d */\n.f()\n",
        "fun f() {\n    if (a) b()\n    /* c */\n    /* e */\n    else d()\n}\n",
        "fun f() {\n    if (a) b()\n    /* c */ /* e */\n    else d()\n}\n",
        "fun f() {\n    g()\n    /* c */\n    /* d */\n    h()\n}\n",
        "class A {\n    val b = 1\n    /* c */ /* d */\n    abstract fun e()\n}\n",
    ]);
}

/// What the grammar does not read: refused, and sent to the lean parser.
#[test]
fn what_the_grammar_does_not_read_is_refused() {
    refused(&[
        // A token on the line of a block comment after one that starts the line: the lean
        // lexer's `asi` inserts no semicolon there, which a mark's questions can not say.
        "val a = 1\n/* c */ /* d */ + 2\n",
        "fun f() {\n    a\n    /* c */ /* d */ .b()\n}\n",
        "val s = 'x\n",
        "val s = \"${}\"\n",
        "val s = \"open\n",
        "/* /* nested */ */\npackage a\n",
        "/* open\n",
        "val x = 1d\n",
        "val x = 1.5L\n",
        "val x = 1_\n",
        "val x = 0x\n",
        "class A { val x = 1 }\n",
    ]);
}

/// A visibility modifier and `constructor` on two lines after a class's name: the lean lexer
/// reads them on one line only (`modifier_then_constructor`), and refuses (a token's `without`).
#[test]
fn a_constructor_off_its_modifiers_line_is_refused() {
    refused(&["class A\nprivate\nconstructor(x: Int)\n", "class A\n@Inject private\nconstructor(x: Int)\n"]);
}

/// A generic call after a tighter operator's operand: over the whole operation, as the lean
/// parser's `expr_in` reads it (its own test's texts, `generic_calls_over_tighter_operations`).
#[test]
fn generic_calls_over_tighter_operations_are_the_lean_parsers() {
    same(&[
        "val a = b ?: f<T>()!!\nval c = \"x\" to g<String>(y)\nval d = -h<T>()\nval e = p < q<T>()\nval f = p + q<T>()\nval g = p == q<T>()\nval h = x in q<T>()\nval i = !q<T>()\nval j = p * q<T>().z\n",
        "val a = b to f<T>()\nval b = x + f<T>(1)\nval c = x < f<T>(1)\n",
        "class A {\n    private val m by lazy { m ?: getSystemService<M>()!! }\n}\n",
        "val a = mapOf(\"a\" to mutableListOf<String>(\"456\"), \"b\" to c)\n",
        "val a = x && y == f<T> { z }\nval b = p || q as R\n",
    ]);
}

/// An accessor alone at a statement's level, a property's on the line after: the lean parser's
/// `declaration_ahead` reads it there too.
#[test]
fn accessors_on_their_own_line_are_the_lean_parsers() {
    same(&[
        "val x: Int\n    get() = 1\n",
        "val x: Int\n    get() = this is A\nval y = 2\n",
        "interface A {\n    val b: Boolean\n        get() = c\n}\n",
        "var x: Int = 0\n    private set\n",
        "fun f() {\n    val get = 1\n    get + 1\n}\n",
        "fun f() {\n    get(1)\n    set\n    get\n    x = get ?: 2\n}\n",
        "class A {\n    var b = 1\n        set\n    val c get() = 2\n}\n",
    ]);
}

/// Types as the lean parser's `ty` reads them: annotations and `suspend` before any type, a
/// function type where `->` follows its `)`, else a parenthesized type.
#[test]
fn types_are_the_lean_parsers() {
    same(&[
        "fun f(): @Nullable A = g()\nval s: Set<@JvmSuppressWildcards C> = d\n",
        "val f: (Int) -> Int = g\nval h: ((Int) -> Int)? = null\nval k: suspend () -> Unit = l\n",
        "fun f(g: ((Int) -> Int) -> Int) {}\nval m: @A suspend (B) -> C = n\nval p: (A) = q\n",
        "val r: suspend A.() -> B = s\nval t: Map<String, (Int) -> Unit> = u\n",
    ]);
}

/// Labels: a loop's before it, a trailing lambda's, and an expression's (`prefix_expression`), as
/// the lean parser reads them.
#[test]
fn labels_are_the_lean_parsers() {
    same(&[
        "fun f() {\n    loop@ for (i in a) break@loop\n    outer@ while (b) { continue@outer }\n}\n",
        "fun f() {\n    set.forEach loop@{ value ->\n        return@loop\n    }\n    g(lit@{ 1 })\n}\n",
    ]);
}

/// An assignment's target, flat as the lean parser's `assignable` reads it: its `!!` a leaf.
#[test]
fn assignment_targets_are_the_lean_parsers() {
    same(&["fun f() {\n    stringPool!![s] = offset\n    a!!.b = c\n    d.e!! += 1\n}\n"]);
}

/// A line break inside brackets is no semicolon: the lean parser asks `asi` only at a statement's
/// level (`level(false, …)`).
#[test]
fn line_breaks_in_brackets_are_the_lean_parsers() {
    same(&["fun f() {\n    g(\n        a + \"b\"\n            + \"c\",\n        d)\n    val e = (h\n        - i)\n}\n"]);
}

/// The type after `as`, as the lean parser's `ty`: a parenthesized function type too.
#[test]
fn as_types_are_the_lean_parsers() {
    same(&[
        "val a = (b as ((C) -> Unit)).invoke(1)\n",
        "val a = b as ((C) -> Unit)\n",
        "val a = b as @A C\n",
        "val a = b as suspend () -> Unit\n",
    ]);
}

/// Delegation specifiers, as the lean parser's `delegation_specifiers`: `suspend` and a function
/// type, a function type with a receiver, a parenthesized one, a constructor's call, a delegate.
#[test]
fn delegation_specifiers_are_the_lean_parsers() {
    same(&[
        "class Block : suspend LifecycleOwner.(View) -> Unit {\n}\n",
        "class Block : LifecycleOwner.(View) -> Unit {\n}\n",
        "class B : (Int) -> Unit, C(), D by e\n",
        "class B : suspend (Int) -> Unit\n",
    ]);
}

/// A class parameter's modifiers: any modifier a name follows (`vararg` among them).
#[test]
fn class_parameter_modifiers_are_the_lean_parsers() {
    same(&[
        "class X(a: Map<String, String>? = null, vararg args: Any?)\n",
        "class X(private vararg val args: Int, @A override val b: B)\n",
    ]);
}

/// Two templates in a block, the second's code across the block's end: the JIT's tier 1 reads
/// the first again, its string's record open (decision 0600's slot) after it.
#[test]
fn templates_read_again_across_a_block_are_the_lean_parsers() {
    let l1 = "val a = \"${A.B}\"\n";
    let head = format!("//{}\n", "x".repeat(67));
    let texts: Vec<String> = (122..127).map(|at| format!("{head}{l1}//{}\nval b = \"${{C.D}}\"\n", "x".repeat(at - head.len() - l1.len() - 11))).collect();
    assert_eq!(texts[2].find("\"${C").expect("the second template"), 124);
    same(&texts.iter().map(String::as_str).collect::<Vec<_>>());
}

/// `x as a.b.C<D>` (`as?`, `is`) where `>` ends its line: tree-sitter's reading, as the lean
/// parser's `split_type`: navigations over `x as a`, then a call's type arguments before `(`
/// or `{` on the next line, or comparisons before an operand there; elsewhere the type whole.
/// Where the lean parser's trial read decides it on what the grammar does not ask (an
/// assignment on the next line), before a string there, or past a blank or a comment after
/// the `>`, the file is refused.
#[test]
fn as_splits_are_the_lean_parsers() {
    same(&[
        "fun f() {\n    val d = x as java.util.List<Int>\n    g(d)\n}\n",
        "fun f() {\n    val d = x as java.util.List<Int>\n    (g)\n}\n",
        "fun f() {\n    val d = x as java.util.List<Int>\n    { g }\n}\n",
        "fun f() {\n    val d = x as java.util.List<Int>\n    val e = 1\n}\n",
        "fun f() {\n    val d = x as java.util.List<Int>\n}\n",
        "val d = x as java.util.List<Int>\n",
        "fun f() {\n    val d = x as java.util.List<Int>\n    a.b()\n}\n",
        "fun f() {\n    val d = x as java.util.List<Int>; g(d)\n}\n",
        "fun f() {\n    g(x as java.util.List<Int>)\n}\n",
        "fun f() {\n    val d = x as? java.util.List<Int>\n    g(d)\n}\n",
        "fun f() {\n    val d = x as java.util.Map<Int, Int>\n    g(d)\n}\n",
        "fun f() {\n    val d = x as java.util.List<Int>\n    assertFailsWith<E> { d.add(4) }\n}\n",
        "fun f() {\n    val d = x as java.util.List<Int>\n    return d\n}\n",
        "fun f() {\n    val d = x is a.B<C>\n    g(d)\n}\n",
        "fun f() {\n    val d = x is a.B<C>\n    val e = 1\n}\n",
        "fun f() {\n    val d = x !is a.B<C>\n    val e = 1\n}\n",
        "fun f() {\n    if (x is a.B<C>) g()\n}\n",
        "fun f() {\n    val d = x !is a.B<C>\n    g(d)\n}\n",
        "fun f() {\n    val d = x ! is a.B<C>\n    g(d)\n}\n",
        "fun f() {\n    val d = x as a.b >\n    c\n}\n",
        "fun f() {\n    g(\n        x as java.util.List<Int>\n    )\n}\n",
        "fun f() {\n    val d = x as java.util.List<Int>\n        .size\n}\n",
        "fun f() {\n    val d = x as java.util.List<Int>\n    if (d) g()\n}\n",
    ]);
    refused(&[
        "fun f() {\n    val d = x as java.util.List<Int>  \n    g(d)\n}\n",
        "fun f() {\n    val d = x as java.util.List<Int> // c\n    g(d)\n}\n",
        "fun f() {\n    val d = x as java.util.List<Int>\r\n    g(d)\n}\n",
        "fun f() {\n    val d = x as java.util.List<Int>\n    a = b\n}\n",
        "fun f() {\n    val d = x as java.util.List<Int>\n    \"s\".f()\n}\n",
    ]);
}

/// A `?.` after a type is its `?` and a `.` (a nullable receiver, `x as T?.f`, `as?.`), and a
/// `>=` that closes type arguments its `>` and a `=`, as the lean parser's `split_at` reads them;
/// a safe call's `?.` and a comparison's `>=` are one leaf, and apart they are no operator.
#[test]
fn split_operators_are_the_lean_parsers() {
    same(&[
        "fun A?.f() {}\n",
        "fun String?.h() = 1\n",
        "val A?.b: Int get() = 1\n",
        "fun <T> T?.g(c: T.() -> Unit) {}\n",
        "fun A.B<C>?.f() {}\n",
        "val a = b?.c?.d()\n",
        "val a = b\n    ?.c\n",
        "val a = (x as? T)?.y\n",
        "val a = b?.let { it }?:c\n",
        "val a: List<Int>= x\n",
        "fun f(): Map<A, B>= m\n",
        "val a = b >= c\n",
        "val a = if (b>=c) 1 else 2\n",
        "val a = b<c>=d\n",
    ]);
    for text in ["val a = b ? .c\n", "val a = b > = c\n", "val a = b >\n  = c\n"] {
        assert!(lean_text(text).is_none(), "the lean parser reads {text:?}");
        assert!(compiled().expect("compiles").parser.image(text.as_bytes()).is_err(), "the grammar reads {text:?}");
    }
}

/// A block comment first on a line where a semicolon may come, then another comment: the lean
/// lexer's `asi` reads the block comment through and loses its bytes (no width, at the next
/// comment's start), and its semicolon is past that comment (an import header's holds both).
#[test]
fn a_block_comment_before_a_comment_is_the_lean_parsers() {
    same(&[
        "fun f() {\n  a()\n  /* x */\n\n  // c\n  b()\n}\n",
        "fun f() {\n  a()\n  /* x */ // c\n  b()\n}\n",
        "fun f() {\n  a()\n  /* x */\n  // c\n  .b()\n}\n",
        "fun f() {\n  a()\n  /* x */\n  // c\n  // d\n  b()\n}\n",
        "fun f() {\n  a()\n  /* x */\n  // c\n  /* y */\n  b()\n}\n",
        "fun f() {\n  a()\n  /* x */\n  // c\n  /* y */\n  .b()\n}\n",
        "class A {\n  val a = 1\n  /* x */\n  // c\n  fun b() {}\n}\n",
        "val x = 1\n\n/** d */\n// c\n\nprivate val u = 1\n",
        "import a.b\n\n/** d */\n// c\n\nprivate val u = 1\n",
        "import a.b\n\n/** d */\n// c\n// e\n\nprivate val u = 1\n",
        "package a\n/* x */\n// c\nimport b\n",
    ]);
    // The tape's blocks between the two comments, at each node count: a name a node.
    let texts: Vec<String> = (0..rerail_tape_format::CHECKPOINT_STRIDE).map(|k| format!("fun f() {{\n{}  /* x */\n  // c\n  d()\n}}\n", "  a\n".repeat(k))).collect();
    same(&texts.iter().map(String::as_str).collect::<Vec<_>>());
}

/// A file's leading blanks are no node's: the root starts at its first token or comment, at
/// the file's end where it has none, as tree-sitter's root starts past its padding.
#[test]
fn leading_blanks_are_the_lean_parsers() {
    same(&["\n", "  \n", "\n// c\n", "\nval a = 1\n", "  val a = 1\n", "\n\n/* c */ val a = 1\n"]);
    // Over many of the tape's blocks, which start at their first node's offset.
    let long = format!("\n\n{}", "val a = f(b, c) // d\n".repeat(600));
    same(&[&long]);
}

/// A block comment's `*/` is past its `/*` (`/*/` is open), and a raw string opened by more quotes
/// than three.
#[test]
fn comment_ends_are_the_lean_parsers() {
    same(&["val a = 1 /*/ x */ + 2\n", "val a = 1 /**/ + 2\n", "/*/ x\n */\nval b = 1\n"]);
}

/// `where` constraints, a function's, a property's and a class's.
#[test]
fn type_constraints_are_the_lean_parsers() {
    same(&[
        "fun <T> f(x: T): T where T : A, T : B = x\n",
        "class C<S> : D() where S : PageScope {\n}\n",
        "val <T> T.x: Int where T : A get() = 1\n",
        "fun <T> g() where T : @A B {}\n",
        "val <T : Any> List<T>.second: T get() = this[1]\n",
    ]);
}

/// `fun interface`: a class, as the lean parser's `declaration_ahead` reads it.
#[test]
fn fun_interfaces_are_the_lean_parsers() {
    same(&[
        "fun interface A {\n    fun f()\n}\n",
        "private fun interface P<T> : Q {\n    fun f(): T\n}\nfun g() = 1\nfun interface R\n",
    ]);
}

/// `by` on the next line: a delegate where a property's or a supertype's is asked (the lean
/// lexer's `asi` with `cx.by`), else a new statement's first name.
#[test]
fn by_on_the_next_line_is_the_lean_parsers() {
    same(&[
        "fun f() {\n    a {\n        size_ = true\n        by = \"foo\"\n    }\n}\n",
        "class A {\n    val m\n        by lazy { 1 }\n}\n",
        "class A : I\n    by d {\n}\n",
        "val x = 1\nby(2)\n",
    ]);
}

#[test]
fn soft_names_are_the_lean_parsers() {
    same(&[
        "fun f(constructor: C) {\n    val constructor = g()\n}\n",
        "val x = f(by = \"foo\")\n",
        "val x = when (c) {\n    !in A..B -> 1\n    !is C -> 2\n    else -> 3\n}\n",
        "val y = a !in b && c !is D\n",
        "class A private constructor(x: Int) : B by x {\n    constructor() : this(1)\n    val m by lazy { 1 }\n}\n",
    ]);
}

#[test]
fn names_after_dots_are_the_lean_parsers() {
    same(&[
        "fun f(limit: W) = limit.value\n\ninternal operator fun g() {}\n",
        "val a = b.data\nprivate fun c() {}\n",
        "internal fun f() {}\n",
        "private val x = 1\n",
    ]);
}

#[test]
fn constructor_lines_are_the_lean_parsers() {
    same(&[
        "class A {\n    private constructor(x: Int) : this()\n}\n",
        "class A {\n    val x = 1\n    private constructor(x: Int) : this()\n}\n",
        "class A {\n    fun f() {}\n\n    @Inject internal constructor(x: Int)\n}\n",
        "class A\ninternal constructor(x: Int) {\n}\n",
        "class A\n@Inject public constructor(x: Int)\n",
        "class A\n@Inject constructor(x: Int)\n",
        "class A\n\ninternal class B private constructor(x: Int)\n",
        "class A\n@Inject\nclass B @Inject constructor(x: Int)\n",
        "class A\nprivate fun f() {}\n",
        "class A\n@Named(\"a(\") @B.C(1, f(2)) constructor(x: Int)\n",
        "class A<T>\nconstructor(x: T)\n",
        "enum class E\nconstructor(val v: Int) { X(1) }\n",
    ]);
}

#[test]
fn soft_words_in_templates_are_the_lean_parsers() {
    same(&[
        "val s = \"a $value b\"\n",
        "val s = \"$data\"\n",
        "val s = \"$values $field.x\"\n",
        "val s = \"\"\"$get x\"\"\"\n",
        "val s = \"$constructor $value\"\n",
        "val s = \"a $field b\"\n",
    ]);
}

#[test]
fn swallowed_comments_are_the_lean_parsers() {
    same(&[
        // A line comment after an annotation, before the line break the annotation takes.
        "@Suppress(\"unused\") // the unused type parameter\nclass A\n",
        "class A {\n    @Test  // SPR-16412\n    fun f() {}\n}\n",
        "abstract class A {\n    val x = 1\n    /* c */\n    abstract fun f()\n}\n",
        "fun f() {\n    a()\n    /* c */\n    when (x) {}\n}\n",
        "fun f() {\n    a()\n    /** c\n     * d */\n    error(1)\n}\n",
        "class A {\n    /* c */\n    abstract fun f()\n}\n",
        "val a = 1\n/* c */\nexternal fun f()\n",
        "abstract class A {\n    fun f() {}\n    /* c */\n    abstract fun g()\n}\n",
        "fun f() {\n    val s = \"a\"\n    /* c */\n    error(s)\n}\n",
        "fun f() {\n    a[1]\n    /* c */\n    error(s)\n}\n",
    ]);
}

#[test]
fn annotated_expressions_are_the_lean_parsers() {
    same(&[
        "fun f() {\n    @Suppress(\"D\")\n    if (a) b()\n}\n",
        "fun f() {\n    @Suppress(\"D\") if (a) b() else c()\n    @Suppress(\"D\")\n    val x = 1\n}\n",
        "fun f() {\n    @Suppress(\"D\", \"E\")\n    when (a) {\n        1 -> b()\n    }\n}\n",
        "val x = when (a) {\n    B,\n    C,\n    -> d\n    else -> e\n}\n",
        "val x = when (a) {\n    B, C, -> d\n    in r -> e\n    else -> f\n}\n",
        // The run to the `)` passes line breaks (Q340): a declaration after is still one.
        "fun f() {\n    @A(x) if (y) z\n    val q = 1\n}\n",
        // An annotation before any expression, as the lean parser's `declaration_ahead` finds no
        // declaration (Q342); and before a declaration's modifiers and keyword.
        "fun f() {\n    @A if (y) z\n}\n",
        "fun f() {\n    @A(x) foo()\n}\n",
        "fun f() {\n    @A foo()\n    val q = 1\n}\n",
        "fun f() {\n    @Suppress(\"D\")\n    get(a, b)\n    val x = @A @B g()\n    @A.B(1) h\n}\n",
        "fun f() {\n    @A private fun g() {}\n    @field:Inject lateinit var x: Int\n    @A(1, \"b\") val y = 1\n    @A @B(c) class C\n}\n",
    ]);
}

#[test]
fn function_types_are_the_lean_parsers() {
    same(&[
        "val h: HTML.() -> Unit = {}\n",
        "fun f(w: A.B.() -> Unit) {}\n",
        "fun f(w: suspend () -> T) {}\n",
        "fun f(b: J.() -> Unit = {}, w: suspend () -> T, x: M<K>.(Int) -> Unit) {}\n",
        "fun A.B.f(x: C.() -> D): E.() -> F = x\n",
    ]);
}

#[test]
fn delegates_and_names_are_the_lean_parsers() {
    same(&[
        "fun f() {\n    val state by viewModel.uiState.collectAsState()\n}\n",
        "fun g(\n    set: AttributeSet,\n    get: Int,\n) {}\n",
    ]);
}

#[test]
fn variance_and_accessors_are_the_lean_parsers() {
    same(&[
        "fun f(a: Array<out Any>) {}\n",
        "val p: P<in T> = x\n",
        "class A<out T>\n",
        "class B<in T, reified out C>\n",
        "val x = out\n",
        "fun f(out: Int) = out\n",
        "fun f() {\n    System.out.println(1)\n}\n",
        "fun f() {\n    a()\n    get(i).g(o)\n}\n",
        "fun f() {\n    set.add(1)\n}\n",
        "fun f() {\n    get(1)\n}\n",
        "class A {\n    val x: Int\n        get() = 1\n    var y = 2\n        set(v) {\n            field = v\n        }\n}\n",
        "class A {\n    private val t = this::class.simpleName\n    private val m by lazy { n }\n}\n",
    ]);
}

#[test]
fn class_delegates_are_the_lean_parsers() {
    same(&[
        "class A(d: C) : C by d {\n}\n",
        "class A(d: C) : C by d {\n    fun f() = 1\n}\n",
        "class A(m: M) : M by m, B {\n}\n",
        "class A : C by D() {\n}\n",
        "class A(d: C) : C by d\n",
        "class A {\n    val b by lazy { 1 }\n}\n",
        "fun f() = object : I by itr {\n    override fun g() = 1\n}\n",
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
    let mut aot = (0, 0, 0);
    let mut tier1 = 0;
    let mut paths: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
    paths.sort();
    for p in &paths {
        let Ok(text) = std::fs::read_to_string(p) else { continue };
        let Some(want) = lean_text(&text) else {
            lean_refused += 1;
            assert!(c.image(&text).is_none(), "{}: accepted a file the lean parser refuses", p.display());
            continue;
        };
        // Tier 1 as Rust: the interpreter's records.
        if let Some(why) = tier1_differs(c, &text) {
            tier1 += 1;
            if tier1 <= 10 {
                eprintln!("tier 1 as Rust differs: {}: {why}", p.display());
            }
        }
        // The parser as Rust: the lean parser's tape, or refused.
        match aot_text(&text) {
            Ok(t) if t == want => aot.0 += 1,
            Ok(t) => {
                aot.2 += 1;
                // Where: the first line that differs, with the lines before it.
                let (have, wanted): (Vec<&str>, Vec<&str>) = (t.lines().collect(), want.lines().collect());
                let at = have.iter().zip(&wanted).position(|(a, b)| a != b).unwrap_or(have.len().min(wanted.len()));
                let from = at.saturating_sub(6);
                eprintln!("the parser as Rust differs: {}\n  context:\n    {}\n  the parser as Rust:\n    {}\n  lean:\n    {}", p.display(), wanted[from..at].join("\n    "), have[at..(at + 4).min(have.len())].join("\n    "), wanted[at..(at + 4).min(wanted.len())].join("\n    "));
            }
            Err(_) => aot.1 += 1,
        }
        match c.image(&text) {
            None => {
                refused += 1;
                if std::env::var_os("RERAIL_KOTLIN_WHY").is_some() {
                    // The reference evaluator's first escape or error; else the grammar refused.
                    let stmts = ::bitlex::syntax::parse(crate::GRAMMAR).expect("parses");
                    let tables = Default::default();
                    let why = match ::bitlex::eval::Ev::run(&stmts, text.as_bytes(), &tables) {
                        Ok(out) => match out.errors.first() {
                            // `RERAIL_KOTLIN_WHY=bytes`: with the byte, and the text there.
                            Some(e) if std::env::var("RERAIL_KOTLIN_WHY").as_deref() == Ok("bytes") => {
                                let at = e.split(" (byte ").nth(1).and_then(|r| r.split(|c: char| !c.is_ascii_digit()).next()).and_then(|n| n.parse::<usize>().ok());
                                let near = at.filter(|&a| text.is_char_boundary(a)).map_or(String::new(), |a| text[a.saturating_sub(60).min(a)..].chars().take(90).collect::<String>().replace('\n', "⏎"));
                                format!("{e} `{near}`")
                            }
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
    eprintln!("the parser as Rust: {} the same, {} refused, {} differ; tier 1's records differ in {tier1}", aot.0, aot.1, aot.2);
    assert_eq!(differ, 0);
    assert_eq!(aot.2, 0, "the parser as Rust: a wrong tape");
    assert_eq!(tier1, 0, "tier 1 as Rust: records not the interpreter's");
}

/// The lean parser's tape for each file in `RERAIL_KOTLIN_SHOW` (paths, `:`-separated), and the
/// grammar's or why it refuses: the reference the grammar is written against.
#[test]
#[ignore = "a tool: set RERAIL_KOTLIN_SHOW"]
fn show() {
    let c = compiled().expect("compiles");
    let r = c.program.block_programs[0].records.as_ref();
    eprintln!("tier 2: {} words; the code holds {:?} marks and {:?} bits in all", c.program.words.len(), r.map(|r| r.mark_bits()), r.map(|r| r.marks.len()));
    // `RERAIL_KOTLIN_GAPS`: the gap patterns (`Words::gap_patterns`), each entry's elements.
    if std::env::var_os("RERAIL_KOTLIN_GAPS").is_some() {
        for (i, m) in c.program.gap_patterns.iter().enumerate() {
            eprintln!("gap pattern {i}: order {} unless {} {:?}", m.order, m.unless, m.elems.iter().map(|e| (e.offset, e.kinds, &e.words, e.not, e.end, e.with, e.without)).collect::<Vec<_>>());
        }
    }
    // `RERAIL_KOTLIN_WORDS=PC`: tier 2's words from 20 lines before its to 20 after.
    if let Ok(pc) = std::env::var("RERAIL_KOTLIN_WORDS") {
        let lines: Vec<&str> = c.words.lines().collect();
        let at = lines.iter().position(|l| l.trim_start().starts_with(&format!("{pc:0>4}"))).unwrap_or(0);
        for l in &lines[at.saturating_sub(20)..(at + 20).min(lines.len())] {
            eprintln!("  {l}");
        }
    }
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
                        if std::env::var_os("RERAIL_KOTLIN_RULES").is_some() {
                            let b = &c.program.block_programs[0];
                            let run = b.run(text.as_bytes());
                            let cpu = c.parser.tier1.extract(text.as_bytes());
                            eprintln!("rules {:?}; ends {:?}; kind of 1 {} 0 {}", r.rules, r.ends, r.kinds[usize::from(b'1')], r.kinds[usize::from(b'0')]);
                            for rule in &r.rules {
                                if let rerail_compiler::records::EndRule::Last(o) = rule {
                                    let w = |v: &Vec<Vec<u64>>| v.get(usize::from(*o)).and_then(|x| x.first()).map(|x| format!("{:016b}", x.reverse_bits() >> 48));
                                    eprintln!("last {o}: interp {:?} cpu {:?}", w(&run.outputs), w(&cpu.outputs));
                                }
                            }
                        }
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
                        let cpu = c.parser.tier1.extract(text.as_bytes());
                        for (i, (x, y)) in run.positions.iter().zip(&cpu.positions).enumerate() {
                            if x != y {
                                eprintln!("positions {i}: interp {x:?}\n             cpu {y:?}");
                            }
                        }
                        eprintln!("totals: interp {:?} cpu {:?}", run.totals, cpu.totals);
                        {
                            let b = &c.program.block_programs[0];
                            for rd in r.readers.iter().filter(|rd| !rd.parts.is_empty()) {
                                let a = &rd.arms[0];
                                eprintln!("reader opener {} delay {} closer {} delay {}", a.opener, b.output_delays[usize::from(a.opener)], a.closer, b.output_delays[usize::from(a.closer)]);
                            }
                        }
                        if run.codes != cpu.codes {
                            eprintln!("codes: interp {:?}\n       cpu {:?}", run.codes, cpu.codes);
                        }
                        for rd in r.readers.iter().filter(|rd| !rd.parts.is_empty()) {
                            for o in rd.parts.iter().flat_map(|p| std::iter::once(p.start).chain(p.last)).chain([rd.arms[0].opener, rd.arms[0].closer]).chain(rd.arms[0].last) {
                                let (a, b) = (&run.outputs[usize::from(o)], cpu.outputs.get(usize::from(o)));
                                for (i, x) in a.iter().enumerate() {
                                    let y = b.and_then(|v| v.get(i)).copied();
                                    if y != Some(*x) {
                                        eprintln!("output {o} word {i}: interp {:064b}\n                       cpu {:?}", x.reverse_bits(), y.map(|y| format!("{:064b}", y.reverse_bits())));
                                    }
                                }
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
                    // `RERAIL_KOTLIN_PRINT`: the tape too.
                    if std::env::var_os("RERAIL_KOTLIN_PRINT").is_some() {
                        eprintln!("{lean}");
                    }
                } else {
                    eprintln!("== {p}: DIFFERS\n-- lean\n{lean}-- grammar\n{got}");
                    if std::env::var_os("RERAIL_KOTLIN_TOKENS").is_some() {
                        eprintln!("-- words\n{}", c.explain(&text));
                    }
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
/// file than the crate's).
#[test]
#[ignore = "a measurement: run by hand"]
fn program_size() {
    let text = match std::env::var("RERAIL_KOTLIN_GRAMMAR") {
        Ok(p) => std::fs::read_to_string(p).expect("the grammar file"),
        Err(_) => crate::GRAMMAR.to_owned(),
    };
    let c = crate::compile_timed(&text, crate::shipped_answers(), &mut |_, _| {}).expect("the grammar compiles");
    let lines: Vec<&str> = c.words.lines().collect();
    let mut by_op: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for l in &lines {
        if let Some(op) = l.split_whitespace().find(|w| w.chars().all(|c| c.is_ascii_uppercase() || c == '_') && w.len() > 1) {
            *by_op.entry(op).or_default() += 1;
        }
    }
    let mut ops: Vec<_> = by_op.into_iter().collect();
    ops.sort_by_key(|o| std::cmp::Reverse(o.1));
    eprintln!("{} lines; by opcode: {:?}", lines.len(), &ops[..ops.len().min(12)]);
    if let Ok(out) = std::env::var("RERAIL_KOTLIN_WORDS") {
        std::fs::write(out, &c.words).expect("the words written");
    }
    if let Ok(out) = std::env::var("RERAIL_KOTLIN_MACHINE") {
        std::fs::write(out, &c.machine).expect("the machine written");
    }
}

/// The grammar's compile, phase by phase (`compile_timed`): what a JIT pays before its first
/// file. Run on an optimized build (`--profile profiling`).
#[test]
#[ignore = "a measurement: run by hand, on an optimized build"]
fn compile_time() {
    let text = match std::env::var("RERAIL_KOTLIN_GRAMMAR") {
        Ok(p) => std::fs::read_to_string(p).expect("the grammar file"),
        Err(_) => crate::GRAMMAR.to_owned(),
    };
    // Cold, then with the answers the cold compile made (decision 0414). With
    // `RERAIL_KOTLIN_EDIT` (a grammar file): that grammar with the first's answers, then cold.
    let edit = std::env::var("RERAIL_KOTLIN_EDIT").ok().map(|p| std::fs::read_to_string(p).expect("the edited grammar"));
    let mut passes = vec![("cold", &text), ("warm", &text)];
    if let Some(e) = &edit {
        passes = vec![("cold", &text), ("edited, warm", e), ("edited, cold", e)];
    }
    let mut known = crate::Proofs::default();
    let mut prints = Vec::new();
    for (pass, text) in passes {
        eprintln!("{pass}:");
        let mut total = std::time::Duration::ZERO;
        let given = if pass.ends_with("cold") { crate::Proofs::default() } else { std::mem::take(&mut known) };
        let c = crate::compile_timed(text, given, &mut |name, t| {
            total += t;
            eprintln!("  {name:>16}: {:8.1} ms", t.as_secs_f64() * 1e3);
        })
        .expect("the grammar compiles");
        eprintln!("  {:>16}: {:8.1} ms", "total", total.as_secs_f64() * 1e3);
        eprintln!("  {:>16}: {} made", "answers", c.proofs.len());
        eprintln!("  {:>16}: {:016x}", "fingerprint", fingerprint(&c));
        prints.push(fingerprint(&c));
        if known.is_empty() {
            known = c.proofs.iter().cloned().collect();
        }
    }
    assert_eq!(prints[prints.len() - 2], prints[prints.len() - 1], "the known answers make the same program");
}

/// What was compiled, as one number: tier 2's words and machine, and tier 1's program.
fn fingerprint(c: &crate::Compiled) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (&c.words, &c.machine, c.program.block_programs.iter().map(|b| b.canonical_bytes()).collect::<Vec<_>>()).hash(&mut h);
    h.finish()
}

/// The shipped answers (`kotlin.answers`, decision 0414's seed) answer every question the grammar
/// asks: a compile from them alone, not the machine's kept answers, proves nothing.
/// `RERAIL_BLESS_KOTLIN_ANSWERS=1` writes them again, from a cold compile's.
#[test]
fn the_shipped_answers_answer_the_grammar() {
    if std::env::var_os("RERAIL_BLESS_KOTLIN_ANSWERS").is_some() {
        let cold = compile_text(GRAMMAR).expect("the grammar compiles");
        let path = &format!("{}/kotlin.answers", std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
        std::fs::write(path, rerail_compiler::bitlex_prove::encode(&cold.proofs)).expect("kotlin.answers written");
        return;
    }
    let c = crate::compile_timed(GRAMMAR, crate::shipped_answers(), &mut |_, _| {}).expect("the grammar compiles");
    assert!(
        c.proofs.is_empty(),
        "{} questions the shipped answers lack (the grammar or the provers changed): RERAIL_BLESS_KOTLIN_ANSWERS=1 writes kotlin.answers",
        c.proofs.len()
    );
}

/// A cold compile (no answers known) makes the program the shipped answers make, and its answers,
/// given to another compile, leave nothing to ask (decision 0414): the answers decide nothing a
/// proof would not, and the compiler's maps hash with a fixed seed. Cold, so in nextest's slow tier.
#[test]
fn a_cold_compile_makes_the_shipped_answers_program() {
    let cold = compile_text(GRAMMAR).expect("the grammar compiles");
    let shipped = super::compiled().expect("the grammar compiles");
    assert_eq!(fingerprint(&cold), fingerprint(shipped), "the shipped answers make another program");
    assert!(cold.words == shipped.words && cold.program.words == shipped.program.words, "the grammar compiles to two programs");
    let known: crate::Proofs = cold.proofs.iter().cloned().collect();
    let warm = crate::compile_timed(GRAMMAR, known, &mut |_, _| {}).expect("the grammar compiles");
    assert_eq!(warm.proofs.len(), 0, "a question asked again");
    assert_eq!(fingerprint(&warm), fingerprint(&cold));
}

/// Tier 1 alone: `RERAIL_KOTLIN_GRAMMAR`'s, the interpreter's records against the CPU's, over
/// `RERAIL_KOTLIN_SHOW`'s files. A grammar's tokens with a small tree compile in seconds.
#[test]
#[ignore = "a tool: set RERAIL_KOTLIN_GRAMMAR and RERAIL_KOTLIN_SHOW"]
fn tier1_records() {
    let text = std::fs::read_to_string(std::env::var("RERAIL_KOTLIN_GRAMMAR").expect("set RERAIL_KOTLIN_GRAMMAR")).expect("the grammar file");
    let c = crate::compile_text(&text).expect("the grammar compiles");
    for p in std::env::var("RERAIL_KOTLIN_SHOW").expect("set RERAIL_KOTLIN_SHOW").split(':') {
        let src = std::fs::read(p).expect("a file");
        let run = c.program.block_programs[0].run(&src);
        let cpu = c.parser.tier1.extract(&src);
        let same = run.positions == cpu.positions && run.codes == cpu.codes && run.totals == cpu.totals;
        eprintln!("== {p}: {} (totals {:?}: a refusal's counts)", if same { "the same" } else { "DIFFERS" }, cpu.totals);
        // `RERAIL_KOTLIN_RECORDS`: the CPU's records, each start, last, code and text, and the
        // records' marks (`Records::marks`) and blank marks (`Records::blanks`).
        if std::env::var_os("RERAIL_KOTLIN_RECORDS").is_some() {
            if let Some(r) = &c.program.block_programs[0].records {
                eprintln!("marks {:?} blanks {:?} names {:?}", r.marks, r.blanks, c.program.names.marks);
            }
            for (i, &code) in cpu.codes.iter().enumerate() {
                let (s, l) = (cpu.positions[0][i] as usize, cpu.positions[1][i] as usize);
                eprintln!("record {i}: {s}..={l} code {code:#x} marks {:#x} {:?}", code >> rerail_compiler::records::MARK_SHIFT, String::from_utf8_lossy(&src[s..=l.min(src.len() - 1)]));
            }
        }
        if !same {
            // The first record where they part, with the ones around it: start, last, code.
            let rec = |r: &rerail_compiler::block::BlockRun, i: usize| (r.positions[0].get(i).copied(), r.positions.get(1).and_then(|l| l.get(i)).copied(), r.codes.get(i).map(|c| format!("{c:#x}")));
            let n = run.positions[0].len().max(cpu.positions[0].len());
            if let Some(i) = (0..n).find(|&i| rec(&run, i) != rec(&cpu, i)) {
                for j in i.saturating_sub(3)..(i + 4).min(n) {
                    eprintln!("record {j}: interp {:?}\n          cpu {:?}", rec(&run, j), rec(&cpu, j));
                }
                if let Some(Some(at)) = rec(&run, i).0.or(rec(&cpu, i).0).map(|a| Some(a as usize)) {
                    let from = at.saturating_sub(60);
                    eprintln!("bytes {from}..: {:?}", String::from_utf8_lossy(&src[from..(at + 40).min(src.len())]));
                }
            }
            eprintln!("records: interp {} cpu {}; totals: interp {:?} cpu {:?}", run.positions[0].len(), cpu.positions[0].len(), run.totals, cpu.totals);
        }
    }
}


/// Tier 1's operations by output: each output's cone (the operations its value needs) and the
/// operations only it needs, most first. A tool for 0415's stages 3 and 4.
#[test]
#[ignore = "a tool: run by name"]
fn tier1_cones() {
    use rerail_compiler::bitlex_masks as bitlex_compile;
    let stmts = ::bitlex::syntax::parse(crate::GRAMMAR).expect("parses");
    let own = std::cell::RefCell::new(std::collections::HashMap::<String, u16>::new());
    let kind = |name: &str| {
        crate::lean_kind(name).or_else(|| {
            let mut own = own.borrow_mut();
            let next = crate::OWN_IDS + own.len() as u16;
            Some(*own.entry(name.to_owned()).or_insert(next))
        })
    };
    let symbols = bitlex_compile::Symbols { kind: &kind, field: &crate::field };
    let c = bitlex_compile::compile_proved(&stmts, &[bitlex_compile::FALLBACK], Some(&symbols), bitlex_compile::Shape::Rules, crate::shipped_answers()).expect("compiles");
    let b = &c.program;
    let n = b.ops.len();
    let cone = |v: u16| {
        let mut seen = vec![false; n];
        let mut stack = vec![v];
        while let Some(x) = stack.pop() {
            if std::mem::replace(&mut seen[usize::from(x)], true) {
                continue;
            }
            stack.extend(b.ops[usize::from(x)].operands());
        }
        seen
    };
    let cones: Vec<Vec<bool>> = b.outputs.iter().map(|&v| cone(v)).collect();
    let users: Vec<usize> = (0..n).map(|i| cones.iter().filter(|c| c[i]).count()).collect();
    let mut rows: Vec<(usize, usize, String)> = Vec::new();
    for (k, cn) in cones.iter().enumerate() {
        let label = c.labels.iter().filter(|l| usize::from(l.1) == k).map(|l| l.0.as_str()).collect::<Vec<_>>().join(", ");
        let size = cn.iter().filter(|&&x| x).count();
        let only = (0..n).filter(|&i| cn[i] && users[i] == 1).count();
        rows.push((only, size, label));
    }
    // Each operation's statement (`Compiled::origin`): the operations each made, most first.
    let mut by: std::collections::HashMap<&str, usize> = Default::default();
    for o in &c.origin {
        *by.entry(o.as_str()).or_default() += 1;
    }
    let mut by: Vec<_> = by.into_iter().collect();
    by.sort_by_key(|o| std::cmp::Reverse(o.1));
    for (o, k) in &by {
        eprintln!("  {k:5} made by {o}");
    }
    // What the CPU's body keeps (`BlockNative::cpu_form`, roughly): the cones of every output but
    // the record pass's starts and lasts, by statement.
    let kept: Vec<bool> = (0..n).map(|i| cones.iter().enumerate().any(|(k, cn)| cn[i] && !c.labels.iter().any(|l| usize::from(l.1) == k && (l.0 == "stream start" || l.0 == "stream last")))).collect();
    let mut cpu: std::collections::HashMap<&str, usize> = Default::default();
    for (i, o) in c.origin.iter().enumerate().filter(|&(i, _)| kept[i]) {
        let _ = i;
        *cpu.entry(o.as_str()).or_default() += 1;
    }
    let mut cpu: Vec<_> = cpu.into_iter().collect();
    cpu.sort_by_key(|o| std::cmp::Reverse(o.1));
    eprintln!("the CPU body's, roughly: {} operations", kept.iter().filter(|&&k| k).count());
    for (o, k) in cpu.iter().take(40) {
        eprintln!("  {k:5} cpu, made by {o}");
    }
    // The biggest group's operations by kind, and the outputs whose cones they are in; or the
    // group `RERAIL_CONES_OF` names (`mask two_start`).
    let named = std::env::var("RERAIL_CONES_OF").ok();
    let pick = named.as_deref().or(by.first().map(|b| b.0));
    if let Some(big) = pick {
        let mut kinds: std::collections::BTreeMap<String, usize> = Default::default();
        for (i, o) in c.origin.iter().enumerate() {
            if o == big {
                let name = format!("{:?}", b.ops[i]);
                let name = name.split(['(', ' ', '{']).next().unwrap_or("").to_string();
                let name = if name == "Prev" { format!("{:?}", b.ops[i]) .split(',').nth(1).map_or(name.clone(), |k| format!("Prev by{}", k.trim_end_matches(')'))) } else { name };
                *kinds.entry(name).or_default() += 1;
            }
        }
        eprintln!("{big}: {kinds:?}");
        let mut outs = vec![0usize; b.outputs.len()];
        for (i, o) in c.origin.iter().enumerate() {
            if o == big {
                for (k, cn) in cones.iter().enumerate() {
                    // A named group: every output that reads them, shared or not.
                    if cn[i] && (users[i] == 1 || named.is_some()) {
                        outs[k] += 1;
                    }
                }
            }
        }
        for (k, n) in outs.iter().enumerate().filter(|x| *x.1 > 0) {
            let label = c.labels.iter().filter(|l| usize::from(l.1) == k).map(|l| l.0.as_str()).collect::<Vec<_>>().join(", ");
            eprintln!("  output {k} ({label}, delay {}): {n} of them its own", b.output_delays[k]);
        }
    }
    rows.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    let shared = (0..n).filter(|&i| users[i] > 1).count();
    eprintln!("{n} operations, {} outputs; {shared} shared by two or more", b.outputs.len());
    for (only, size, label) in rows {
        eprintln!("  {only:5} own {size:5} cone  {label}");
    }
}

/// 0429's step 1, a probe: the grammar's masks that are token-level (their value at a grammar
/// token's start a function of the tokens: word tests, first-byte classes, their logic, fills
/// between masks that hold at token starts only) and read by token-level readers only, and the
/// operations of tier 1's body they make (`Compiled::origin`). Approximate: it reads the syntax,
/// not the proofs 0429 asks for.
#[test]
#[ignore = "a probe: prints"]
fn token_level_probe() {
    use ::bitlex::syntax::{Arg, Expr, Stmt};
    use rerail_compiler::bitlex_masks as bitlex_compile;
    use std::collections::{HashMap, HashSet};
    // The crate's grammar, or `RERAIL_KOTLIN_GRAMMAR`'s file (a variant measured without a build).
    let text = std::env::var("RERAIL_KOTLIN_GRAMMAR").map_or_else(|_| crate::GRAMMAR.to_owned(), |p| std::fs::read_to_string(p).expect("the grammar file"));
    let stmts = ::bitlex::syntax::parse(&text).expect("parses");
    let mut defs: HashMap<String, Expr> = HashMap::new();
    let mut classes: HashSet<String> = HashSet::new();
    fn walk(s: &[Stmt], defs: &mut HashMap<String, Expr>, classes: &mut HashSet<String>) {
        for s in s {
            match s {
                Stmt::Mask(n, e) => {
                    defs.insert(n.clone(), e.clone());
                }
                Stmt::Class(n, _) => {
                    classes.insert(n.clone());
                }
                Stmt::Block { body, .. } | Stmt::Hyp { body, .. } | Stmt::Rounds { body, .. } => walk(body, defs, classes),
                _ => {}
            }
        }
    }
    walk(&stmts, &mut defs, &mut classes);
    // (token-level, holds at token starts only)
    fn conj<'e>(e: &'e Expr, out: &mut Vec<&'e Expr>) {
        if let Expr::And(a, b) = e {
            conj(a, out);
            conj(b, out);
        } else {
            out.push(e);
        }
    }
    struct Cx<'a> {
        defs: &'a HashMap<String, Expr>,
        classes: &'a HashSet<String>,
        memo: HashMap<String, (bool, bool)>,
    }
    fn word_text(e: &Expr) -> Option<usize> {
        match e {
            Expr::Call(f, a) if f == "text" => match a.as_slice() {
                [Arg::S(s)] if s.iter().all(|b| b.is_ascii_alphanumeric() || *b == b'_') => Some(s.len()),
                _ => None,
            },
            _ => None,
        }
    }
    fn tl(cx: &mut Cx, e: &Expr) -> (bool, bool) {
        match e {
            Expr::Name(n) if n == "kw_start" => (true, true),
            Expr::Name(n) if n == "code" => (true, false),
            Expr::Name(n) if cx.classes.contains(n) => (true, false),
            Expr::Name(n) => {
                if let Some(&v) = cx.memo.get(n) {
                    return v;
                }
                cx.memo.insert(n.clone(), (false, false));
                let v = match cx.defs.get(n).cloned() {
                    Some(d) => tl(cx, &d),
                    None => (false, false),
                };
                cx.memo.insert(n.clone(), v);
                v
            }
            Expr::Empty => (true, false),
            Expr::Not(x) => (tl(cx, x).0, false),
            Expr::Or(a, b) | Expr::Xor(a, b) => {
                let (x, y) = (tl(cx, a), tl(cx, b));
                (x.0 && y.0, x.1 && y.1)
            }
            Expr::And(..) => {
                let mut fs = Vec::new();
                conj(e, &mut fs);
                // A word test: `kw_start & text(W) & ~next(w, |W|)`.
                let starts = fs.iter().any(|f| matches!(f, Expr::Name(n) if n == "kw_start"));
                let mut all = true;
                let mut only = false;
                for f in &fs {
                    let v = match f {
                        _ if starts && word_text(f).is_some() => (true, true),
                        Expr::Not(x) if starts && matches!(&**x, Expr::Call(c, _) if c == "next") => (true, false),
                        Expr::Call(c, a) if c == "text" && matches!(a.as_slice(), [Arg::S(_)]) => (true, true),
                        f => tl(cx, f),
                    };
                    all &= v.0;
                    only |= v.1;
                }
                (all, all && only)
            }
            Expr::Call(c, a) if c == "text" && matches!(a.as_slice(), [Arg::S(_)]) => (true, true),
            Expr::Call(c, a) if c == "fill" => match a.as_slice() {
                [Arg::E(s), Arg::E(t)] => {
                    let (x, y) = (tl(cx, s), tl(cx, t));
                    (x.0 && x.1 && y.0 && y.1, false)
                }
                _ => (false, false),
            },
            _ => (false, false),
        }
    }
    let mut cx = Cx { defs: &defs, classes: &classes, memo: HashMap::new() };
    let names: Vec<String> = defs.keys().cloned().collect();
    let mut level: HashSet<String> = names.iter().filter(|n| tl(&mut cx, &Expr::Name((*n).clone())).0).cloned().collect();
    // The readers: masks by their definitions; marks read at token starts; every other statement
    // bytewise.
    fn uses(e: &Expr, out: &mut HashSet<String>) {
        let s = format!("{e:?}");
        for part in s.split("Name(\"").skip(1) {
            if let Some(n) = part.split('"').next() {
                out.insert(n.to_string());
            }
        }
    }
    let mut bytewise: HashSet<String> = HashSet::new();
    let mut readers: HashMap<String, Vec<String>> = HashMap::new();
    for (n, e) in &defs {
        let mut u = HashSet::new();
        uses(e, &mut u);
        for x in u {
            readers.entry(x).or_default().push(n.clone());
        }
    }
    fn others(s: &[Stmt], bytewise: &mut HashSet<String>) {
        for s in s {
            match s {
                Stmt::Mask(..) | Stmt::Class(..) => {}
                Stmt::Mark { .. } => {}
                Stmt::Block { body, .. } | Stmt::Hyp { body, .. } => others(body, bytewise),
                s => {
                    let d = format!("{s:?}");
                    for part in d.split("Name(\"").skip(1) {
                        if let Some(n) = part.split('"').next() {
                            bytewise.insert(n.to_string());
                        }
                    }
                }
            }
        }
    }
    others(&stmts, &mut bytewise);
    loop {
        let before = level.len();
        let keep: HashSet<String> = level.iter().filter(|n| !bytewise.contains(*n) && readers.get(*n).is_none_or(|rs| rs.iter().all(|r| level.contains(r)))).cloned().collect();
        level = keep;
        if level.len() == before {
            break;
        }
    }
    let own = std::cell::RefCell::new(HashMap::<String, u16>::new());
    let kind = |name: &str| {
        crate::lean_kind(name).or_else(|| {
            let mut own = own.borrow_mut();
            let next = crate::OWN_IDS + own.len() as u16;
            Some(*own.entry(name.to_owned()).or_insert(next))
        })
    };
    let symbols = bitlex_compile::Symbols { kind: &kind, field: &crate::field };
    let c = bitlex_compile::compile_proved(&stmts, &[bitlex_compile::FALLBACK], Some(&symbols), bitlex_compile::Shape::Rules, crate::shipped_answers()).expect("compiles");
    let mut by: HashMap<String, usize> = HashMap::new();
    for o in &c.origin {
        if let Some(m) = o.strip_prefix("mask ") {
            *by.entry(m.to_string()).or_default() += 1;
        }
    }
    let mut rows: Vec<(usize, String)> = level.iter().map(|n| (by.get(n).copied().unwrap_or(0), n.clone())).collect();
    rows.sort_by(|a, b| b.cmp(a));
    let total: usize = rows.iter().map(|r| r.0).sum();
    eprintln!("token-level masks read by token-level readers only: {}, {total} of {} operations", rows.len(), c.program.ops.len());
    for (k, n) in rows.iter().take(40) {
        eprintln!("  {k:4} {n}");
    }
    // The biggest token-level masks some bytewise reader keeps, and the readers.
    let tl_all: Vec<String> = names.iter().filter(|n| !level.contains(*n) && tl(&mut cx, &Expr::Name((*n).clone())).0).cloned().collect();
    let mut kept: Vec<(usize, String)> = tl_all.iter().map(|n| (by.get(n).copied().unwrap_or(0), n.clone())).collect();
    kept.sort_by(|a, b| b.cmp(a));
    eprintln!("token-level but read bytewise:");
    for (k, n) in kept.iter().take(25) {
        let rs: Vec<&String> = readers.get(n).map_or(vec![], |r| r.iter().filter(|r| !level.contains(*r)).collect());
        eprintln!("  {k:4} {n}: {}{rs:?}", if bytewise.contains(n) { "a statement, " } else { "" });
    }
}

/// Block comments the scanner reads after a line break (the lean lexer's `asi`, 0429): swallowed
/// before a word with `a`, `e` or `w` where the state takes a semicolon (after a header's line
/// too, whose node then ends at the word), and any other comment where it takes none (a call's
/// arguments, after an operator). From the corpus's files the escape refused.
#[test]
fn asked_comments_are_the_lean_parsers() {
    same(&[
        "package a.b\n\n/**\n * Doc\n */\nabstract class X\n",
        "import a.b\n/** d */\nenum class E { A }\n",
        "import a.b\n\n/**\n * d\n */\nabstract class X\n",
        "package a\n\n/* c */\nabstract class A\n",
        "package a\n/* c */\nabstract class A\n",
        "class C {\n    fun f() {\n    }\n\n    /*\n     * c\n     */\n    external fun g(): String?\n}\n",
        "fun f() {\n    g(\n        /* a = */ x,\n        /* b = */ eq(false),\n    )\n}\n",
        "fun f() {\n    g(\n        a(),\n        /* c */ eq(false),\n    )\n}\n",
        "abstract class A {\n    abstract val x: Int\n\n    /**\n     * c\n     */\n    abstract fun g()\n}\n",
        "val x = 1\n/* c */\nabstract class X\n",
        "fun f() = g()\n\n/* c */\nwhen (x) {}\n",
        "fun f() {\n    a +\n    /* c */\n    abstract()\n}\n",
        // No comment the scanner reads: a `*/` in a string, a comment inside a line.
        "fun f() {\n    val x = g(\"*/*\", y)\n        ?: h()\n}\n",
        "val x = \"\"\"\n  a/*/\n\"\"\"\n    .trimIndent()\n",
        "fun f() = g(a, 0 /*r*/, b)\n",
        "fun f() {\n    g(\n        true /* s */,\n        x,\n    )\n}\n",
        // Before `catch` or `finally`: the `try`'s comment. Before a `when` entry's `else ->`:
        // swallowed, the entry to the `else`. Before an `if`'s `else`: lost, of no width there.
        "fun f() {\n    try { a() }\n    /* c */ catch (e: E) {}\n}\n",
        "fun f() {\n    try { a() }\n    /* c */\n    catch (e: E) {}\n}\n",
        "fun f() {\n    try { a() } catch (e: E) {}\n    /* c */ finally {}\n}\n",
        "val x = try { a() }\n/* c */ catch (e: E) { b }\n",
        "fun f() {\n    try {\n        a()\n    }\n    /* c */\n    finally {\n        b()\n    }\n}\n",
        "val x = when (a) {\n    1 -> b\n    /* c */\n    else -> d\n}\n",
        "val x = when (a) {\n    1 -> b\n    /* c */ else -> d\n}\n",
        "val x = when (a) {\n    1 -> b\n    /* c */\n    else\n    -> d\n}\n",
        "val x = when (a) {\n    1 -> b\n    /* c */\n    2 -> d\n}\n",
        "fun f() {\n    if (a) b()\n    /* c */\n    else d()\n}\n",
        "fun f() {\n    if (a) { b() }\n    /* c */ else { d() }\n}\n",
        "val x = if (a) b\n/* c */ else d\n",
        "fun f() {\n    if (a) b()\n    /* c */ elsewhere()\n}\n",
    ]);
}

/// A probe: Bitlex's reference (`eval`, `tree`) on `RERAIL_KOTLIN_TEXT`, its tree or its refusal.
#[test]
#[ignore = "a probe: prints"]
fn reference_probe() {
    let text = std::env::var("RERAIL_KOTLIN_TEXT").unwrap_or_default().replace("\\n", "\n");
    let stmts = ::bitlex::syntax::parse(crate::GRAMMAR).expect("parses");
    let tables = Default::default();
    match ::bitlex::eval::Ev::run(&stmts, text.as_bytes(), &tables) {
        Ok(out) => {
            eprintln!("errors {:?}", out.errors);
            if let Some(tree) = &out.tree {
                eprintln!("reference tree: {:?}", ::bitlex::tree::nodes(tree, &out.tokens, text.len()).map(|n| n.len()));
            }
        }
        Err(e) => eprintln!("eval: {e:?}"),
    }
    let c = compiled().expect("compiles");
    match c.parser.image(text.as_bytes()) {
        Ok(i) => eprintln!("compiled: {}", image_text(&i).expect("an image")),
        Err(e) => eprintln!("compiled refuses: {e:?}\n{}", c.explain(&text)),
    }
}

/// Each record mark's lowering on the CPU: its output, word, and the flags that decide whether
/// tier 2 reads it from the bytes itself (`native_cpu_block::lazy_marks`).
#[test]
#[ignore = "a probe: prints each mark's flags"]
fn lazy_probe() {
    let c = compiled().expect("compiles");
    let b = &c.program.block_programs[0];
    let r = b.records.as_ref().expect("records");
    let native = rerail_compiler::block_native::BlockNative::lower(b);
    let bw = rerail_compiler::native_cpu_bytes::Bytewise::of(&native);
    let lazy = rerail_compiler::native_cpu_block::lazy_marks(r, &|o| bw.can(o));
    let coded = rerail_compiler::native_cpu_block::lazy_coded(r);
    for (k, &(o, w)) in r.marks.iter().enumerate() {
        let bit = |x: u32| x >> k & 1;
        eprintln!(
            "mark {k}: output {o} word {w} gap {} tested_only {} alone {} unplaced {} extra {} coded {} unless {} bytewise {} lazy {}",
            bit(r.gaps), bit(r.tested_only), bit(r.alone), bit(r.unplaced), r.extra == Some(k as u8), bit(coded),
            r.unless.iter().any(|&(m, _)| m as usize == k), o != rerail_compiler::records::WORD_ALONE && bw.can(o), lazy.iter().any(|&(m, _)| m as usize == k)
        );
    }
    for ch in &r.checks {
        eprintln!("check: {ch:?}");
    }
    eprintln!("blanks: {:?} gap {:?} skip {:?} gaps {:b}", r.blanks, r.gap, r.skip, r.gaps);
    let blank_bits = rerail_compiler::native_cpu_block::blank_bits(r, &|o| bw.can(o));
    eprintln!("blank marks read by the CPU loop: {blank_bits:b}; records_made {:?}", native.records_made());
}

/// The walk lemmas (`bitlex_masks::NO_WALK_LEMMAS`: a walk's regions never overlap) change at most what tier 2 is
/// proved to need: the grammar reads each text as without them, the same tape or the same refusal.
#[test]
fn walk_lemmas_keep_the_tapes() {
    let with = super::compiled().expect("the grammar compiles");
    assert!(!with.program.block_programs[0].walks.is_empty(), "the grammar has no walk for the lemmas");
    bitlex_compile::NO_WALK_LEMMAS.set(true);
    let without = compile_text(GRAMMAR);
    bitlex_compile::NO_WALK_LEMMAS.set(false);
    let without = without.expect("the grammar compiles without the walk lemmas");
    // Since tier 1 reads a string as one token (3a5cf85230) and the comment escapes became questions (86d31b6fae),
    // Kotlin's tier 2 is proved to need the same without them (5,892 words either way, as `.rerail`'s 2,745; JSON's
    // grammars have no walk): what is checked is that they never make tier 2 need more.
    assert!(with.program.words.len() <= without.program.words.len(), "the walk lemmas make tier 2 larger: {} words, {} without", with.program.words.len(), without.program.words.len());
    let texts = [
        "package a\n\nimport b.c\n",
        "val s = \"a // b\" // c \"d\"\n",
        "/* \"x\" */ val c = '\"'\n",
        "val a = 0x1F + 1_000L - 1.5e-3f // \"\n",
        "val s = \"\"\"raw /* x */ \"\"\"\n",
        "val s = \"${a + \"b\"}\"\n",
        "// only\n",
        "/* open\n",
        "val s = \"open\n",
        "val c = '\\n' + '\\'' /* '\" */\n",
        "fun f() {\n    a(\"x\") /* y */ // z\n}\n",
    ];
    for t in texts {
        assert_eq!(with.parser.image(t.as_bytes()).ok(), without.parser.image(t.as_bytes()).ok(), "{t:?}");
    }
}


/// Each text of the file `RERAIL_KOTLIN_PROBE` (texts separated by a line `---`): whether the lean
/// parser and the compiled grammar read it, and whether their tapes agree. A tool, run by hand.
#[test]
#[ignore]
fn probe() {
    let Ok(path) = std::env::var("RERAIL_KOTLIN_PROBE") else { return };
    let c = compiled().expect("compiles");
    for text in std::fs::read_to_string(path).expect("the probe file").split("\n---\n") {
        let text = format!("{}\n", text.trim_end_matches('\n'));
        let lean = lean_text(&text);
        let ours = c.parser.image(text.as_bytes()).ok().and_then(|i| image_text(&i).ok());
        let verdict = match (&lean, &ours) {
            (Some(a), Some(b)) if a == b => "same".to_string(),
            (Some(a), Some(b)) => {
                let (x, y): (Vec<&str>, Vec<&str>) = (a.lines().collect(), b.lines().collect());
                let i = x.iter().zip(&y).position(|(p, q)| p != q).unwrap_or(x.len().min(y.len()));
                format!("differ: lean {:?} ours {:?}", x.get(i), y.get(i))
            }
            (Some(_), None) => {
                let run = c.program.block_programs[0].run(text.as_bytes());
                let why = rerail_compiler::stream::run(&c.program, text.as_bytes(), &run).err();
                let near = why.as_ref().and_then(|w| w.at).map(|a| text[a as usize..].chars().take(30).collect::<String>());
                let jit2 = c.parser.tier2.run(text.as_bytes(), &run).map(|n| n.len());
                let cpu = c.parser.tier1.extract(text.as_bytes());
                let same1 = cpu.codes == run.codes && cpu.positions == run.positions;
                let first = (0..cpu.codes.len().min(run.codes.len())).find(|&i| cpu.codes[i] != run.codes[i] || cpu.positions.iter().zip(&run.positions).any(|(a, b)| a.get(i) != b.get(i)));
                let first = first.or((cpu.codes.len() < run.codes.len()).then_some(cpu.codes.len()));
                let at = first.and_then(|i| run.positions.first().and_then(|p| p.get(i))).map(|&a| (a, text[a as usize..].chars().take(30).collect::<String>()));
                format!("ours refuses: interpreter {why:?} {near:?}; JIT tier 2 over its tier 1: {jit2:?}; JIT tier 1 the same: {same1} (codes {} vs {}), first differing token at {at:?}", cpu.codes.len(), run.codes.len())
            }
            (None, Some(_)) => "lean refuses, ours reads".into(),
            (None, None) => "both refuse".into(),
        };
        let shown: String = text.chars().take(120).collect();
        eprintln!("PROBE {shown:?} ({} bytes)\n  => {verdict}", text.len());
        {
            // Tier 1: the JIT's records against the interpreter's.
            let run = c.program.block_programs[0].run(text.as_bytes());
            let cpu = c.parser.tier1.extract(text.as_bytes());
            let n = cpu.codes.len().min(run.codes.len());
            let first = (0..n).find(|&i| cpu.codes[i] != run.codes[i] || cpu.positions.iter().zip(&run.positions).any(|(a, b)| a.get(i) != b.get(i)));
            let first = first.or((cpu.codes.len() != run.codes.len()).then_some(n));
            if let Some(i) = first {
                let at = run.positions.first().and_then(|p| p.get(i)).copied().unwrap_or(0) as usize;
                eprintln!("  tier 1: the JIT's {} records, the interpreter's {}; first apart: record {i} at {at} {:?}", cpu.codes.len(), run.codes.len(), text.get(at..).map(|t| t.chars().take(30).collect::<String>()));
            }
        }
        if std::env::var_os("RERAIL_KOTLIN_PROBE_LEAN").is_some() {
            eprintln!("  lean:\n{}", lean.as_deref().unwrap_or("(refused)"));
        }
        if std::env::var_os("RERAIL_KOTLIN_PROBE_INTERP").is_some() {
            // The interpreter's nodes, in its order: kind and span.
            let run = c.program.block_programs[0].run(text.as_bytes());
            match rerail_compiler::stream::run(&c.program, text.as_bytes(), &run) {
                Ok(nodes) => {
                    let shown: Vec<String> = nodes.iter().map(|n| format!("{}[{}..{}]", n.kind, n.start, n.end)).collect();
                    eprintln!("  interpreter: {}", shown.join(" "));
                }
                Err(why) => eprintln!("  interpreter refuses: {why:?}"),
            }
            if let Ok(nodes) = c.parser.tier2.run(text.as_bytes(), &run) {
                let shown: Vec<String> = nodes.iter().map(|n| format!("{}[{}..{}]", n.kind, n.start, n.end)).collect();
                eprintln!("  JIT tier 2:  {}", shown.join(" "));
            }
        }
        if std::env::var_os("RERAIL_KOTLIN_PROBE_TOKENS").is_some() {
            // The reference evaluator's errors and tokens, marks by name.
            let stmts = ::bitlex::syntax::parse(crate::GRAMMAR).expect("parses");
            let out = ::bitlex::eval::Ev::run(&stmts, text.as_bytes(), &Default::default()).expect("runs");
            eprintln!("  errors: {:?}", out.errors);
            let toks: Vec<String> = out.tokens.iter().map(|t| if t.mark { format!("<{}>", t.kind) } else { format!("{:?}", &text[t.start..t.end]) }).collect();
            eprintln!("  tokens: {}", toks.join(" "));
            let tree = out.tree.as_ref().map(|d| ::bitlex::tree::nodes(d, &out.tokens, text.len()).map(|n| n.len()));
            eprintln!("  the reference tree: {tree:?}");
        }
    }
}
