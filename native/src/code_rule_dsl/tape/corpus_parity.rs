//! Opt-in comparison of this worktree's tape executor with native trees.
//! Reports unsupported corpus forms explicitly; never substitutes the spike executor.
use super::*;
use crate::code_rule_dsl::blade::forte::tree::{BladeDoc, BladeLang};
use ast_grep_core::{
    AstGrep, Doc,
    matcher::{KindMatcher, Pattern},
    meta_var::MetaVariable,
    tree_sitter::StrDoc,
};
use serde_json::json;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};
use tree_sitter::StreamingIterator;

/// A file's native tree: tree-sitter's, or a Blade template's Forte tree.
enum Native {
    TreeSitter(StrDoc<GrammarDialect>),
    Blade(BladeDoc),
}

#[derive(Debug, PartialEq, Eq)]
struct NodeRecord {
    kind: String,
    kind_id: u16,
    named: bool,
    extra: bool,
    error: bool,
    missing: bool,
    field: u16,
    range: Range<usize>,
    start: (usize, usize, usize),
    end: (usize, usize, usize),
    parent: Option<usize>,
    children: Vec<usize>,
    next: Option<usize>,
    prev: Option<usize>,
    text: String,
}

fn native_records(
    node: tree_sitter::Node<'_>,
    source: &[u8],
    field: u16,
    parent: Option<usize>,
    records: &mut Vec<NodeRecord>,
) -> usize {
    let index = records.len();
    records.push(NodeRecord {
        kind: node.kind().to_owned(),
        kind_id: node.kind_id(),
        named: node.is_named(),
        extra: node.is_extra(),
        error: node.is_error(),
        missing: node.is_missing(),
        field,
        range: node.byte_range(),
        start: (
            node.start_position().row,
            node.start_position().column,
            node.start_byte(),
        ),
        end: (
            node.end_position().row,
            node.end_position().column,
            node.end_byte(),
        ),
        parent,
        children: Vec::new(),
        next: None,
        prev: None,
        text: node.utf8_text(source).unwrap().to_owned(),
    });
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = native_records(
                cursor.node(),
                source,
                cursor.field_id().map_or(0, |id| id.get()),
                Some(index),
                records,
            );
            records[index].children.push(child);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    let children = records[index].children.clone();
    for pair in children.windows(2) {
        records[pair[0]].next = Some(pair[1]);
        records[pair[1]].prev = Some(pair[0]);
    }
    index
}

fn tape_records(tape: &TapeDoc) -> Vec<NodeRecord> {
    tape.root_node()
        .dfs()
        .map(|node| NodeRecord {
            kind: node.kind().into_owned(),
            kind_id: node.kind_id(),
            named: node.is_named(),
            extra: node.is_extra(),
            error: node.is_error(),
            missing: node.is_missing(),
            field: tape.transducer_field(node.node_id() as u32),
            range: node.range(),
            start: {
                let position = node.start_pos();
                let (line, column) = position.byte_point();
                (line, column, node.range().start)
            },
            end: {
                let position = node.end_pos();
                let (line, column) = position.byte_point();
                (line, column, node.range().end)
            },
            parent: node.parent().map(|parent| parent.node_id()),
            children: node.children().map(|child| child.node_id()).collect(),
            next: node.next_sibling().map(|next| next.node_id()),
            prev: node.prev().map(|prev| prev.node_id()),
            text: tape.get_node_text(&node).into_owned(),
        })
        .collect()
}

fn native_syntax_error_ranges(root: tree_sitter::Node<'_>) -> Vec<Range<usize>> {
    let mut found = Vec::new();
    let mut stack = root
        .has_error()
        .then_some(root)
        .into_iter()
        .collect::<Vec<_>>();
    while let Some(node) = stack.pop() {
        if node.is_error() || node.is_missing() {
            found.push(node.byte_range());
            continue;
        }
        let mut cursor = node.walk();
        stack.extend(node.children(&mut cursor).filter(|child| child.has_error()));
    }
    found.sort_by_key(|range| (range.start, range.end));
    found.into_iter().fold(Vec::new(), |mut regions, range| {
        match regions.last_mut() {
            Some(last) if range.start < last.end || *last == range => {
                last.end = last.end.max(range.end);
            }
            _ => regions.push(range),
        }
        regions
    })
}

fn persisted_tree_parity(source: &str, language: GrammarDialect) -> Result<usize, String> {
    let grammar = Grammar::new(language).map_err(|error| error.to_string())?;
    let native = StrDoc::new(source, language);
    let image = TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false)?;
    let bytes = image.to_bytes()?;
    let decoded = TapeImage::from_bytes(&bytes, &grammar)?;
    if decoded.to_bytes()? != bytes {
        return Err("serialized tape did not round-trip canonically".into());
    }
    let tape = TapeDoc::restore(Arc::new(source.to_owned()), grammar, &decoded, &|| false)?;
    tree_parity(&native, source, &tape)
}

/// Every node's kind, flags, field, span, position, links and text, and the syntax error regions,
/// of a restored tape against the native tree it was encoded from.
fn tree_parity(
    native: &StrDoc<GrammarDialect>,
    source: &str,
    tape: &TapeDoc,
) -> Result<usize, String> {
    let mut expected = Vec::new();
    native_records(
        native.tree.root_node(),
        source.as_bytes(),
        0,
        None,
        &mut expected,
    );
    let actual = tape_records(tape);
    if expected != actual {
        let difference = expected
            .iter()
            .zip(&actual)
            .position(|(left, right)| left != right)
            .unwrap_or(expected.len().min(actual.len()));
        return Err(format!(
            "node record mismatch at {difference}: expected={:?}, actual={:?}",
            expected.get(difference),
            actual.get(difference)
        ));
    }
    if native_syntax_error_ranges(native.tree.root_node()) != tape.syntax_error_ranges() {
        return Err("syntax error range mismatch".into());
    }
    Ok(expected.len())
}

