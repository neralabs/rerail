//! The Bitlex ladder in this crate (`ladder`): every step parses as the reference.

use crate::ladder::*;

/// Every step: the reference's nodes equal the interpreters' and the compiled path's on every
/// document; tier 1 compiled equals its interpreter; each step parses some documents and sends
/// some to tree-sitter. Printed per step: the grammar's sentences and the mutated copies apart,
/// and why the reference refused each sentence it refused.
#[test]
fn every_step_parses_as_the_reference() {
    let differ = check_ladder(&steps_in(DIR), b"<>();=#^ \nav9~,eE-{}", &[], &[], &|_, _, _, _| None);
    assert!(differ.is_empty(), "{} steps differ:\n{}", differ.len(), differ.join("\n"));
}


/// A gap mark's `before`s and `unless before`s (decision 0416), a token mark's run of tokens,
/// and token marks tier 2 tests alone (0407): a line break ends a header unless
/// the token after it is a `.`, which goes on the name, as a parser asks of the token it peeks
/// at; at the file's end the mark holds (`, end`). The reference's nodes equal the interpreter's
/// and Cranelift's, the mark tested in tier 2 at the token, past the comments between.
#[test]
fn a_gap_mark_is_decided_by_the_token_after() {
    let source = r#"lexer kotlin
class ws        = ' ' '\t' '\n'
class lf        = '\n'
class slash     = '/'
class star      = '*'
class word      = 'a'..'z' 'A'..'Z' '0'..'9' '_'
class alpha     = 'a'..'z' 'A'..'Z' '_'
class punct     = '.' '*'
mask line_open  = slash & next(slash)
regions hidden {
  line  = line_open .. lf outside
}
mask code       = ~hidden & ~o_line
mask w          = code & word
mask name       = fill(w & alpha, ~w)
error "a byte no token takes" by tokens when code & ~ws & ~name & ~punct
token comment = start slash & o_line read line
token simple_identifier = start alpha & name & ~prev(name) run word
token punct     = start code & punct
keywords simple_identifier { } soft { "package" "import" "as" }
keywords punct { "." "*" } required
skip ws
mark line       = lf & code, unless before "." simple_identifier, unless before "." "*", end, settles
mark more       = lf & code, before "." simple_identifier
mark wild       = tokens "." "*"
mark alias      = tokens after "import" simple_identifier (simple_identifier | ".")* "as"
tree kotlin {
  node source_file whole = [line] [package_header] import_header*
  node package_header   = 'package' identifier line
  node import_header    = 'import' (alias identifier 'as' simple_identifier as alias_name | identifier [wild '.' '*' as wildcard_import]) line
  node identifier       = simple_identifier ([more] '.' simple_identifier)*
  implied package_header 'package'
  implied import_header 'import'
  implied identifier '.'
  extra comment
}
"#;
    let fixed = ["package a\n.b\nimport c\n// x\n.d.*\n", "package a.b\n\nimport c\n", "package a\n// x\n\n.b", "import a\n.\n", "import a\n*", "import a.b.*\n", "import a\n// x\n.*", "// x\npackage a // y\nimport b.* // z\n", "import a // x\n.b // y\n", "import a.b as c\n", "import a\n.b as c\n", "import a as\n", "import a.b // x\n.c as d\n", "package a // x\n", "import a // x\n// y\n.b.*\n"];
    let c = compile_step("before", source);
    // The line breaks before a `.` and a name, or a `.*`, go on, past a comment; one before a
    // `*` ends the header; a `.` with no name after is refused. An alias, after a run of names
    // and dots, over a line break.
    let parses: Vec<bool> = fixed.iter().map(|d| reference(&c, d.as_bytes()).is_some()).collect();
    assert_eq!(parses, [true, true, true, false, false, true, true, true, true, true, true, false, true, true, true]);
    // `wild`, a token mark after the gap marks, holds no bit of the code (decision 0407).
    let r = c.program.block_programs[0].records.as_ref().expect("records");
    // The patterns: `line`'s two and `more`'s, in the code; `wild`'s and `alias`'s past it.
    let orders: Vec<(u8, bool)> = c.program.token_marks.iter().map(|m| (m.order, m.unless)).collect();
    assert_eq!((r.mark_bits(), orders), (2, vec![(0, true), (0, true), (1, false), (2, false), (3, false)]));
    assert_eq!(check_documents("before", &c, b"pa.*\n/ ", &[], &fixed, &|_, _, _, _| None), None);
}

