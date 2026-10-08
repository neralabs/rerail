<?php
// Writes phpdoc.expected.json: for doc comments written for Rerail, the tree phpstan/phpdoc-parser
// 2.3.5 (as PHPStan 2.2.16 bundles it) parses from each, recorded by `phpdoc_tree.php` (which
// documents the record). The cases are hand-written comments covering every tag the parser reads
// and every type and constant form, then seeded random comments assembled from tag, type and text
// fragments (some cut or spliced so the parser fails), then seeded random strings over the bytes
// the lexer gives meaning; each is a comment PHP lexes as one `T_DOC_COMMENT`.
//
// With `--corpus OUT`, it also writes OUT: every distinct doc comment of the corpora
// /tmp/rerail-php-corpus/laravel (e5f3d6265f) and /tmp/rerail-php-corpus/symfony (2c4011aa7e), the
// input of the ignored benchmark `benchmark_phpdoc_corpus` (not checked in; the checked-in
// samples are under `literal/{laravel,symfony}/`).
//
// Run offline, not in CI:
//   /opt/homebrew/Cellar/php/8.5.11/bin/php -n generate_phpdoc.php \
//     /tmp/rerail-php-tools/vendor/phpstan/phpstan/phpstan.phar [--corpus /tmp/rerail-phpdoc-corpus.json]
// Requires PHP 8.5.11 and PHPStan 2.2.16 (bundling phpstan/phpdoc-parser 2.3.5).

require __DIR__ . '/phpdoc_tree.php';

$phar = $argv[1] ?? '';
$version = phpdoc_require($phar);