fn assert_persisted_tree_parity(source: &str, language: GrammarDialect) -> usize {
    persisted_tree_parity(source, language).unwrap()
}

type Matches = Vec<(Range<usize>, Vec<(String, Vec<Range<usize>>)>)>;
fn pattern_matches<D: Doc>(root: &AstGrep<D>, pattern: &Pattern) -> Matches {
    root.root()
        .find_all(pattern)
        .map(|found| {
            let env = found.get_env();
            let mut captures = Vec::new();
            for var in env.get_matched_variables() {
                match var {
                    MetaVariable::Capture(name, _) => {
                        let ranges = env
                            .get_match(&name)
                            .into_iter()
                            .map(|n| n.range())
                            .collect();
                        captures.push((name, ranges));
                    }
                    MetaVariable::MultiCapture(name) => {
                        let ranges = env
                            .get_multiple_matches(&name)
                            .into_iter()
                            .map(|n| n.range())
                            .collect();
                        captures.push((name, ranges));
                    }
                    _ => {}
                }
            }
            captures.sort_by(|a, b| a.0.cmp(&b.0));
            (found.get_node().range(), captures)
        })
        .collect()
}

#[test]
fn persisted_python_typescript_and_tsx_risky_nodes_match_native() {
    let fixtures = [
        (
            GrammarDialect::Python,
            "# extra\n@decorator\nclass C:\n    value = f\"{name!r}: {1 + 2}\"\n    broken = (\n",
        ),
        (
            GrammarDialect::TypeScript,
            "// extra\n@sealed\nclass C { value = `text ${name}`; broken = foo(; }\n",
        ),
        (
            GrammarDialect::Tsx,
            "// extra\n@sealed\nconst C = () => <View title={`text ${name}`} />;\nconst broken = <;\n",
        ),
    ];
    for (language, source) in fixtures {
        assert!(
            assert_persisted_tree_parity(source, language) > 1,
            "{language:?} fixture has no syntax tree"
        );
    }
}

/// PHP forms whose trees depend on the external scanner (heredoc, nowdoc, interpolation), on
/// grammar aliases (`name`, `variable_name`, casts, `static` keywords) or on error recovery,
/// including `?>` templates, which the PHP-only grammar does not read as text; and member,
/// scoped and static calls with named and unpacked arguments, so every `PHP_PATTERNS` entry
/// matches.
const PHP_RISKY_FIXTURES: &[&str] = &[
    "<?php\n# hash comment\n// line comment\n/** @var int $x */\n$x = 1;\n",
    "<?php\n$a = <<<EOT\n  Hello {$name->first} and ${legacy} and $arr[0]\n  EOT;\n$b = <<<'NOW'\nraw $x\nNOW;\n",
    "<?php\n$s = \"a {$o?->p} b $o->q c \\u{1F600} \\x41\";\n$t = 'single \\' quoted';\n$u = `ls $dir`;\n",
    "<?php\nnamespace App\\Http;\nuse Foo\\{Bar, Baz as Q};\nuse function strlen;\nuse const PHP_EOL;\n",
    "<?php\n#[Attr(1, name: 'x')]\nfinal readonly class C extends B implements I {\n    use T { a as protected b; }\n    public const int X = 1;\n    public function __construct(private readonly ?int $x = null, int ...$rest) {}\n    public static function f(): static { return new static(); }\n}\n",
    "<?php\nenum Suit: string { case Hearts = 'H'; public function color(): string { return match($this) { self::Hearts => 'Red', default => 'Black' }; } }\n",
    "<?php\n$f = fn($x) => $x * 2;\n$g = static function () use (&$y) { yield from gen(); };\n$h = strlen(...);\n$i = $o::class;\n$j = (int) $k;\n$l = $m ?? $n ?: $p;\n[$q, [, $r]] = $arr;\nlist('a' => $s) = $arr;\n",
    "<?php\nif ($a): ?>\n<div><?= $b ?></div>\n<?php elseif ($c): ?>text<?php else: ?>\n<?php endif; ?>\ntrailing html\n",
    "<?php\nswitch ($a) { case 1: break; default: continue 2; }\ntry { throw new E(); } catch (A|B $e) { } finally { goto end; }\nend:\ndeclare(strict_types=1);\n",
    "<?php\nfunction broken( { $x = [1, 2;\nclass {\n",
    "<?php\necho \"unterminated {$a\n",
    "<?php\n$x = (1 + 2;\n",
    "<?php\n$y = [1, 2;\n",
    "<?php\nif ($a) { f();\n",
    "<?php\nfunction handle(User $user, array $rest) {\n    $user->save();\n    $this->repo->find(id: 1, ...$rest);\n    $model = User::create(name: 'x', ...$rest);\n    static::boot(...$rest);\n    parent::__construct($user, flag: true);\n    $copy = new Copy($user, ...$rest);\n    if ($user->active) { notify($user, channel: 'mail'); }\n    $map = fn($x) => $x->id;\n    return $model;\n}\n",
    "<?php\nreturn 1\n",
];