/// A settling mark's `drops W` (Q235): where `W` holds at the token after it (here an `import`),
/// the extra before that token, after the mark's bit, goes; those before the bit are the header's.
/// Bitlex's reference and every executor.
#[test]
fn a_settling_mark_drops_in_the_reference() {
    let source = r#"lexer kotlin
class ws        = ' ' '\t' '\n'
class lf        = '\n'
class slash     = '/'
class star      = '*'
class word      = 'a'..'z' 'A'..'Z' '0'..'9' '_'
class alpha     = 'a'..'z' 'A'..'Z' '_'
class punct     = '.' '*'
class ichar     = 'i'
mask line_open  = slash & next(slash)
regions hidden {
  line  = line_open .. lf outside
}
mask code       = ~hidden & ~o_line
mask w          = code & word
mask name       = fill(w & alpha, ~w)
mask swallow    = code & ichar & ~prev(word)
error "a byte no token takes" by tokens when code & ~ws & ~name & ~punct
token comment = start slash & o_line read line
token simple_identifier = start alpha & name & ~prev(name) run word
token punct     = start code & punct
keywords simple_identifier { } soft { "package" "import" "as" }
keywords punct { "." "*" } required
skip ws
mark line       = lf & code, unless before "." simple_identifier, unless before "." "*", end, settles, drops swallow
mark more       = lf & code, before "." simple_identifier
mark wild       = tokens "." "*"
mark alias      = tokens after "import" simple_identifier (simple_identifier | ".")* "as"
tree kotlin {
  node source_file whole = [line] [package_header] import_header*
  node package_header   = 'package' identifier line
  node import_header    = 'import' (alias identifier 'as' simple_identifier as alias_name | identifier [wild '.' '*' as wildcard_import]) line
  node identifier       = simple_identifier ([more] '.' simple_identifier)*
  implied package_header 'package'
  implied import_header 'import'
  implied identifier '.'
  extra comment
}
"#;
    let c = compile_step("drops", source);
    let fixed = ["package a // x\n// y\nimport b\n", "package a\n// y\n// z\nimport b\n", "// x\nimport a // y\nimport b\n", "package a\n// y\n"];
    let comments = |d: &str| reference(&c, d.as_bytes()).expect("parses").iter().filter(|n| n.0 == c.ids.of("comment")).count();
    // The comment on the line before an `import` goes; the one after the header's name stays.
    assert_eq!(fixed.iter().map(|d| comments(d)).collect::<Vec<_>>(), [1, 1, 2, 1]);
    assert_eq!(check_documents("drops", &c, b"pai.*\n/ ", &[], &fixed, &|_, _, _, _| None), None);
}

/// A token mark's run that counts depths (Q271), as the lean parser's `type_args_end` does: a
/// `<` opens type arguments where its `>` closes the depth and a `(` follows, with `<` `>` and
/// `(` `)` counted between. The reference's nodes equal the interpreter's and Cranelift's.
#[test]
fn a_run_counts_its_depths() {
    let source = r#"lexer t
class ws        = ' ' '\n'
class word      = 'a'..'z' 'A'..'Z'
class punct     = '<' '>' '(' ')' ','
mask w          = word
mask name       = fill(w, ~w)
error "a byte no token takes" by tokens when ~ws & ~name & ~punct
token simple_identifier = start name & ~prev(name) run word
token punct     = start punct
keywords punct { "<" ">" "(" ")" "," } required
skip ws
mark generic    = tokens "<" (simple_identifier | "<" | ">" | "(" | ")" | ",")* nest ("<" to ">") ("(" to ")") ">" "("
tree t {
  node source_file whole = (item)*
  choice item           = type_open | simple_identifier | '<' | '>' | '(' | ')' | ','
  node type_open        = generic '<'
}
"#;
    let c = compile_step("nest", source);
    let fixed = ["f<A<B>,C>(x)", "f<A>(x)", "a<b>c>(d)", "a<b)>(c)", "a<(b>c)>(d)", "f<A<(B)>>(x) g<C>(y)", "a<b<c>(d)"];
    let open = c.ids.of("type_open");
    // Where the reference opens type arguments: at a `<` whose depth closes before a `(`.
    let opened: Vec<usize> = fixed.iter().map(|d| reference(&c, d.as_bytes()).expect("parses").iter().filter(|n| n.0 == open).count()).collect();
    // An inner `<` whose `>` is followed by a `,` or a `>` opens none, as in the lean parser.
    assert_eq!(opened, [1, 1, 0, 0, 0, 2, 1]);
    assert_eq!(check_documents("nest", &c, b"ab<>(), ", &[], &fixed, &|_, _, _, _| None), None);
}

