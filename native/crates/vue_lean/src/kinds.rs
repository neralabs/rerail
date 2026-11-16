// Written by gen_kinds.py from the token grammar; do not edit.

/// Node kinds, as the tape names them: tree-sitter-vue's, named and anonymous.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum K {
    Document,
    Doctype,
    Element,
    ScriptElement,
    StartTag,
    StyleElement,
    TagName,
    ScriptStartTag,
    StyleStartTag,
    SelfClosingTag,
    EndTag,
    ErroneousEndTag,
    Attribute,
    AttributeName,
    AttributeValue,
    Entity,
    QuotedAttributeValue,
    Text,
    TemplateElement,
    TemplateStartTag,
    Interpolation,
    RawText,
    DirectiveAttribute,
    DirectiveName,
    DirectiveValue,
    DynamicDirectiveValue,
    DynamicDirectiveInnerValue,
    DirectiveModifiers,
    DirectiveModifier,
    ErroneousEndTagName,
    Comment,
    TLtBang,
    TDoctype,
    TGt,
    TLt,
    TSlashGt,
    TLtSlash,
    TEq,
    TQuote,
    TDQuote,
    TLBraceLBrace,
    TRBraceRBrace,
    TColon,
    TDot,
    TAt,
    THash,
    TLBracket,
    TRBracket,
}

/// Each kind's name and whether it is named, in `K`'s order.
pub const KINDS: &[(&str, bool)] = &[
    ("document", true),
    ("doctype", true),
    ("element", true),
    ("script_element", true),
    ("start_tag", true),
    ("style_element", true),
    ("tag_name", true),
    ("script_start_tag", true),
    ("style_start_tag", true),
    ("self_closing_tag", true),
    ("end_tag", true),
    ("erroneous_end_tag", true),
    ("attribute", true),
    ("attribute_name", true),
    ("attribute_value", true),
    ("entity", true),
    ("quoted_attribute_value", true),
    ("text", true),
    ("template_element", true),
    ("template_start_tag", true),
    ("interpolation", true),
    ("raw_text", true),
    ("directive_attribute", true),
    ("directive_name", true),
    ("directive_value", true),
    ("dynamic_directive_value", true),
    ("dynamic_directive_inner_value", true),
    ("directive_modifiers", true),
    ("directive_modifier", true),
    ("erroneous_end_tag_name", true),
    ("comment", true),
    ("<!", false),
    ("doctype", false),
    (">", false),
    ("<", false),
    ("/>", false),
    ("</", false),
    ("=", false),
    ("'", false),
    ("\"", false),
    ("{{", false),
    ("}}", false),
    (":", false),
    (".", false),
    ("@", false),
    ("#", false),
    ("[", false),
    ("]", false),
];

/// The last kind.
#[allow(dead_code)] // the tests' check of the table
pub const LAST: K = K::TRBracket;

/// Fields, as the tape names them; 0 is no field.
pub const FIELDS: &[&str] = &[
    "",
];

/// Field ids, in `FIELDS`' order.
pub mod f {
    #[allow(dead_code)] // no field
    pub const NONE: u8 = 0;
}