$lines = [
    // Text and generic tags.
    'Summary.',
    'Summary with `code` and a {@link Foo} inline tag.',
    '@internal',
    '@api',
    '@since 1.2.3',
    '@see Foo::bar() for more',
    '@inheritDoc',
    '@todo fix this',
    '@author Jane <jane@example.com>',
    '@foo:bar baz',
    '@phpstan-ignore-next-line',
    '@phpstan-ignore argument.type (because)',
    '@psalm-suppress MixedArgument',
    '@_underscore',
    '@1numeric',
    // A tag's description parses the next tag on its line to see whether it ends there.
    '@a @b @c',
    rtrim(str_repeat('@a ', 40)),
    rtrim(str_repeat('@see x ', 12)),
    '@a @param @b @c',
    '@a @param(1) @b',
    '@Foo(1) @a @Bar(2) @b',
    '@\\Doctrine\\ORM\\Column(type="string", length=255)',
    '@ORM\\Entity(repositoryClass=FooRepository::class)',
    '@ORM\\Table(name="foo", indexes={@ORM\\Index(name="a", columns={"a", "b"})})',
    '@Annotation',
    '@Assert\\Choice({"a", "b"})',
    '@Route("/path/{id}", name="route", requirements={"id"="\\d+"})',
    '@Foo(1, 2.5, true, false, null, FOO, Bar::BAZ, {1: "a", "k": 2})',
    '@Foo("unterminated)',
    '@Foo(',
    '@Foo(a=)',
    '@Foo(@Bar, @Baz(1))',
    '@Foo({key: @Bar})',
    '@Foo(\'single\')',
    '@Foo("a""b")',
    '@Foo(Bar::*)',
    '@Foo(Bar::BAZ_*)',
    '@Foo(array(1, 2))',
    '@Foo([1, 2])',
    // Params.
    '@param int $a',
    '@param int $a The first.',
    '@param int|string $a',
    '@param ?int $a',
    '@param int ...$rest',
    '@param int &$ref',
    '@param int &...$refs',
    '@param $untyped',
    '@param ...$untypedRest',
    '@param &$untypedRef',
    '@param int',
    '@param int $a|string',
    '@param',
    '@phpstan-param list<int> $a',
    '@psalm-param array<string, mixed> $a',
    '@phan-param int $a',
    '@param mixed $value',
    '@param array{a: int, b?: string, 0: float, \'q\': bool, "d": null} $shape',
    '@param array{int, string} $tuple',
    '@param list{int, string} $list',
    '@param non-empty-array{a: int} $a',
    '@param non-empty-list{int} $a',
    '@param array{a: int, ...} $open',
    '@param array{a: int, ...<string>} $open',
    '@param array{a: int, ...<string, int>} $open',
    '@param list{int, ...<string>} $open',
    '@param array{...} $open',
    '@param array {a: int} $spaced',
    '@param array{a: int $broken',
    '@param array{Foo::BAR: int} $const',
    '@param object{a: int, b?: string} $object',
    '@param object{\'a\': int} $object',
    '@param object{} $empty',
    '@param callable(int, string): bool $callback',
    '@param callable(int $a, string ...$rest): void $callback',
    '@param callable(int &$a, int=): void $callback',
    '@param \\Closure(int): (int|string) $callback',
    '@param Closure<T of object>(T): T $callback',
    '@param callable(): ?int $callback',
    '@param callable(): $this $callback',
    '@param callable(): static[] $callback',
    '@param callable(): array{a: int} $callback',
    '@param callable(): Foo::BAR $callback',
    '@param callable(): int<0, max> $callback',
    '@param callable(): $broken',
    '@param callable(int $callback',
    '@param pure-callable(): void $callback',
    '@param int<0, max> $range',
    '@param int<min, -1> $negative',
    '@param key-of<Foo::BAR> $key',
    '@param value-of<Suit> $value',
    '@param int-mask-of<Foo::*> $mask',
    '@param int-mask<1, 2, 4> $mask',
    '@param class-string<Foo> $class',
    '@param Foo&Bar $both',
    '@param (Foo&Bar)|null $maybe',
    '@param Foo[] $list',
    '@param Foo[][] $nested',
    '@param array<int, Foo>[] $arrays',
    '@param Foo [] $spaced',
    '@param T[K] $offset',
    '@var int' . str_repeat('[]', 12),
    '@var T' . str_repeat("['a']", 10) . '[] $deep',
    '@param Foo::BAR $const',
    '@param Foo::BAR_* $consts',
    '@param Foo::* $all',
    '@param \'literal\' $literal',
    '@param "double" $literal',
    '@param 42 $int',
    '@param -1.5 $float',
    '@param 0x1F $hex',
    '@param 0b101 $binary',
    '@param 0o17 $octal',
    '@param 1_000_000 $underscored',
    '@param 1e10 $exponent',
    '@param .5 $leadingDot',
    '@param true|false|null $constants',
    '@param array<covariant Foo, contravariant Bar> $variance',
    '@param Foo<*> $wildcard',
    '@param Foo<int,> $trailing',
    '@param Foo<int $broken',
    '@param Foo< $broken',
    '@param ($a is int ? string : bool) $conditional',
    '@param \\Foo\\Bar $qualified',
    '@param Foo<div>bar</div> $html',
    '@param $this $self',
    '@param static $static',
    '@param ?Foo[] $nullableArray',
    '@param int|string|Foo<Bar|Baz> $many',
    '@param array<' . "\n" . ' *   int,' . "\n" . ' *   string' . "\n" . ' * > $multiline',
    '@param array{' . "\n" . ' *   a: int, // the a' . "\n" . ' *   b: string,' . "\n" . ' * } $commented',
    '@param (int|' . "\n" . ' *   string) $multilineUnion',
    '@param int|' . "\n" . ' * string $split',
    '@param-immediately-invoked-callable $cb',
    '@param-later-invoked-callable $cb desc',
    '@phpstan-param-closure-this Foo $cb',
    '@param-closure-this Foo $cb',
    '@pure-unless-callable-is-impure $cb',
    '@phpstan-pure-unless-parameter-passed $cb',
    '@param-out int $out',
    '@phpstan-param-out list<int> $out',
    '@param-out int',
    // Var, return, throws.
    '@var int',
    '@var int $a',
    '@var int $a desc',
    '@var int desc',
    '@var int|desc',
    '@var $this',
    '@var Foo $this',
    '@var',
    '@psalm-var array<int> $a',
    '@return int',
    '@return int The answer.',
    '@return static',
    '@return $this',
    '@return void',
    '@return never',
    '@return ($a is null ? int : string)',
    '@return ($a is not null ? int : string)',
    '@return (T is int ? int : (T is string ? string : never))',
    '@return ($a is int' . "\n" . ' *   ? int' . "\n" . ' *   : string)',
    '@return int|',
    '@return int&',
    '@return int)',
    '@return int<',
    '@return',
    '@return array<string, mixed>|null Description that' . "\n" . ' *   continues here.',
    '@phpstan-return list<T>',
    '@psalm-return T',
    '@phan-real-return int',
    '@throws \\RuntimeException',
    '@throws \\RuntimeException|\\LogicException When it fails.',
    '@phpstan-throws void',
    '@mixin Foo',
    '@phan-mixin Foo<Bar>',
    '@phpstan-require-extends Foo',
    '@psalm-require-implements Foo',
    '@phpstan-sealed Foo|Bar',
    '@psalm-inheritors Foo|Bar',
    '@deprecated',
    '@deprecated since 2.0, use bar()',
    // Properties and methods.
    '@property int $a',
    '@property-read string $b desc',
    '@property-write Foo $c',
    '@phpstan-property int $a',
    '@psalm-property-read int $a',
    '@phan-property-write int $a',
    '@property int',
    '@method int foo()',
    '@method static int foo(int $a, string ...$b)',
    '@method static foo()',
    '@method static static foo()',
    '@method foo()',
    '@method foo(int $a = 1, $b = "x", ?Foo $c = null, array $d = [], $e = array(1 => 2))',
    '@method int foo(&$a, ...$b)',
    '@method T foo<T of object>(T $a)',
    '@method T foo<T, U = int>(T $a, U $b)',
    '@method int foo() Description.',
    '@method int',
    '@method int foo(',
    '@method int foo($a = Foo::BAR)',
    '@method int foo($a = Foo::*)',
    '@method int foo($a = -1.5e3)',
    '@method array{a: int} foo()',
    '@method (int|string) foo()',
    '@phpstan-method int foo()',
    '@psalm-method int foo()',
    // Templates.
    '@template T',
    '@template T of object',
    '@template T as object',
    '@template T of object = stdClass',
    '@template T super int',
    '@template T of int super int = int desc',
    '@template-covariant T',
    '@template-contravariant T of Foo',
    '@phpstan-template T',
    '@psalm-template-covariant T',
    '@template',
    '@template T of',
    '@template T of int|',
    // Extends and friends.
    '@extends Foo<int>',
    '@extends Foo<int> desc',
    '@extends Foo',
    '@extends Foo<int',
    '@implements Foo<int, string>',
    '@template-implements Foo<T>',
    '@use Foo<int>',
    '@template-use Foo<int>',
    '@phpstan-extends Foo<covariant T>',
    // Type aliases.
    '@phpstan-type Foo int|string',
    '@phpstan-type Foo = int',
    '@psalm-type Foo = array{a: int}',
    '@phan-type Foo int',
    '@phpstan-type Foo array{' . "\n" . ' *   a: int,' . "\n" . ' * }',
    '@phpstan-type Foo int extra',
    '@phpstan-type Foo array{a: // comment' . "\n" . ' *   broken',
    '@phpstan-type',
    '@phpstan-type Foo',
    '@phpstan-import-type Foo from Bar',
    '@phpstan-import-type Foo from Bar as Baz',
    '@psalm-import-type Foo from \\Bar\\Baz',
    '@phpstan-import-type Foo Bar',
    '@phpstan-import-type Foo from',
    // Asserts and outs.
    '@phpstan-assert int $a',
    '@phpstan-assert !null $a',
    '@phpstan-assert =int $a',
    '@phpstan-assert-if-true int $a',
    '@phpstan-assert-if-false int $this->prop',
    '@psalm-assert int $this->method()',
    '@phan-assert int $a->b',
    '@phpstan-assert int $this',
    '@phpstan-assert int',
    '@phpstan-assert int $a->',
    '@phpstan-assert int $a->b(',
    '@phpstan-this-out self<T>',
    '@phpstan-self-out static',
    '@psalm-this-out Foo',
];

