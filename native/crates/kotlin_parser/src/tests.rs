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
        "package a\n\n/* c */\nabstract class A\n",
        "package a\n/* c */\nabstract class A\n",
        "val a = 1\n/* c */ /* d */\nexternal fun f()\n",
        "val x = when (a) {\n    1 -> b\n    /* c */\n    else -> d\n}\n",
        "fun f() {\n    a +\n    /* c */\n    abstract()\n}\n",
        "fun f() {\n    g(\n        a(),\n        /* c */ eq(false),\n    )\n}\n",
    ]);
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
    eprintln!("tier 2: {} words", c.program.words.len());
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
    // Cold, then with the answers the cold compile made (decision 0404). With
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

/// The answers a compile made, given to another compile of the grammar (decision 0404): it asks
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
