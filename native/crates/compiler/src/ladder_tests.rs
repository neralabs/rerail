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


/// A gap mark's `before` and `unless before` (decision 0416), and a token mark tier 2 tests
/// alone (0407): a line break ends a header unless
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
keywords simple_identifier { } soft { "package" "import" }
keywords punct { "." "*" } required
skip ws
mark line       = lf & code, unless before ".", end
mark more       = lf & code, before "."
mark wild       = tokens "." "*"
tree kotlin {
  node source_file whole = [line] [package_header] import_header*
  node package_header   = 'package' identifier line
  node import_header    = 'import' identifier [wild '.' '*' as wildcard_import] line
  node identifier       = simple_identifier ([more] '.' simple_identifier)*
  implied package_header 'package'
  implied import_header 'import'
  implied identifier '.'
  extra comment
}
"#;
    let fixed = ["package a\n.b\nimport c\n// x\n.d.*\n", "package a.b\n\nimport c\n", "package a\n// x\n\n.b", "import a\n.\n", "import a\n*", "import a.b.*\n", "import a\n// x\n.*", "// x\npackage a // y\nimport b.* // z\n", "import a // x\n.b // y\n"];
    let c = compile_step("before", source);
    // The line breaks before a `.` go on the name, past a comment; one before a `*` ends the
    // header; a `.` with no name after is refused.
    let parses: Vec<bool> = fixed.iter().map(|d| reference(&c, d.as_bytes()).is_some()).collect();
    assert_eq!(parses, [true, true, true, false, false, true, false, true, true]);
    // `wild`, a token mark after the gap marks, holds no bit of the code (decision 0407).
    let r = c.program.block_programs[0].records.as_ref().expect("records");
    // The patterns: `line`'s and `more`'s token after, in the code; `wild`'s past it.
    let orders: Vec<u8> = c.program.token_marks.iter().map(|m| m.order).collect();
    assert_eq!((r.mark_bits(), orders), (2, vec![0, 1, 2]));
    assert_eq!(check_documents("before", &c, b"pa.*\n/ ", &[], &fixed, &|_, _, _, _| None), None);
}
