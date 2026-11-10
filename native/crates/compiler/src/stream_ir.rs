//! Tier 2's tables (decision 0087): `SWITCH`'s byte switches and `RETAG`'s readings, which
//! the transducer's words name by index, and the targets and kinds they hold.

/// A target that refuses: the file goes to tree-sitter.
pub const REFUSE: u16 = u16::MAX;

/// `SWITCH`'s table: a target per value of the token's byte at `offset`, and one for the end
/// of the stream. `REFUSE` refuses.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ByteSwitch {
    pub offset: u8,
    pub targets: Vec<u16>,
    pub end: u16,
}

impl ByteSwitch {
    pub fn refusing(offset: u8) -> Self {
        Self { offset, targets: vec![REFUSE; 256], end: REFUSE }
    }
}

/// A record a reading dropped (`GONE`), left out at `ACCEPT`.
pub const DEAD: u16 = u16::MAX;

/// What a reading does with a node of a kind: `KEEP` it with every node inside it, unread; it is
/// `GONE`, the nodes inside taking its place; else the kind it takes, the nodes inside read too.
pub const KEEP: u16 = u16::MAX;
pub const GONE: u16 = u16::MAX - 1;

/// A reading (`reading R = …`, `reread R`): what `RETAG m=3` does with the node made last at a
/// level and each node inside it, by kind; a kind not listed refuses. A node read but the first
/// whose field `fields` names takes the field it gives.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Reading {
    pub kinds: Vec<(u16, u16)>,
    pub fields: Vec<(u16, u16)>,
    /// What the node read itself takes, whatever its kind (`root gone`, `root as M`): `GONE` or
    /// a kind; `None`, its kind's way.
    pub root: Option<u16>,
}

/// A program's readings, and its provisional fields: a node holding one at `ACCEPT` refuses.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Readings {
    pub maps: Vec<Reading>,
    pub provisional: Vec<u16>,
    /// The extras' kinds (`extra K`): a node of one is flagged an extra in the table (comments).
    pub extras: Vec<u16>,
}

impl Readings {
    pub fn is_empty(&self) -> bool {
        self.maps.is_empty() && self.provisional.is_empty() && self.extras.is_empty()
    }
}