/// A level that took a mark at a token does not end there at a hidden `ends` mark (0430 step
/// 1b, Q277): the line break's one decision is made, as the lean parser asks `asi` once. `goes`,
/// declared before `semi`, says a line that starts with `c` goes on a declaration; after it is
/// taken, `semi` (live at the same token) ends no level, and `c()` is the declaration's. Elsewhere
/// `semi` ends the declaration. Bitlex's reference does not implement `ends` (Q278): the
/// interpreter's nodes are the expected ones, and every executor's equal them.
#[test]
fn a_taken_mark_is_the_line_breaks_decision() {
    let source = r#"lexer t
class ws        = ' ' '\n'
class lf        = '\n'
class word      = 'a'..'z'
class punct     = '(' ')'
mask name       = fill(word, ~word)
error "a byte no token takes" by tokens when ~ws & ~name & ~punct
token simple_identifier = start name & ~prev(name) run word
token punct     = start punct
keywords simple_identifier { "k" } soft { "c" }
keywords punct { "(" ")" } required
skip ws
mark goes       = lf, before "c" "("
mark semi       = lf, end, ends
tree t {
  node source_file whole = (statement semi)*
  choice statement nested = decl | simple_identifier
  node decl             = 'k' simple_identifier [goes] [ctor]
  node ctor             = 'c' '(' ')'
  hidden goes semi
}
"#;
    let c = compile_step("decided", source);
    let decl = c.ids.of("decl");
    let ctor = c.ids.of("ctor");
    let count = |d: &str, k: u16| -> usize {
        let run = c.program.block_programs[0].run_files(&[d.as_bytes()]).remove(0);
        crate::stream::run(&c.program, d.as_bytes(), &run).map(|n| as_ids(n).iter().filter(|x| x.0 == k).count()).unwrap_or(usize::MAX)
    };
    let fixed = ["k a\nc()\n", "k a\nb\n", "k a c()\nb\n", "k a\nc\nb\n", "b\nk a\nc()"];
    assert_eq!(fixed.iter().map(|d| (count(d, decl), count(d, ctor))).collect::<Vec<_>>(), [(1, 1), (1, 0), (1, 1), (1, 0), (1, 1)]);
    // Cranelift's nodes and tape: the interpreter's.
    for d in fixed {
        let run = c.program.block_programs[0].run_files(&[d.as_bytes()]).remove(0);
        let nodes = crate::stream::run(&c.program, d.as_bytes(), &run).expect("the interpreter parses");
        let want = crate::stream::image(&nodes, &c.program.readings.extras, c.parser.tier2.widths_tag().1, d.as_bytes());
        assert_eq!(c.parser.parse(d.as_bytes()).ok().map(as_ids), Some(as_ids(nodes)), "{d:?}");
        assert_eq!(c.parser.image(d.as_bytes()).ok(), Some(want), "{d:?}");
    }
}