/// Blade forms that are structurally meaningful before PHP injection: raw and escaped echoes,
/// directives, components and recovery, and the embedded PHP each makes (`blade::view`).
const BLADE_RISKY_FIXTURES: &[&str] = &[
    "{!! $html !!}\n{{ $name }}\n@if($condition)<x-alert :message=\"$name\" />@endif\n",
    "@foreach($items as $item)\n    <li>{{ $item }}</li>\n@endforeach\n",
    "@if($unfinished)\n",
    "@php\n    $x = [1, 2];\n@endphp\n@forelse($x as $y)\n{{ $loop->index }}\n@empty\nnone\n@endforelse\n",
    "@switch($a)\n@case(1)\none\n@break\n@default\nother\n@endswitch\n@use('App\\Models\\User', 'U')\n",
    "{{ $broken + }}\n<?php echo $raw; ?>\n<script>let a = @json($data);</script>\n",
    "@error('email')<p>{{ $message }}</p>@enderror\n@inject('metrics', 'App\\Metrics')\n{{-- {{ $hidden }} --}}\n@verbatim {{ $kept }} @endverbatim\n",
    // Recovery that makes the root an ERROR, so the program under it lies inside that error.
    "@foreach($items as $item)\n<li title=\"@lang('next')\">\n",
];

/// The line, column and byte of `byte` in `source`, as the tape's line table counts them.
fn point(source: &str, byte: usize) -> (usize, usize, usize) {
    let before = &source.as_bytes()[..byte];
    let line = before.iter().filter(|&&b| b == b'\n').count();
    let column = byte
        - before
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(0, |at| at + 1);
    (line, column, byte)
}