$docs = [
    "/** */",
    "/**\n */",
    "/***/x",
    "/** @var int */",
    "/** @var int \$a */",
    "/** @param int \$a */",
    "/**\n * Summary.\n *\n * Description\n * on lines.\n *\n * @param int \$a\n * @return void\n */",
    "/**\r\n * Windows.\r\n * @param int \$a\r\n */",
    "/**\n * @param int \$a\n *\n *\n * @return void\n */",
    "/**\n * @foo bar\n * baz\n * @qux\n */",
    "/**\n * @foo bar @baz\n */",
    "/**\n * @foo(bar)\n */",
    "/**\n * @Foo(\n *   a=1,\n *   b={1, 2}\n * ) trailing\n */",
    "/**\n * @Foo(\n *   a=1,\n */",
    "/**\n * @\\Foo(\n */",
    "/** @\\Foo( */",
    "/**\n * @return int\n * @foo bar @param int \$a\n */",
    "/**\n * @foo text @param\n */",
    "/**\n * @foo text @param (x)\n */",
    "/**\n * @foo text @Bar(1) more\n */",
    "/**\n * @foo text@bar\n */",
    "/**\n\t * tabs\n\t */",
    "/**\n   no star\n   @param int \$a\n */",
    "/**\n * ** stars\n * @return int */",
    "/** @param int \$a */ ",
    "/**\n * @var int \$a\v vertical tab\n */",
    "/**\n * @var int \$a\f form feed\n */",
    "/**\n * bare cr\r here\n */",
    "/**\n * é unicode ünïcödé\n * @param Föö \$bär\n */",
    "/**\n * @param array{\n *   a: int,\n * } \$a\n */",
    "/**\n * @param array{\n *   a: int, // a\n *   // b\n *   b: string,\n * } \$a\n */",
    "/**\n * @return array{\n *   // only\n * }\n */",
    "/**\n * @return array{ // c\n *   a: int }\n */",
    "/**\n * @return array<\n *   // key\n *   int,\n *   string\n * >\n */",
    "/**\n * @return callable(\n *   int, // a\n *   string\n * ): void\n */",
    "/**\n * @phpstan-type A array{\n *   // comment\n *   a: int\n * }\n */",
    "/**\n * @phpstan-type A array{a: // c\n * int\n */",
    "/**\n * @var Foo<\n *   // c\n *   int\n * > \$a\n */",
    "/**\n * @param int \$a // comment\n */",
    "/**\n * //only a comment\n */",
    "/** // c */",
    "/** @var int // c */",
    "/**\n * @return (int|\n *   // c\n *   string)\n */",
    "/**\n * @param \$a is int ? 1 : 2\n */",
    "/**\n * @return \$a is int ? 1 : 2\n */",
    "/**\n * @return (\$a is int ? 1 : \$b is int ? 2 : 3)\n */",
    "/**\n * @return (\$a is int ? 1)\n */",
    "/** @return callable(): int|string */",
    "/** @return callable(): Foo<int */",
    "/** @return Foo[callable(): int */",
    "/** @return Closure(): (int|string)[] */",
    "/** @return Closure(): \$this[] */",
    "/** @return Closure(): Foo<int>[] */",
    "/** @return Closure(): object{a: int}[] */",
    "/** @return Closure(): list{int}[] */",
    "/** @return Closure(): 1[] */",
    "/** @return Foo<int>[]|Bar */",
    "/** @return Foo<div>x</div> */",
    "/** @return Foo<div> */",
    "/** @return Foo<div>x</span> */",
    "/** @return Foo<div>x <div/div> */",
    "/** @return array{a: int}[K] */",
    "/** @return \$this[K] */",
    "/** @return (A)[B] */",
    "/** @return A[B][] */",
    "/** @return A[ B] */",
    "/** @return A[B */",
    "/** @return 'a\\'b'|\"c\\\"d\\n\\x41\\101\\u{1F600}\\u{110000}\\e\" */",
    "/** @return 'a\\\\'|\"\\\\\\\"\" */",
    "/** @return array{'a\\tb': int, \"c\\x\": int} */",
    "/** @Foo(\"a\nb\") */",
    "/**\n * @Foo(\"line\n * break\") */",
    "/** @Foo(\"q\"\"q\") */",
    "/** @Foo({\"a\"=\"b\", 'c': 1, 2=3, FOO=4, Bar::BAZ=5, @X=6}) */",
    "/** @Foo({1: 2,}) */",
    "/** @Foo(a=1, a) */",
    "/** @Foo(Bar) */",
    "/** @Foo(Bar::class) */",
    "/** @Foo(-1, +2, 0x1F, 1_0, 1.5e-3) */",
    "/** @Foo(\"\") */",
    "/** @Foo(\"a\" \"b\") */",
    "/** @Foo(@) */",
    "/** @Foo(]) */",
    "/** @Foo({]}) */",
    "/** @Foo({a=}) */",
    "/** @Foo(Bar::) */",
    "/** @Foo(Bar::*Baz) */",
    "/** @Foo(Bar::* Baz) */",
    "/** @Foo(Bar::Baz*) */",
    "/** @var Bar::*Baz */",
    "/** @var Bar::BAZ* */",
    "/** @var Bar:: */",
    "/** @var array() */",
    "/** @var array(1) */",
    "/** @var [1, 2] */",
    "/** @var TRUE|False|NULL */",
    "/** @var \$THIS */",
    "/** @var \$this_thing */",
    "/** @param int &\$a */",
    "/** @param int & \$a */",
    "/** @param int &\n * \$a */",
    "/** @param A & B \$a */",
    "/** @param A&\$this */",
    "/** @param A&...\$a */",
    "/** @param A&=\$a */",
    "/** @method int foo(int &\$a, int & ...\$b) */",
    "/** @var 1. */",
    "/** @var 1._5 */",
    "/** @var -_1.5 */",
    "/** @var 1e */",
    "/** @var 1e+ */",
    "/** @var 1E5 */",
    "/** @var 0B11|0X1f|0O7 */",
    "/** @var 0b2 */",
    "/** @var 0x */",
    "/** @var +5 */",
    "/** @var -foo|bar */",
    "/** @var ..5 */",
    "/** @var \\1 */",
    "/** @var Foo\\ */",
    "/** @var \\Foo\\ \\Bar */",
    "/** @var Foo--bar */",
    "/** @var @foo */",
    "/** @var `a` */",
    "/** @var 'unterminated */",
    "/** @var \"unterminated */",
    "/** @var 'a\nb' */",
    "/** @param int \$a\n * @param string \$b\n * @param array<int, list<string>> \$c Description\n *   on two lines.\n * @return void */",
    "/**\n * @param\n * @return\n */",
    "/**\n * @return int\n *\n * more text\n */",
    "/**\n * @return int text\n *\n * more text\n */",
    "/**\n * text\n * @return int\n * trailing text */",
    "/**\n * @see Foo\n * continues\n * @see Bar */",
    "/** @Foo @Bar */",
    "/** @Foo\n * @Bar */",
    "/** @Foo(1) @Bar(2) */",
    "/** @Foo(1)\n * @Bar(2) */",
    "/** @Foo(1)text */",
    "/** @Foo(1) text @param int \$a */",
    "/** @Foo(1) text @param int */",
    "/** @Foo(1) text @param (int */",
    "/** @Foo(1) text @return int */",
    "/** @Foo(1) text @return */",
    "/** @Foo(1) text @return ( */",
    "/** @Foo(1) text @x\\y */",
    "/** @Foo(1) text @x(1) */",
    "/** @return int @Foo(1) */",
    "/** @return Foo<int>|Bar<string> desc */",
    "/**\n * @return int\n * @return-x\n */",
    "/**\n * Hello @param int \$a\n */",
    "/**\n * Hello\n * @param int \$a\n * @\n */",
    "/**\n * @@param\n */",
    "/**\n *@param int \$a\n */",
    "/**\n*@param int \$a\n*/",
    "/**    \n * x \n */",
    "/**\n * x  \n *  y\n */",
    "/**\n * \n * \n */",
];
foreach ($lines as $line) {
    $docs[] = "/** $line */";
    $docs[] = "/**\n * $line\n */";
    $docs[] = "/**\n * Summary.\n *\n * $line\n * @return void\n */";
}