/// A gap mark read from the blanks (0430, step 2): `semi`'s mask is a line break in code, so
/// the compiler proves it holds at every line break between tokens and at none inside a token
/// (a comment's or a string's line breaks are their regions'), and the CPU's token loop tests
/// the line break class in the blank run before each token, as the lean lexer's `skip` notes
/// `nl`, with an extra's run carried to the grammar token after it. The reference's nodes equal
/// the interpreter's and Cranelift's, line breaks in comments and strings and between extras
/// included.
#[test]
fn a_gap_mark_read_from_the_blanks() {
    let source = r#"lexer t
class ws        = ' ' '\n'
class lf        = '\n'
class slash     = '/'
class star      = '*'
class dq        = '"'
class word      = 'a'..'z'
class punct     = '(' ')' ';'
mask line_open  = slash & next(slash)
mask block_open = slash & next(star)
mask block_end  = slash & prev(star)
regions hidden {
  line  = line_open .. lf outside
  block = block_open .. block_end
  str   = dq .. dq
}
mask code       = ~hidden & ~o_line & ~o_block & ~o_str
mask last_byte  = ~next(~empty)
error "a comment or a string open at the end of the file" by tokens when (f_block | f_str) & last_byte
mask w          = code & word
mask name       = fill(w, ~w)
error "a byte no token takes" by tokens when code & ~ws & ~name & ~punct
token line_comment = start slash & o_line read line
token multiline_comment = start slash & o_block read block
token string    = start o_str read str
token simple_identifier = start name & ~prev(name) run word
token punct     = start code & punct
keywords simple_identifier { "k" }
keywords punct { "(" ")" ";" } required
skip ws
mark semi       = lf & code unless after ";", end, ends
tree t {
  node source_file whole = (statement (semi | ';'))*
  choice statement nested = call | simple_identifier | string
  node call             = 'k' '(' [simple_identifier | string] ')'
  hidden semi
  extra line_comment multiline_comment
}
"#;
    let c = compile_step("blanks", source);
    let r = c.program.block_programs[0].records.as_ref().expect("records");
    assert_eq!(r.blanks.iter().map(|b| b.0).collect::<Vec<_>>(), vec![0], "semi is read from the blanks");
    let fixed = ["a\nb\n", "k(b)\nc", "a /* x\ny */ b\n", "a // x\nb\n", "k(\"x\ny\")\nb", "a;\nb\n", "a /* x */\n/* y */ b\n", "a\n\n// x\n\nb", "a b\n", "k /* \n */ (b)\n", "k(\nb)\n"];
    // The CPU's loop reads it so: an extra's mark is the loop's (its reader's), not a late one.
    let native = crate::block_native::BlockNative::lower(&c.program.block_programs[0]);
    let bw = crate::native_cpu_bytes::Bytewise::of(&native);
    assert_eq!(crate::native_cpu_block::blank_bits(r, &|o| bw.can(o)), 1);
    let samples: [(&str, &[&str]); 2] = [("line_comment", &["// x", "//", "// \"y", "// /* z"]), ("multiline_comment", &["/* x */", "/*\n*/", "/* \" \n */", "/**/"])];
    assert_eq!(check_documents("blanks", &c, b"abk(); \n/*\"x", &samples, &fixed, &|_, _, _, _| None), None);
}

/// A gap mark the token loop sets from the bytes, where the extra's mark is set after the
/// tokens (Q279): the loop cannot tell a comment from a grammar token, so the mark is set after
/// the tokens too, and a comment never takes its bit. One comment kind read by one reader of
/// two regions.
#[test]
fn a_loop_gap_mark_waits_for_a_late_extra() {
    let source = r#"lexer t
class ws        = ' ' '\n'
class lf        = '\n'
class slash     = '/'
class star      = '*'
class word      = 'a'..'z'
class punct     = '(' ')'
mask line_open  = slash & next(slash)
mask block_open = slash & next(star)
mask block_end  = slash & prev(star)
regions hidden {
  line  = line_open .. lf outside
  block = block_open .. block_end
}
mask code       = ~hidden & ~o_line & ~o_block
mask last_byte  = ~next(~empty)
error "a comment open at the end of the file" by tokens when f_block & last_byte
mask w          = code & word
mask name       = fill(w, ~w)
error "a byte no token takes" by tokens when code & ~ws & ~name & ~punct
token comment   = start slash & (o_line | o_block) read line | block
token simple_identifier = start name & ~prev(name) run word
token punct     = start code & punct
keywords punct { "(" ")" } required
skip ws
mark brk        = lf
mark call       = tokens simple_identifier "(" without brk
tree t {
  node source_file whole = (item)*
  choice item           = called | simple_identifier | '(' | ')'
  node called           = call simple_identifier
  hidden brk
  extra comment
}
"#;
    let c = compile_step("late extra", source);
    let fixed = ["f /* x */\n(x)", "f\n/* x */ (x)", "f /**/ (x)", "a // b\n(c)", "// c\n/**///\n(a)", "f(x) /* \n */ g (y)"];
    assert_eq!(check_documents("late extra", &c, b"ab() \n/*", &[], &fixed, &|_, _, _, _| None), None);
}