/// The records of the embedded program `view` makes from the native template, as a Blade tape
/// appends them under its root (`parent`, spanning `parent_range`): PHP's kinds and fields as the
/// tape numbers them (`numbering`), each node at its template span kept inside its parent's,
/// placeholders left out.
fn view_records(
    view: &crate::code_rule_dsl::blade::view::View,
    node: tree_sitter::Node<'_>,
    source: &str,
    field: u16,
    parent: (usize, Range<usize>),
    numbering: &crate::code_rule_dsl::language::Embedded,
    records: &mut Vec<NodeRecord>,
) -> Option<usize> {
    if view.omitted(node) {
        return None;
    }
    let span = view.span(node);
    let (low, high) = (parent.1.start, parent.1.end);
    let start = span.start.clamp(low, high);
    let range = start..span.end.clamp(start, high);
    let index = records.len();
    records.push(NodeRecord {
        kind: node.kind().to_owned(),
        kind_id: numbering.kind_code(node.kind_id()),
        named: node.is_named(),
        extra: node.is_extra(),
        error: node.is_error(),
        missing: node.is_missing(),
        field,
        range: range.clone(),
        start: point(source, range.start),
        end: point(source, range.end),
        parent: Some(parent.0),
        children: Vec::new(),
        next: None,
        prev: None,
        text: source[range.clone()].to_owned(),
    });
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let field = cursor
                .field_id()
                .map_or(0, |id| numbering.field_code(id.get()));
            if let Some(child) = view_records(
                view,
                cursor.node(),
                source,
                field,
                (index, range.clone()),
                numbering,
                records,
            ) {
                records[index].children.push(child);
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    let children = records[index].children.clone();
    for pair in children.windows(2) {
        records[pair[0]].next = Some(pair[1]);
        records[pair[1]].prev = Some(pair[0]);
    }
    Some(index)
}

/// A Blade tape, encoded, serialized and restored, against the template's Forte tree and the view
/// built from it (`blade_tree_parity`).
fn blade_tape_parity(source: &str) -> Result<(usize, usize), String> {
    let grammar = Grammar::new(GrammarDialect::Blade).map_err(|error| error.to_string())?;
    let image = TapeImage::encode_template(&grammar, source, &|| false)?;
    let bytes = image.to_bytes()?;
    let decoded = TapeImage::from_bytes(&bytes, &grammar)?;
    if decoded.to_bytes()? != bytes {
        return Err("serialized tape did not round-trip canonically".into());
    }
    let tape = TapeDoc::restore(Arc::new(source.to_owned()), grammar, &decoded, &|| false)?;
    blade_tree_parity(&parse_tree(source.as_bytes()), source, &tape)
}

/// The records of a template's Forte tree, as a Blade tape numbers its kinds and fields.
fn forte_records(
    node: crate::code_rule_dsl::blade::forte::tree::BladeNode<'_>,
    source: &str,
    parent: Option<usize>,
    records: &mut Vec<NodeRecord>,
) -> usize {
    let range = node.range();
    let index = records.len();
    records.push(NodeRecord {
        kind: node.kind().into_owned(),
        kind_id: node.kind_id(),
        named: node.is_named(),
        extra: false,
        error: false,
        missing: false,
        field: node.field_name().map_or(0, |name| {
            GrammarDialect::Blade
                .field_id(name)
                .expect("a catalog field")
        }),
        range: range.clone(),
        start: point(source, range.start),
        end: point(source, range.end),
        parent,
        children: Vec::new(),
        next: None,
        prev: None,
        text: source[range].to_owned(),
    });
    for child in node.children() {
        let child = forte_records(child, source, Some(index), records);
        records[index].children.push(child);
    }
    let children = records[index].children.clone();
    for pair in children.windows(2) {
        records[pair[0]].next = Some(pair[1]);
        records[pair[1]].prev = Some(pair[0]);
    }
    index
}

/// A restored Blade tape against the template's Forte tree and the view built from it: the tape's
/// first nodes are exactly the template's, and the rest, under the root's last child, exactly the
/// view's. Returns the template's node count and the view's.
fn blade_tree_parity(
    tree: &BladeTree,
    source: &str,
    tape: &TapeDoc,
) -> Result<(usize, usize), String> {
    let mut expected = Vec::new();
    forte_records(tree.root(), source, None, &mut expected);
    let template = expected.len();
    let reading = View::build(tree, source, &|| false)?;
    let mut errors = Vec::new();
    if let Some(view) = &reading.view {
        let numbering = GrammarDialect::Blade
            .kinds()
            .embedding(GrammarDialect::Php)
            .expect("Blade embeds PHP");
        let program = view_records(
            view,
            view.root(),
            source,
            0,
            (0, 0..source.len()),
            numbering,
            &mut expected,
        )
        .ok_or("the view's program is a placeholder")?;
        if let Some(&last) = expected[0].children.last() {
            expected[last].next = Some(program);
            expected[program].prev = Some(last);
        }
        expected[0].children.push(program);
        // The view's syntax errors, outermost first, as the tape reports them: none an ERROR
        // contains, and the program is under the root, which recovery can make an ERROR.
        let mut skip_until = 0;
        for (index, record) in expected.iter().enumerate() {
            if index >= skip_until && (record.error || record.missing) {
                if index >= template {
                    errors.push(record.range.clone());
                }
                skip_until = index + subtree_len(&expected, index);
            }
        }
    }
    // Then the template's own as Blade reads it: Forte's lexer errors, or the template it did
    // not parse (`view::Reading::syntax_errors`).
    errors.extend(reading.syntax_errors);
    let actual = tape_records(tape);
    if expected != actual {
        let difference = expected
            .iter()
            .zip(&actual)
            .position(|(left, right)| left != right)
            .unwrap_or(expected.len().min(actual.len()));
        return Err(format!(
            "node record mismatch at {difference} (template has {template}): expected={:?}, actual={:?}",
            expected.get(difference),
            actual.get(difference)
        ));
    }
    if errors != tape.syntax_error_ranges() {
        return Err(format!(
            "syntax error range mismatch: expected={errors:?}, actual={:?}",
            tape.syntax_error_ranges()
        ));
    }
    Ok((template, expected.len() - template))
}

/// The number of records in `index`'s subtree, itself included.
fn subtree_len(records: &[NodeRecord], index: usize) -> usize {
    1 + records[index]
        .children
        .iter()
        .map(|&child| subtree_len(records, child))
        .sum::<usize>()
}

#[test]
fn persisted_blade_risky_nodes_match_native() {
    let mut views = 0;
    for &source in BLADE_RISKY_FIXTURES {
        let (template, view) = blade_tape_parity(source).unwrap_or_else(|error| {
            panic!("{source}: {error}");
        });
        assert!(template > 1, "Blade fixture has no syntax tree: {source}");
        views += usize::from(view > 1);
    }
    assert_eq!(
        views,
        BLADE_RISKY_FIXTURES.len(),
        "every fixture embeds PHP"
    );
}

#[test]
fn persisted_php_risky_nodes_match_native() {
    let mut kinds = BTreeSet::new();
    for &source in PHP_RISKY_FIXTURES {
        assert!(
            assert_persisted_tree_parity(source, GrammarDialect::Php) > 1,
            "PHP fixture has no syntax tree: {source}"
        );
        let native = StrDoc::new(source, GrammarDialect::Php);
        let mut records = Vec::new();
        native_records(
            native.tree.root_node(),
            source.as_bytes(),
            0,
            None,
            &mut records,
        );
        kinds.extend(records.into_iter().map(|record| {
            if record.missing {
                "MISSING".to_owned()
            } else {
                record.kind
            }
        }));
    }
    let risky = [
        "heredoc",
        "nowdoc",
        "encapsed_string",
        "shell_command_expression",
        "attribute",
        "match_expression",
        "enum_declaration",
        "arrow_function",
        "nullsafe_member_access_expression",
        "variadic_placeholder",
        "cast_expression",
        "list_literal",
        "named_type",
        "ERROR",
        "MISSING",
    ];
    let absent: Vec<_> = risky
        .iter()
        .filter(|kind| !kinds.contains(**kind))
        .collect();
    assert!(absent.is_empty(), "fixtures no longer produce {absent:?}");
}

fn source_paths(
    root: &Path,
    extensions: &[&str],
    excluded_suffixes: &[&str],
    out: &mut Vec<PathBuf>,
) -> std::io::Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_symlink() {
            continue;
        }
        if ty.is_dir() {
            if entry.file_name() != ".git" {
                source_paths(&entry.path(), extensions, excluded_suffixes, out)?;
            }
        } else {
            let path = entry.path();
            let name = path.to_string_lossy();
            let included = extensions
                .iter()
                .any(|extension| name.ends_with(&format!(".{extension}")));
            let excluded = excluded_suffixes
                .iter()
                .any(|suffix| name.ends_with(&format!(".{suffix}")));
            if included && !excluded {
                out.push(path);
            }
        }
    }
    Ok(())
}

