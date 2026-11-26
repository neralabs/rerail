//! Answerers (P§12 item 7, "DeltaOp and Answerer: the operators"; P§7): an answerer maps a request's key to its
//! answer, a row or a blob. Its kind says what can change the answer between runs. Here are the names P§12 gives the
//! blocks crate; the answerers themselves (load, parse, intern) are the engine's, over its world and its blobs.

use crate::blob::BlobId;

/// What can change an answer between runs (P§7.2).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AnswerKind {
    /// The world, through a leaf the answer read: each leaf recorded, misses too (D3).
    World,
    /// Nothing: the key is the whole input; kept by key for every run and workspace (D4).
    Pure,
    /// The sub-run's own inputs.
    SubRun,
}

/// An answer: a row, a blob, or a miss (a world reader's key with nothing there).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    Row(Vec<i64>),
    Blob(BlobId),
    Miss,
}

/// An answerer: one key at a time. A world reader writes every leaf it read, misses too (D3), into `leaves`, the
/// run's layer of the world's leaves (`Leaves`, the engine's writer of that trace's layer); a pure answerer reads no
/// leaf.
pub trait Answerer {
    const KIND: AnswerKind;
    type Leaves;

    fn answer(&mut self, key: &[i64], leaves: &mut Self::Leaves) -> Result<Answer, String>;
}