/// Per kind: its automaton, plus one (0: none; its tokens are all stored).
pub static AUTOMATON: &[u16] = &[
    0, 4, 0, 0, 12, 0, 0, 10, 13, 11, 6, 7, 1, 0, 0, 0, 9, 0, 0, 14, 8, 0, 2, 0, 0, 5, 0, 3, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// Per automaton: its first state.
pub static START: &[u32] = &[
    0, 4, 13, 16, 20, 24, 28, 32, 36, 43, 48, 53, 58, 63,
];

/// Per state: where its moves start in `MOVES`, and (the next one's) where they end.
pub static MOVES_AT: &[u32] = &[
    0, 1, 2, 4, 4, 9, 12, 14, 15, 17, 19, 25, 25, 27, 28, 29,
    30, 31, 32, 33, 33, 34, 36, 37, 37, 38, 39, 40, 40, 41, 42, 43,
    43, 44, 46, 47, 47, 49, 51, 53, 54, 54, 55, 55, 56, 57, 60, 63,
    63, 64, 65, 68, 71, 71, 72, 73, 76, 79, 79, 80, 81, 84, 87, 87,
    88, 89, 92, 95, 95,
];

/// Moves, by state, sorted by label: (label, the state moved to).
pub static MOVES: &[(u32, u32)] = &[
    (13, 1), (37, 2), (14, 3), (16, 3), (23, 5), (42, 6), (43, 6), (44, 6),
    (45, 6), (27, 7), (37, 8), (42, 9), (24, 10), (25, 10), (37, 8), (14, 11),
    (16, 11), (24, 12), (25, 12), (27, 7), (37, 8), (42, 6), (43, 6), (44, 6),
    (45, 6), (27, 7), (37, 8), (43, 14), (28, 15), (43, 14), (31, 17), (32, 18),
    (33, 19), (46, 21), (26, 22), (47, 23), (47, 23), (36, 25), (6, 26), (33, 27),
    (36, 29), (29, 30), (33, 31), (40, 33), (21, 34), (41, 35), (41, 35), (38, 37),
    (39, 38), (14, 39), (38, 40), (14, 41), (39, 42), (38, 40), (39, 42), (34, 44),
    (6, 45), (12, 46), (22, 46), (33, 47), (12, 46), (22, 46), (33, 47), (34, 49),
    (6, 50), (12, 51), (22, 51), (35, 52), (12, 51), (22, 51), (35, 52), (34, 54),
    (6, 55), (12, 56), (22, 56), (33, 57), (12, 56), (22, 56), (33, 57), (34, 59),
    (6, 60), (12, 61), (22, 61), (33, 62), (12, 61), (22, 61), (33, 62), (34, 64),
    (6, 65), (12, 66), (22, 66), (33, 67), (12, 66), (22, 66), (33, 67),
];

/// Per state: whether it accepts, a bit each.
pub static ACCEPTS: &[u64] = &[
    4760451077827501226, 8,
];

/// Per state: where its implied words start in `IMPLIED`, and (the next one's) where they end.
pub static IMPLIED_AT: &[u32] = &[
    0, 0, 2, 2, 2, 2, 4, 4, 6, 6, 6, 8, 8, 10, 11, 11,
    12, 12, 12, 12, 12, 13, 13, 13, 13, 14, 14, 14, 14, 15, 15, 15,
    15, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 17, 17, 17, 17,
    17, 18, 18, 18, 18, 18, 19, 19, 19, 19, 19, 20, 20, 20, 20, 20,
    21, 21, 21, 21, 21,
];

/// The implied words from a gap's first state to a named child, by state, sorted by the child's label: (label, the word packed: `WORD` start << 12 | length << 8 | the tokens left out).
pub static IMPLIED: &[(u32, u32)] = &[
    (14, 257), (16, 4353), (14, 8449), (16, 12545), (14, 16641), (16, 20737), (14, 24833), (16, 28929),
    (14, 33025), (16, 37121), (28, 41217), (28, 45313), (26, 73985), (6, 94465), (29, 102657), (21, 110849),
    (6, 147713), (6, 160001), (6, 172289), (6, 184577), (6, 196865),
];

/// Per state: the implied word from it to the end, packed as in `IMPLIED` (0: none that leaves a token out).
pub static IMPLIED_END: &[u32] = &[
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    49927, 61955, 69889, 0, 78339, 86273, 90369, 0, 0, 0, 98561, 0, 0, 0, 106753, 0,
    115203, 123137, 127233, 0, 0, 131329, 135425, 139521, 0, 143617, 0, 0, 0, 151809, 155905, 0,
    0, 0, 164097, 168193, 0, 0, 0, 176385, 180481, 0, 0, 0, 188673, 192769, 0, 0,
    0, 200961, 205057, 0,
];

/// The implied words' labels.
pub static WORD: &[u32] = &[
    37, 37, 37, 37, 37, 37, 37, 37, 37, 37, 43, 43, 31, 32, 33, 32,
    33, 33, 46, 46, 47, 47, 47, 36, 33, 36, 33, 40, 40, 41, 41, 41,
    38, 39, 38, 39, 34, 33, 33, 34, 35, 35, 34, 33, 33, 34, 33, 33,
    34, 33, 33,
];