// Seeded random comments assembled from fragments.
mt_srand(1678, MT_RAND_MT19937);
function pick(array $items): string
{
    return $items[mt_rand(0, count($items) - 1)];
}
function random_type(int $depth): string
{
    $atoms = ['int', 'string', 'mixed', 'Foo', '\\Foo\\Bar', 'T', 'static', '$this', 'null', 'true', '1', "'a'", '"b"', '-2.5', 'Foo::BAR', 'Foo::*', 'list', 'array', 'non-empty-string', 'callable', 'object', '?int', 'value-of<T>', 'key-of<Foo::A>'];
    if ($depth <= 0 || mt_rand(0, 9) < 4) {
        return pick($atoms);
    }
    switch (mt_rand(0, 13)) {
        case 0: return random_type($depth - 1) . '|' . random_type($depth - 1);
        case 1: return random_type($depth - 1) . '&' . random_type($depth - 1);
        case 2: return '?' . random_type($depth - 1);
        case 3: return random_type($depth - 1) . '[]';
        case 4: return pick(['array', 'list', 'Foo', 'iterable', 'int']) . '<' . random_type($depth - 1) . (mt_rand(0, 1) ? ', ' . random_type($depth - 1) : '') . '>';
        case 5: return pick(['array', 'list', 'non-empty-array']) . '{' . (mt_rand(0, 1) ? 'a: ' : '') . random_type($depth - 1) . (mt_rand(0, 1) ? ', b?: ' . random_type($depth - 1) : '') . (mt_rand(0, 3) === 0 ? ', ...' : '') . '}';
        case 6: return 'object{a: ' . random_type($depth - 1) . '}';
        case 7: return pick(['callable', 'Closure', '\\Closure']) . '(' . random_type($depth - 1) . (mt_rand(0, 1) ? ' $x' : '') . '): ' . random_type($depth - 1);
        case 8: return '(' . random_type($depth - 1) . ')';
        case 9: return '(' . pick(['$a', 'T']) . ' is ' . (mt_rand(0, 1) ? 'not ' : '') . random_type($depth - 1) . ' ? ' . random_type($depth - 1) . ' : ' . random_type($depth - 1) . ')';
        case 10: return random_type($depth - 1) . '[' . random_type($depth - 1) . ']';
        case 11: return 'int<' . pick(['0', 'min', '-1']) . ', ' . pick(['max', '10']) . '>';
        case 12: return random_type($depth - 1) . "|\n *   " . random_type($depth - 1);
        default: return 'Foo<covariant ' . random_type($depth - 1) . ', *>';
    }
}
function random_line(): string
{
    $type = random_type(3);
    $variable = pick(['$a', '$value', '...$rest', '&$ref', '$this', '']);
    $text = pick(['', 'Description.', 'more text here', '@inline', '{@see x}', '| bar', 'x|y']);
    switch (mt_rand(0, 15)) {
        case 0: return "@param $type $variable $text";
        case 1: return "@return $type $text";
        case 2: return "@var $type $variable $text";
        case 3: return "@throws $type";
        case 4: return "@template T of $type";
        case 5: return "@extends Foo<$type>";
        case 6: return "@method $type foo($type \$a = 1) $text";
        case 7: return "@property $type $variable";
        case 8: return "@phpstan-type Alias $type";
        case 9: return "@phpstan-assert $type $variable";
        case 10: return "@psalm-param $type $variable";
        case 11: return "@Foo(a=" . pick(['1', '"s"', '{1, 2}', '@Bar', 'X::Y']) . ") $text";
        case 12: return "@param-out $type $variable";
        case 13: return "@custom $text";
        case 14: return $text;
        default: return "@phpstan-return $type";
    }
}
$seen = array_flip($docs);
$target = count($docs) + 3000;
while (count($docs) < $target) {
    $lines = [];
    for ($n = mt_rand(1, 4); $n > 0; $n--) {
        $lines[] = random_line();
    }
    $doc = "/**\n * " . implode("\n * ", $lines) . "\n */";
    if (mt_rand(0, 2) === 0) {
        // Cut a random span out, so the parser meets a broken comment.
        $at = mt_rand(4, max(4, strlen($doc) - 3));
        $doc = substr($doc, 0, $at) . substr($doc, min(strlen($doc) - 2, $at + mt_rand(1, 6)));
    }
    if (phpdoc_is_doc_comment($doc) && mb_check_encoding($doc, 'UTF-8') && !isset($seen[$doc])) {
        $seen[$doc] = true;
        $docs[] = $doc;
    }
}

