//! The compiled grammar against the lean parser: the same tape, node for node, on every text it
//! reads; a refusal (to the lean parser) where it does not.

use super::*;

/// The tests' grammar: the crate's, or `RERAIL_KOTLIN_GRAMMAR`'s file (a variant tried without a
/// rebuild). It shadows the crate's `compiled`.
fn compiled() -> Result<&'static Compiled, &'static str> {
    static OTHER: std::sync::OnceLock<Result<Compiled, String>> = std::sync::OnceLock::new();
    match std::env::var("RERAIL_KOTLIN_GRAMMAR") {
        Ok(p) => OTHER.get_or_init(|| compile_text(&std::fs::read_to_string(p).expect("the grammar file"))).as_ref().map_err(String::as_str),
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

/// Runs that count their depths (Q326), as `type_args_end` and `lambda_params_end` do: type
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
    ]);
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

#[test]
fn what_step_35_does_not_read_is_refused() {
    refused(&[
        // A string, a character or a backtick name in a raw string's template: its `}` would
        // open a tail; a raw string in a template, a raw tail.
        "val h = \"\"\"${f(\"a\")}\"\"\"\n",
        "val h = \"\"\"${f('a')}\"\"\"\n",
        "val h = \"${f(\"\"\"a\"\"\")}\"\n",
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
        // The run to the `)` passes line breaks (Q272): a declaration after is still one.
        "fun f() {\n    @A(x) if (y) z\n    val q = 1\n}\n",
        // An annotation before any expression, as the lean parser's `declaration_ahead` finds no
        // declaration (Q274); and before a declaration's modifiers and keyword.
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
fn what_step_34_does_not_read_is_refused() {
    refused(&[
        // The lean parser's `fun` leaf: a function's `fun` is implied, and the `interface`
        // after it comes too late to keep it.
        "fun interface A {\n    fun f()\n}\n",
    ]);
}

#[test]
fn what_step_30_does_not_read_is_refused() {
    refused(&[
        "val a = 1\n/* c */ /* d */\nexternal fun f()\n",
        "val x = when (a) {\n    1 -> b\n    /* c */\n    else -> d\n}\n",
    ]);
}

/// A visibility modifier and `constructor` on two lines after a class's name: the lean lexer
/// reads them on one line only (`modifier_then_constructor`), and refuses (a token's `without`).
#[test]
fn a_constructor_off_its_modifiers_line_is_refused() {
    refused(&["class A\nprivate\nconstructor(x: Int)\n", "class A\n@Inject private\nconstructor(x: Int)\n"]);
}

#[test]
fn what_step_27_does_not_read_is_refused() {
    refused(&[
        "class A {\n    private val m by lazy { m ?: getSystemService<M>()!! }\n}\n",
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

#[test]
fn what_step_24_does_not_read_is_refused() {
    refused(&[
        "val s = \"${f(\"}\")}\"\n",
        "val s = \"${f(\"${x}\")}\"\n",
        "val s = \"${if (a) { 1 } else 2}\"\n",
    ]);
}

#[test]
fn what_step_23_does_not_read_is_refused() {
    refused(&[
        "fun interface P {\n    fun f()\n}\n",
        "fun f(g: ((Int) -> Int) -> Int) {}\n",
    ]);
}

#[test]
fn what_step_22_does_not_read_is_refused() {
    refused(&[
        "fun f() {\n    val d = x as java.util.List<Int>\n    g(d)\n}\n",
        "fun <T : Comparable<T>> B.f() {}\n",
        "val <T : Any> List<T>.second: T get() = this[1]\n",
    ]);
}

#[test]
fn what_step_21_does_not_read_is_refused() {
    refused(&[
        "val h = \"\"\"\"a\"\"\"\n",
        "val h = \"\"\"a\"\"\"\"\n",
        "val h = \"\"\"\\$a\"\"\"\n",
    ]);
}

#[test]
fn what_step_20_does_not_read_is_refused() {
    refused(&[
        "val a = b to f<T>()\n",
        "val a = x + f<T>(1)\n",
        "val a = x < f<T>(1)\n",
        "import a.b import c\n",
        "val s = 'x\n",
        "val s = \"a\nb\"\n",
        "val s = \"${ {x} }\"\n",
        "val s = \"${}\"\n",
        "val s = \"open\n",
        "/* /* nested */ */\npackage a\n",
        "/* open\n",
        "val f = { a, -> a }\n",
        "val x = when (y) { else -> 1 }\n",
        "val x = 1d\n",
        "val x = 1.5L\n",
        "val x = 1_\n",
        "val x = 0x\n",
        "class A { val x = 1 }\n",
        "val x: Int\n    get() = 1\n",
        "fun f(noinline: Int) {}\n",
        "fun String?.h() = 1\n",
        "fun f() {\n    loop@ for (i in a) break@loop\n}\n",
        "val a = b as java.util.List<Int>\n",
        "fun f() {\n    a()\n    /* c */\n    // d\n    b()\n}\n",
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
    assert_eq!(differ, 0);
}

/// The lean parser's tape for each file in `RERAIL_KOTLIN_SHOW` (paths, `:`-separated), and the
/// grammar's or why it refuses: the reference a ladder step is written against.
#[test]
#[ignore = "a tool: set RERAIL_KOTLIN_SHOW"]
fn show() {
    let c = compiled().expect("compiles");
    let r = c.program.block_programs[0].records.as_ref();
    eprintln!("tier 2: {} words; the code holds {:?} marks and {:?} bits in all", c.program.words.len(), r.map(|r| r.mark_bits()), r.map(|r| r.marks.len()));
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

/// The answers a compile made, given to another compile of the grammar (decision 0414): it asks
/// none of its questions again, and makes the same program.
#[test]
fn known_answers_make_the_same_program() {
    let cold = compiled().expect("compiles");
    let known: crate::Proofs = cold.proofs.iter().cloned().collect();
    let warm = crate::compile_timed(crate::GRAMMAR, known, &mut |_, _| {}).expect("compiles");
    assert_eq!(warm.proofs.len(), 0, "a question asked again");
    assert_eq!(fingerprint(&warm), fingerprint(cold));
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
        eprintln!("== {p}: {}", if same { "the same" } else { "DIFFERS" });
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
    let c = bitlex_compile::compile_proved(&stmts, &[bitlex_compile::FALLBACK], Some(&symbols), bitlex_compile::Shape::Rules, Default::default()).expect("compiles");
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
    by.sort_by(|a, b| b.1.cmp(&a.1));
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
    cpu.sort_by(|a, b| b.1.cmp(&a.1));
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
    let stmts = ::bitlex::syntax::parse(crate::GRAMMAR).expect("parses");
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
    let c = bitlex_compile::compile_proved(&stmts, &[bitlex_compile::FALLBACK], Some(&symbols), bitlex_compile::Shape::Rules, Default::default()).expect("compiles");
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

/// One grammar compiles to one program, each time (the compiler's maps hash with a fixed seed), and the walk lemmas
/// (`bitlex_masks::NO_WALK_LEMMAS`: a walk's regions never overlap) change only what tier 2 is proved to need: a
/// step whose walk they speak of reads each text as without them, the same tape or the same refusal. The step is
/// the ladder's last, the one it keeps (0422).
#[test]
#[ignore = "slow: compiles step 36 three times, ~85 s each on an M1 Max (0422 kept no smaller step); Q331"]
fn a_step_compiles_to_one_program_and_walk_lemmas_keep_its_tapes() {
    let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/bitlex_ladder/36-no-back-fills.bitlex")).expect("step 36");
    let with = compile_text(&source).expect("step 36 compiles");
    assert!(!with.program.block_programs[0].walks.is_empty(), "step 36 has no walk for the lemmas");
    let again = compile_text(&source).expect("step 36 compiles");
    assert!(with.words == again.words && with.program.words == again.program.words, "step 36 compiles to two programs");
    bitlex_compile::NO_WALK_LEMMAS.set(true);
    let without = compile_text(&source);
    bitlex_compile::NO_WALK_LEMMAS.set(false);
    let without = without.expect("step 36 compiles without the walk lemmas");
    assert!(with.program.words.len() < without.program.words.len(), "the walk lemmas prove nothing in step 36: {} words either way", with.program.words.len());
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
