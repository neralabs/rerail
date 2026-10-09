//! Forte's flat nodes (`Parser\NodeKind`, `Internal\FlatNodeRecord`): the tree builder's output,
//! an array of records linked by index, `NONE` (-1) for no node.

/// No node, no token (`TreeBuilder::NONE`).
pub(crate) const NONE: i32 = -1;

/// A node kind, numbered as Forte numbers it (the oracle records the numbers). Forte's
/// `Fragment` (3) is left out: its tree builder never creates one.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum NodeKind {
    Root = 0,
    Element = 1,
    Text = 2,
    Echo = 4,
    RawEcho = 5,
    TripleEcho = 6,
    Directive = 7,
    DirectiveBlock = 8,
    Verbatim = 9,
    PhpBlock = 10,
    PhpTag = 11,
    Comment = 12,
    BogusComment = 13,
    ConditionalComment = 14,
    BladeComment = 15,
    Doctype = 16,
    Cdata = 17,
    Decl = 18,
    Attribute = 19,
    JsxAttribute = 20,
    UnpairedClosingTag = 21,
    NonOutput = 22,
    ElementName = 23,
    ClosingElementName = 24,
    AttributeWhitespace = 25,
    AttributeName = 26,
    AttributeValue = 27,
    ProcessingInstruction = 28,
}

/// A directive's place in a block (`Parser\Directives\StructureRole`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StructureRole {
    Opening,
    Closing,
    Intermediate,
    Mixed,
    None,
}

impl StructureRole {
    /// The case name.
    #[cfg(test)]
    pub(crate) fn name(self) -> &'static str {
        match self {
            StructureRole::Opening => "Opening",
            StructureRole::Closing => "Closing",
            StructureRole::Intermediate => "Intermediate",
            StructureRole::Mixed => "Mixed",
            StructureRole::None => "None",
        }
    }
}

/// The fields the tree builder adds to a directive's or directive block's record beyond the
/// structural ones (`FlatNodeRecord::$extra`): `name` and `args` always together, `role` on
/// directives but not on blocks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DirectiveInfo {
    /// The directive's name as the tree builder resolved it (`$directiveName`).
    pub(crate) name: Box<[u8]>,
    /// The arguments' text (`$argsContent`), `None` for PHP's null.
    pub(crate) args: Option<Box<[u8]>>,
    pub(crate) role: Option<StructureRole>,
}

/// A flat node (`FlatNodeRecord`). Links are node indices and token fields token indices,
/// `NONE` for none, as Forte holds them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FlatNode {
    pub(crate) kind: NodeKind,
    pub(crate) parent: i32,
    pub(crate) first_child: i32,
    pub(crate) last_child: i32,
    pub(crate) next_sibling: i32,
    pub(crate) token_start: i32,
    pub(crate) token_count: i32,
    pub(crate) generic_offset: i32,
    pub(crate) data: i32,
    pub(crate) directive: Option<Box<DirectiveInfo>>,
}

impl FlatNode {
    /// `TreeBuilder::createNode`: a record with no links.
    pub(crate) fn new(kind: NodeKind, parent: i32, token_start: i32, token_count: i32) -> Self {
        Self {
            kind,
            parent,
            first_child: NONE,
            last_child: NONE,
            next_sibling: NONE,
            token_start,
            token_count,
            generic_offset: 0,
            data: 0,
            directive: None,
        }
    }
}