// Seeded random strings over the bytes the lexer gives meaning.
$alphabet = ['@', '$', '&', '|', '?', '!', '(', ')', '<', '>', '[', ']', '{', '}', ',', '.', ':', '=', '-', '+', '*', '/', "'", '"', '\\', '_', ' ', ' ', "\t", "\n", "\n * ", "\r\n", "\r", "\x0b", '0', '1', 'x', 'e', 'a', 'Z', 'é', 'is', 'of', 'as', 'param', 'var', 'return', 'int', 'this'];
$target = count($docs) + 2000;
while (count($docs) < $target) {
    $doc = '/** ';
    for ($n = mt_rand(1, 24); $n > 0; $n--) {
        $doc .= pick($alphabet);
    }
    $doc .= ' */';
    if (phpdoc_is_doc_comment($doc) && !isset($seen[$doc])) {
        $seen[$doc] = true;
        $docs[] = $doc;
    }
}

$records = [];
foreach ($docs as $doc) {
    if (!phpdoc_is_doc_comment($doc)) {
        continue;
    }
    $records[] = phpdoc_record($doc);
}
$header = ['php' => PHP_VERSION, 'phpstan' => $version, 'phpdocParser' => PHPDOC_PARSER_VERSION];
phpdoc_write(__DIR__ . '/phpdoc.expected.json', $header, $records);
fwrite(STDERR, count($records) . " cases\n");

$corpus = array_search('--corpus', $argv, true);
if ($corpus !== false) {
    $all = [];
    $corpora = [];
    foreach (['laravel', 'symfony'] as $name) {
        $docs = phpdoc_corpus("/tmp/rerail-php-corpus/$name");
        $corpora[] = ['name' => $name, 'docComments' => count($docs)];
        foreach ($docs as $doc) {
            $all[$doc] = true;
        }
    }
    $records = array_map('phpdoc_record', array_keys($all));
    phpdoc_write($argv[$corpus + 1], $header + ['corpora' => $corpora], $records);
    fwrite(STDERR, count($records) . " corpus doc comments\n");
}