fn corpus_parity(
    root: &Path,
    paths: Vec<PathBuf>,
    language_for_path: impl Fn(&Path) -> GrammarDialect,
) -> serde_json::Value {
    let mut files_compared = 0usize;
    let mut nodes_compared = 0usize;
    let mut differences = Vec::new();
    for path in paths {
        let name = path.strip_prefix(root).unwrap_or(&path).to_string_lossy();
        match fs::read_to_string(&path)
            .map_err(|error| error.to_string())
            .and_then(|source| persisted_tree_parity(&source, language_for_path(&path)))
        {
            Ok(nodes) => {
                files_compared += 1;
                nodes_compared += nodes;
            }
            Err(error) => differences.push(json!({"path":name, "error":error})),
        }
    }
    json!({"filesCompared":files_compared,"nodesCompared":nodes_compared,"differences":differences})
}

#[test]
#[ignore = "full Python and TypeScript corpus tree parity through persisted tape format 3"]
fn persisted_python_and_typescript_corpora_match_native() {
    let python_root =
        PathBuf::from(std::env::var("RERAIL_PYTHON_CORPUS").expect("RERAIL_PYTHON_CORPUS"));
    let manifest =
        PathBuf::from(std::env::var("RERAIL_PYTHON_MANIFEST").expect("RERAIL_PYTHON_MANIFEST"));
    let web_root =
        PathBuf::from(std::env::var("RERAIL_TYPESCRIPT_CORPUS").expect("RERAIL_TYPESCRIPT_CORPUS"));
    let output = PathBuf::from(
        std::env::var("RERAIL_TAPE_PARITY_REPORT").expect("RERAIL_TAPE_PARITY_REPORT"),
    );
    let python_paths: Vec<PathBuf> =
        serde_json::from_slice::<Vec<String>>(&fs::read(&manifest).expect("read Python manifest"))
            .expect("parse Python manifest")
            .into_iter()
            .map(|path| python_root.join(path))
            .collect();
    let python = corpus_parity(&python_root, python_paths, |_| GrammarDialect::Python);
    let mut typescript_paths = Vec::new();
    source_paths(&web_root, &["ts", "tsx"], &[], &mut typescript_paths)
        .expect("read web source tree");
    typescript_paths.sort();
    let typescript = corpus_parity(&web_root, typescript_paths, |path| {
        if path.extension().is_some_and(|extension| extension == "tsx") {
            GrammarDialect::Tsx
        } else {
            GrammarDialect::TypeScript
        }
    });
    let report = json!({"tapeFormat":TAPE_FORMAT_VERSION,"python":python,"typescript":typescript});
    fs::write(&output, serde_json::to_vec_pretty(&report).unwrap()).expect("write parity report");
    assert!(
        report["python"]["differences"]
            .as_array()
            .unwrap()
            .is_empty()
            && report["typescript"]["differences"]
                .as_array()
                .unwrap()
                .is_empty(),
        "tree parity differences; see {}",
        output.display()
    );
}

#[test]
fn persisted_go_structural_captures_match_native() {
    for source in [
        "package p\nfunc f(x int) { g(x, /* comment */ x); return }\n",
        "package p\nfunc broken( { g(\"é\", 2) }",
    ] {
        let grammar = Grammar::new(GrammarDialect::Go).unwrap();
        let native = StrDoc::new(source, GrammarDialect::Go);
        let image =
            TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap();
        let bytes = image.to_bytes().unwrap();
        let decoded = TapeImage::from_bytes(&bytes, &grammar).unwrap();
        assert_eq!(bytes, decoded.to_bytes().unwrap());
        let tape = TapeDoc::restore(Arc::new(source.into()), grammar, &decoded, &|| false).unwrap();
        let pattern = Pattern::try_new("$F($$$ARGS)", GrammarDialect::Go).unwrap();
        assert_eq!(
            pattern_matches(&AstGrep::doc(native), &pattern),
            pattern_matches(&AstGrep::doc(tape), &pattern)
        );
    }
}

#[test]
fn persisted_php_structural_captures_match_native() {
    let grammar = Grammar::new(GrammarDialect::Php).unwrap();
    let patterns: Vec<_> = PHP_PATTERNS
        .iter()
        .map(|pattern| Pattern::try_new(pattern, GrammarDialect::Php).unwrap())
        .collect();
    let mut matched = vec![0; PHP_PATTERNS.len()];
    for &source in PHP_RISKY_FIXTURES {
        let native = StrDoc::new(source, GrammarDialect::Php);
        let image =
            TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap();
        let decoded = TapeImage::from_bytes(&image.to_bytes().unwrap(), &grammar).unwrap();
        let tape = TapeDoc::restore(Arc::new(source.into()), grammar.clone(), &decoded, &|| {
            false
        })
        .unwrap();
        let (native, tape) = (AstGrep::doc(native), AstGrep::doc(tape));
        for ((text, pattern), count) in PHP_PATTERNS.iter().zip(&patterns).zip(&mut matched) {
            let expected = pattern_matches(&native, pattern);
            *count += expected.len();
            assert_eq!(
                expected,
                pattern_matches(&tape, pattern),
                "{text} over {source}"
            );
        }
    }
    let unmatched: Vec<_> = PHP_PATTERNS
        .iter()
        .zip(&matched)
        .filter(|(_, count)| **count == 0)
        .map(|(text, _)| text)
        .collect();
    assert!(
        unmatched.is_empty(),
        "PHP patterns no fixture matches: {unmatched:?}"
    );
}