/// `without M` (0430): an element holds only at a token whose record has no bit of mark `M`, as
/// the lean parser asks `!peek_at(j).nl`. A name and a `(` with a line break between are no
/// call; with blanks, one. Bitlex's reference and every executor.
#[test]
fn a_token_without_a_mark() {
    let source = r#"lexer t
class ws        = ' ' '\n'
class lf        = '\n'
class word      = 'a'..'z'
class punct     = '(' ')'
mask name       = fill(word, ~word)
error "a byte no token takes" by tokens when ~ws & ~name & ~punct
token simple_identifier = start name & ~prev(name) run word
token punct     = start punct
keywords punct { "(" ")" } required
skip ws
mark brk        = lf
mark call       = tokens simple_identifier "(" without brk
tree t {
  node source_file whole = (item)*
  choice item           = called | simple_identifier | '(' | ')'
  node called           = call simple_identifier
  hidden brk
}
"#;
    let c = compile_step("without", source);
    let fixed = ["f(x)", "f (x)", "f\n(x)", "a b\n(c) d(e)", "a\n\n(", "(a)(b\n)"];
    let called = c.ids.of("called");
    let calls: Vec<usize> = fixed.iter().map(|d| reference(&c, d.as_bytes()).expect("parses").iter().filter(|n| n.0 == called).count()).collect();
    assert_eq!(calls, [1, 1, 0, 1, 0, 0]);
    assert_eq!(check_documents("without", &c, b"ab() \n", &[], &fixed, &|_, _, _, _| None), None);
}

/// A run's `ending (C)` (Q274): the rest is tried only where the run's last token, or the token
/// before it where it took none, is one of `C`, in Bitlex's reference and every executor.
#[test]
fn a_run_ends_on_a_token() {
    let source = r#"lexer t
class ws        = ' ' '\n'
class word      = 'a'..'z' 'A'..'Z'
class punct     = '<' '>' '(' ')' ','
mask w          = word
mask name       = fill(w, ~w)
error "a byte no token takes" by tokens when ~ws & ~name & ~punct
token simple_identifier = start name & ~prev(name) run word
token punct     = start punct
keywords punct { "<" ">" "(" ")" "," } required
skip ws
mark ann        = tokens "<" (~("(" | ","))* ending (simple_identifier) "("
tree t {
  node source_file whole = (item)*
  choice item           = ann_open | simple_identifier | '<' | '>' | '(' | ')' | ','
  node ann_open         = ann '<'
}
"#;
    let c = compile_step("ending", source);
    let fixed = ["<a(", "<(", "<a>(", "<a>b(", "<<a(", "a<b,c(", "<a"];
    let open = c.ids.of("ann_open");
    let opened: Vec<usize> = fixed.iter().map(|d| reference(&c, d.as_bytes()).expect("parses").iter().filter(|n| n.0 == open).count()).collect();
    // `<(`: the run took none, and the token before it, the `<`, is no name.
    assert_eq!(opened, [1, 0, 0, 1, 2, 0, 0]);
    assert_eq!(check_documents("ending", &c, b"ab<>(), ", &[], &fixed, &|_, _, _, _| None), None);
}