#[test]
fn persisted_blade_structural_captures_match_native() {
    let grammar = Grammar::new(GrammarDialect::Blade).unwrap();
    let patterns = ["{!! $X !!}", "{{ $X }}", "@if($C)"];
    let patterns: Vec<_> = patterns
        .iter()
        .map(|pattern| Pattern::try_new(pattern, GrammarDialect::Blade).unwrap())
        .collect();
    let mut matched = 0;
    for &source in BLADE_RISKY_FIXTURES {
        let native = BladeLang.doc(source.as_bytes());
        let image = TapeImage::encode_template(&grammar, source, &|| false).unwrap();
        let decoded = TapeImage::from_bytes(&image.to_bytes().unwrap(), &grammar).unwrap();
        let tape = TapeDoc::restore(Arc::new(source.into()), grammar.clone(), &decoded, &|| {
            false
        })
        .unwrap();
        let (native, tape) = (AstGrep::doc(native), AstGrep::doc(tape));
        for pattern in &patterns {
            let expected = pattern_matches(&native, pattern);
            matched += expected.len();
            assert_eq!(expected, pattern_matches(&tape, pattern));
        }
    }
    assert!(matched > 0, "no Blade pattern matched a fixture");
}

/// A corpus the oracle benchmark reads: its grammar, the extensions that select its files, the
/// environment prefix naming its root, report and shard (`{env}_CORPUS`, `{env}_REPORT`,
/// `{env}_SHARDS`, `{env}_SHARD`), and the structural ast-grep patterns compared.
struct OracleCorpus {
    name: &'static str,
    language: GrammarDialect,
    extensions: &'static [&'static str],
    excluded_suffixes: &'static [&'static str],
    env: &'static str,
    patterns: &'static [&'static str],
}

const GO_PATTERNS: &[&str] = &[
    "$F($$$ARGS)",
    "$X.$FIELD",
    "return $$$VALUES",
    "var $NAME = $VALUE",
    "$LEFT := $RIGHT",
    "if $COND { $$$BODY }",
    "func $NAME($$$PARAMS) { $$$BODY }",
];

const PHP_PATTERNS: &[&str] = &[
    "$F($$$ARGS)",
    "$X->$M($$$ARGS)",
    "$C::$M($$$ARGS)",
    "new $C($$$ARGS)",
    "$X->$P",
    "return $V;",
    "$L = $R",
    "if ($COND) { $$$BODY }",
    "function $NAME($$$PARAMS) { $$$BODY }",
    "fn($$$PARAMS) => $BODY",
];

#[test]
#[ignore = "full Go corpus: native tree-sitter / ast-grep versus Rerail tapes"]
fn benchmark_go_worktree_oracle_parity() {
    worktree_oracle_parity(&OracleCorpus {
        name: "Go",
        language: GrammarDialect::Go,
        extensions: &["go"],
        excluded_suffixes: &[],
        env: "RERAIL_GO",
        patterns: GO_PATTERNS,
    });
}

const PYTHON_PATTERNS: &[&str] = &[
    "$F($$$ARGS)",
    "$X.$A",
    "return $V",
    "$L = $R",
    "if $COND: $$$BODY",
    "def $NAME($$$PARAMS): $$$BODY",
    "lambda $$$PARAMS: $BODY",
    "import $M",
];

#[test]
#[ignore = "full Python corpus: native tree-sitter / ast-grep versus Rerail tapes"]
fn benchmark_python_worktree_oracle_parity() {
    worktree_oracle_parity(&OracleCorpus {
        name: "Python",
        language: GrammarDialect::Python,
        extensions: &["py", "pyi"],
        excluded_suffixes: &[],
        env: "RERAIL_PYTHON_ORACLE",
        patterns: PYTHON_PATTERNS,
    });
}

#[test]
#[ignore = "full PHP corpus: native tree-sitter / ast-grep versus Rerail tapes"]
fn benchmark_php_worktree_oracle_parity() {
    worktree_oracle_parity(&OracleCorpus {
        name: "PHP",
        language: GrammarDialect::Php,
        extensions: &["php"],
        excluded_suffixes: &["blade.php"],
        env: "RERAIL_PHP",
        patterns: PHP_PATTERNS,
    });
}

#[test]
#[ignore = "full Blade corpus: native tree-sitter / ast-grep versus Rerail tapes"]
fn benchmark_blade_worktree_oracle_parity() {
    worktree_oracle_parity(&OracleCorpus {
        name: "Blade",
        language: GrammarDialect::Blade,
        extensions: &["blade.php"],
        excluded_suffixes: &[],
        env: "RERAIL_BLADE",
        patterns: &["{!! $X !!}", "{{ $X }}", "@if($C)"],
    });
}

fn worktree_oracle_parity(corpus: &OracleCorpus) {
    let var = |suffix: &str| std::env::var(format!("{}_{suffix}", corpus.env));
    let root = PathBuf::from(var("CORPUS").expect("corpus root"));
    let output = var("REPORT").expect("report path");
    let shards: usize = var("SHARDS").unwrap_or("1".into()).parse().unwrap();
    let shard: usize = var("SHARD").unwrap_or("0".into()).parse().unwrap();
    assert!(shards > 0 && shard < shards);
    let mut paths = Vec::new();
    source_paths(
        &root,
        corpus.extensions,
        corpus.excluded_suffixes,
        &mut paths,
    )
    .expect("read corpus tree");
    paths.sort();
    let inventory_count = paths.len();
    let language = corpus.language;
    let grammar = Grammar::new(language).unwrap();
    // These are real .scm queries executed by the native QueryCursor. Their explicit bridge
    // uses the existing KindMatcher on TapeDoc, not another SQL implementation. Blade has no
    // tree-sitter grammar: its tree is compared node by node and by pattern.
    let queries: Vec<_> = match grammar.kinds.grammar() {
        Some(ts) => {
            let mut names = BTreeSet::new();
            for id in 0..ts.node_kind_count() as u16 {
                if ts.node_kind_is_named(id) && ts.node_kind_is_visible(id) {
                    names.insert(ts.node_kind_for_id(id).unwrap().to_owned());
                }
            }
            names
                .into_iter()
                .filter_map(|kind| {
                    let text = format!("({kind}) @root");
                    let query = tree_sitter::Query::new(ts, &text).ok()?;
                    Some((kind.clone(), query, KindMatcher::new(&kind, language)))
                })
                .collect()
        }
        None => Vec::new(),
    };
    let patterns: Vec<_> = corpus
        .patterns
        .iter()
        .map(|p| Pattern::try_new(p, language).unwrap())
        .collect();
    let mut native_ms = vec![0.0; queries.len()];
    let mut tape_ms = vec![0.0; queries.len()];
    let mut counts = vec![0usize; queries.len()];
    let mut pattern_native_ms = vec![0.0; patterns.len()];
    let mut pattern_tape_ms = vec![0.0; patterns.len()];
    let mut pattern_counts = vec![0usize; patterns.len()];
    let mut excluded = Vec::new();
    let mut failures = Vec::new();
    let mut eligible = 0usize;
    let mut source_bytes = 0usize;
    let mut node_count = 0usize;
    let mut tape_bytes = 0usize;
    let mut syntax_error_files = Vec::new();
    let mut largest = serde_json::Value::Null;
    let mut largest_bytes = 0usize;
    let mut parse_ms = 0.0;
    let mut encode_ms = 0.0;
    let mut restore_ms = 0.0;
    let mut serialize_ms = 0.0;
    let mut manifest_digest = Sha256::new();
    let mut result_digest = Sha256::new();
    let started = Instant::now();
    for (ordinal, path) in paths.into_iter().enumerate() {
        if ordinal % shards != shard {
            continue;
        }
        let path_name = path
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .to_string();
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                failures.push(json!({"path":path_name,"error":error.to_string()}));
                continue;
            }
        };
        let hash = format!("{:x}", Sha256::digest(&bytes));
        let reason = if bytes.is_empty() {
            Some("empty")
        } else if bytes.len() > 4_000_000 {
            Some("over4000000Bytes")
        } else {
            None
        };
        if let Some(reason) = reason {
            excluded.push(
                json!({"path":path_name,"sha256":hash,"bytes":bytes.len(),"excluded":reason}),
            );
            continue;
        }
        let source = match String::from_utf8(bytes) {
            Ok(source) => source,
            Err(error) => {
                excluded.push(json!({"path":path_name,"sha256":hash,"bytes":error.as_bytes().len(),"excluded":"invalidUtf8","firstInvalidByte":error.utf8_error().valid_up_to()}));
                continue;
            }
        };
        manifest_digest.update(serde_json::to_vec(&(&path_name, &hash)).unwrap());
        eligible += 1;
        source_bytes += source.len();
        let t = Instant::now();
        let native = match language {
            GrammarDialect::Blade => Native::Blade(BladeLang.doc(source.as_bytes())),
            // The tree the workspace tapes: the family's parse input (a PHP file's view), whose
            // nodes read the source.
            _ => Native::TreeSitter({
                let input = crate::code_rule_dsl::language::adapter::parse_input(language, &source);
                let mut doc = StrDoc::new(&input, language);
                doc.src = source.to_string();
                doc
            }),
        };
        let file_parse_ms = t.elapsed().as_secs_f64() * 1000.0;
        parse_ms += file_parse_ms;
        let has_error = match &native {
            Native::TreeSitter(doc) => doc.tree.root_node().has_error(),
            Native::Blade(doc) => !doc.tree.error_ranges().is_empty(),
        };
        if has_error {
            syntax_error_files.push(path_name.clone());
        }
        let t = Instant::now();
        // A Blade template's tape is encoded from its own parse: Forte's tree is not kept.
        let encoded = match &native {
            Native::TreeSitter(doc) => {
                TapeImage::encode(doc.tree.root_node(), &grammar, &source, &|| false)
            }
            Native::Blade(_) => TapeImage::encode_template(&grammar, &source, &|| false),
        };
        let image = match encoded {
            Ok(image) => image,
            Err(error) => {
                failures.push(json!({"path":path_name,"error":error}));
                continue;
            }
        };
        let file_encode_ms = t.elapsed().as_secs_f64() * 1000.0;
        encode_ms += file_encode_ms;
        let t = Instant::now();
        let persisted = image.to_bytes().unwrap();
        let decoded = TapeImage::from_bytes(&persisted, &grammar).unwrap();
        let file_serialize_ms = t.elapsed().as_secs_f64() * 1000.0;
        serialize_ms += file_serialize_ms;
        if decoded.to_bytes().unwrap() != persisted {
            failures.push(json!({"path":path_name,"error":"nonCanonicalTape"}));
        }
        let t = Instant::now();
        let tape = TapeDoc::restore(Arc::new(source.clone()), grammar.clone(), &decoded, &|| {
            false
        })
        .unwrap();
        let file_restore_ms = t.elapsed().as_secs_f64() * 1000.0;
        restore_ms += file_restore_ms;
        result_digest.update(serde_json::to_vec(&path_name).unwrap());
        let nodes = tape.transducer_len();
        node_count += nodes;
        tape_bytes += persisted.len();
        // Compare the entire node stream (kind, flags, field, span, position, links, text) and
        // the syntax error regions, not only the nodes selected by queries; a Blade tape's
        // template part against the native tree and its embedded view against the native view.
        let parity = match &native {
            Native::Blade(doc) => blade_tree_parity(&doc.tree, &source, &tape).map(|_| ()),
            Native::TreeSitter(doc) => tree_parity(doc, &source, &tape).map(|_| ()),
        };
        if let Err(error) = parity {
            failures.push(json!({"path":path_name,"error":error}));
        }
        let ast = AstGrep::doc(tape.clone());
        for (i, (kind, query, matcher)) in queries.iter().enumerate() {
            let Native::TreeSitter(doc) = &native else {
                unreachable!("Blade has no tree-sitter queries");
            };
            let t = Instant::now();
            let mut qc = tree_sitter::QueryCursor::new();
            let mut matches = qc.matches(query, doc.tree.root_node(), source.as_bytes());
            let mut expected = Vec::new();
            while let Some(m) = matches.next() {
                expected.push(m.captures()[0].node.byte_range());
            }
            native_ms[i] += t.elapsed().as_secs_f64() * 1000.0;
            drop(matches);
            assert!(
                !qc.did_exceed_match_limit(),
                "incomplete native query: {path_name} {kind}"
            );
            let t = Instant::now();
            let actual: Vec<_> = ast
                .root()
                .find_all(matcher)
                .map(|m| m.get_node().range())
                .collect();
            tape_ms[i] += t.elapsed().as_secs_f64() * 1000.0;
            counts[i] += expected.len();
            result_digest.update(serde_json::to_vec(&(kind, &actual)).unwrap());
            if actual != expected {
                failures.push(
                    json!({"path":path_name,"query":kind,"expected":expected,"actual":actual}),
                );
            }
        }
        let native_matches: Box<dyn Fn(&Pattern) -> Matches> = match native {
            Native::TreeSitter(doc) => {
                let root = AstGrep::doc(doc);
                Box::new(move |pattern| pattern_matches(&root, pattern))
            }
            Native::Blade(doc) => {
                let root = AstGrep::doc(doc);
                Box::new(move |pattern| pattern_matches(&root, pattern))
            }
        };
        for (i, pattern) in patterns.iter().enumerate() {
            let t = Instant::now();
            let expected = native_matches(pattern);
            pattern_native_ms[i] += t.elapsed().as_secs_f64() * 1000.0;
            let t = Instant::now();
            let actual = pattern_matches(&ast, pattern);
            pattern_tape_ms[i] += t.elapsed().as_secs_f64() * 1000.0;
            pattern_counts[i] += expected.len();
            result_digest.update(serde_json::to_vec(&(corpus.patterns[i], &actual)).unwrap());
            if actual != expected {
                failures.push(json!({"path":path_name,"pattern":corpus.patterns[i],"expected":expected,"actual":actual}));
            }
        }
        if source.len() > largest_bytes {
            largest_bytes = source.len();
            largest = json!({"path":path_name,"sha256":hash,"bytes":source.len(),"nodes":nodes,"parseMs":file_parse_ms,"encodeMs":file_encode_ms,"serializeDecodeMs":file_serialize_ms,"restoreMs":file_restore_ms,"tapeBytes":persisted.len()});
        }
    }
    let report = json!({
        "benchmark": format!("{} worktree persisted-tape oracle parity", corpus.name),
        "tapeFormat": TAPE_FORMAT_VERSION,
        "executor": "this worktree: ast-grep over TapeDoc",
        "oracle": "native Tree-sitter node records for every node; native Tree-sitter QueryCursor for kind queries; native tree-backed ast-grep for structural patterns",
        "scope": format!("Full node-record parity, a generated kind-query bridge and {} structural ast-grep patterns. Not full harvested .scm corpus parity. No FTS filtering, SQL composition, or output publication.", corpus.patterns.len()),
        "shard": shard,
        "shards": shards,
        "coverage": {
            "enumeratedFiles": inventory_count,
            "eligibleFiles": eligible,
            "sourceBytes": source_bytes,
            "nodes": node_count,
            "tapeBytes": tape_bytes,
            "filesWithSyntaxErrors": syntax_error_files,
            "manifestSha256": format!("{:x}", manifest_digest.finalize()),
            "sourceIdentity": "SHA-256 over the ordered (relative path, file SHA-256) pairs of eligible files",
            "excluded": excluded,
            "largest": largest,
        },
        "failures": failures,
        "parseMs": parse_ms,
        "encodeMs": encode_ms,
        "restoreMs": restore_ms,
        "serializeDecodeMs": serialize_ms,
        "orderedResultSha256": format!("{:x}", result_digest.finalize()),
        "wallMs": started.elapsed().as_secs_f64() * 1000.0,
        "kindQueries": queries.iter().enumerate().map(|(i, (kind, _, _))| json!({"query":format!("({kind}) @root"),"matches":counts[i],"nativeMs":native_ms[i],"rerailMs":tape_ms[i]})).collect::<Vec<_>>(),
        "structuralPatterns": corpus.patterns.iter().enumerate().map(|(i, p)| json!({"pattern":p,"matches":pattern_counts[i],"nativeMs":pattern_native_ms[i],"rerailMs":pattern_tape_ms[i]})).collect::<Vec<_>>(),
    });
    fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    assert!(
        report["failures"].as_array().unwrap().is_empty(),
        "{} parity failures (see report)",
        report["failures"].as_array().unwrap().len()
    );
}