/// A rule made a call keeps the reference's meaning (doc 24 §2.3): a rule not `nested` takes
/// its caller's follow, so a hidden mark its caller takes after it ends it. `ops` is a call
/// under `Shape::Rules`; inlined, its loop stops at the line's `eol`, which `line` takes. As a
/// call it must stop there too, not read the next line's first word as its own.
#[test]
fn a_called_rule_ends_at_the_hidden_marks_its_caller_takes() {
    let source = r#"lexer t
class ws        = ' ' '\n'
class lf        = '\n'
class word      = 'a'..'z'
mask name       = fill(word, ~word)
error "a byte no token takes" by tokens when ~ws & ~name
token simple_identifier = start name & ~prev(name) run word
skip ws
mark eol        = lf, end
tree t {
  node source_file whole = (line)*
  node line       = simple_identifier ops eol
  node ops        = simple_identifier (simple_identifier)*
  hidden eol
}
"#;
    let fixed = ["a b c\nd e\n", "a b\n", "a b c\nd e", "a b\n\nc d e f\n"];
    for shape in [crate::bitlex_masks::Shape::Machine, crate::bitlex_masks::Shape::Rules] {
        let c = compile_step_shaped("follow", source, shape);
        let line = c.ids.of("line");
        let lines: Vec<usize> = fixed.iter().map(|d| reference(&c, d.as_bytes()).expect("parses").iter().filter(|n| n.0 == line).count()).collect();
        assert_eq!(lines, [2, 1, 2, 2], "the reference: a line a line break");
        assert_eq!(check_documents("follow", &c, b"ab \n", &[], &fixed, &|_, _, _, _| None), None, "{shape:?}");
    }
}

/// A called level that takes only a mark: `rest` is `eol` alone or words then `eol`, a call
/// under `Shape::Rules`. A taken mark is progress (a mark taken clears it and the marks before
/// it, and only `TOKEN` sets them again), so the verifier accepts the call, and it parses as
/// the reference.
#[test]
fn a_called_level_may_take_only_a_mark() {
    let source = r#"lexer t
class ws        = ' ' '\n'
class lf        = '\n'
class word      = 'a'..'z'
mask name       = fill(word, ~word)
error "a byte no token takes" by tokens when ~ws & ~name
token simple_identifier = start name & ~prev(name) run word
skip ws
mark eol        = lf, end
tree t {
  node source_file whole = (line)*
  node line       = simple_identifier rest
  choice rest     = eol | simple_identifier (simple_identifier)* eol
  hidden eol
}
"#;
    let fixed = ["a\nb c d\n", "a\n", "a b\nc\n", "a b c\n\nd\n"];
    for shape in [crate::bitlex_masks::Shape::Machine, crate::bitlex_masks::Shape::Rules] {
        let c = compile_step_shaped("mark_only", source, shape);
        assert_eq!(check_documents("mark_only", &c, b"ab \n", &[], &fixed, &|_, _, _, _| None), None, "{shape:?}");
        if shape != crate::bitlex_masks::Shape::Rules {
            continue;
        }
        // `rest`'s own marks kept (`MARK_KEEP`): taken by nobody, no progress, refused.
        let first_call = c.text.lines().filter_map(|l| l.strip_prefix("  rule ")).filter_map(|l| l.split_once(" @")).filter(|(r, _)| *r != "0").map(|(_, pc)| pc.to_owned()).min().expect("a called rule");
        let kept: Vec<String> = c
            .text
            .lines()
            .map(|l| match l.trim_start().split_once(" MARK m=") {
                Some((pc, rest)) if pc >= first_call.as_str() => {
                    let (m, operand) = rest.split_once(' ').expect("an operand");
                    format!("  {pc} MARK m={} {operand}", m.parse::<u8>().expect("a mode") | crate::program::MARK_KEEP)
                }
                _ => l.to_owned(),
            })
            .collect();
        let e = crate::rir_asm::assemble(&(kept.join("\n") + "\n")).err().expect("a kept mark is no progress");
        assert!(e.contains("NonProgressingCycle"), "{e}");
    }
}
