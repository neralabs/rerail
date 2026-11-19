//! Each stored tape's presence (decision 0434 step 6, part 3): the kinds and fields its nodes have, the document's and
//! each block's, as the format 16 writer made them. A derived fact, written once where it is computed (0353): a kept
//! fact of one space of its own, one entry a tape (keyed by the tape's write-back key), its rows the presence's words
//! in their order (one column, kept as given). A run reads it in place, by position (`Packed::get`), from the chain's
//! mapped files: nothing is decoded and no copy of it is made.
//!
//! The words are written behind the run with the tape: a wave's presences are gathered and written as one file of the
//! chain (`write_space`), not a file a tape. Until then a read of a queued tape's presence gets the writer's own
//! (`queued_presence`).

use super::{KeptPut, KeptWhat, KeptWrite};
use crate::content_store::ContentStore;
use crate::content_store::write_back::{Closed, Gather, Held, Key, Kind, Syms, Work};
use std::any::Any;
use std::collections::HashMap;

/// The relation the presences are kept as.
pub const PRESENCE: &str = "tape_presence";

/// The space the presences are kept in: one for the store, a tape's entry keyed by the tape's write-back key (its
/// source's digest, language, grammar and format versions).
pub fn presence_space() -> [u8; 32] {
    Key::of(&[b"tape presence"]).0
}

/// A wave's presences, written as one kept file when the wave is sealed.
struct PresenceGather {
    puts: Vec<KeptPut>,
    held: HashMap<Key, Held>,
    bytes: usize,
}

impl Gather for PresenceGather {
    fn keys(&self) -> Vec<(Key, Option<Held>)> {
        self.held.iter().map(|(k, h)| (*k, Some(h.clone()))).collect()
    }
    fn held(&self, key: &Key) -> Option<Held> {
        self.held.get(key).cloned()
    }
    fn bytes(&self) -> usize {
        self.bytes
    }
    fn close(self: Box<Self>) -> Option<Closed> {
        let PresenceGather { puts, held, bytes } = *self;
        if puts.is_empty() {
            return None;
        }
        let keys = held.into_iter().map(|(k, h)| (k, Some(h))).collect();
        Some(Closed { what: "tape presences", bytes, keys, syms: Syms::None, job: Work(Kind::Kept(Box::new(KeptWrite { space: presence_space(), puts }))) })
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// The write-back key a tape's presence is queued under: apart from the tape's own (`tape`), whose gather holds the
/// tape under it.
fn queued_key(tape: &Key) -> Key {
    Key::of(&[b"tape presence", &tape.0])
}

/// A presence's words as a put of the entry of the tape queued under `key`.
fn put(key: &Key, words: Vec<i64>) -> KeptPut {
    let digest = words.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, w| (h ^ *w as u64).wrapping_mul(0x100_0000_01b3));
    let n = words.len();
    KeptPut { rel: PRESENCE.to_owned(), ident: key.0, part: 0, digest, reads: Vec::new(), types: vec![b'i'], what: KeptWhat::Flat { widths: vec![64], rows: words, n, levels: None, given: true } }
}

/// A kept presence's rows: one column of words, read by position (`Packed::get`).
pub use rerail_blocks::packed::Packed;

/// The rows of the presence `space` (`presence_space`, mapped) keeps for the tape queued under `tape`, where they lie
/// in its files; `None` when it keeps none.
pub fn find<'s>(space: &'s super::KeptSpace, tape: &Key) -> Option<&'s Packed> {
    Some(space.view().get_kept(&rerail_blocks::kept::FactKey { rel: PRESENCE, ident: &tape.0, part: 0 })?.rows)
}

impl ContentStore {
    /// Keeps the presence of the tape queued under `tape`, behind the run: its `words` (the document's, then each
    /// block's), and `held`, what a read of it gets until it is written (`queued_presence`). Gathered with the wave's
    /// others; outside a run's waves, its own write.
    pub fn queue_presence(&self, tape: &Key, words: Vec<i64>, held: Held) {
        let bytes = words.len() * 8 + 64;
        let key = queued_key(tape);
        let mut item = Some((put(tape, words), held));
        let make = || PresenceGather { puts: Vec::new(), held: HashMap::new(), bytes: 0 };
        let gathered = self.gather("tape presences", make, |g| {
            let (p, h) = item.take().expect("the presence");
            g.puts.push(p);
            g.held.insert(key, h);
            g.bytes += bytes;
        });
        if !gathered && let Some((p, _)) = item {
            self.queue_kept(presence_space(), vec![p]);
        }
    }

    /// The presence this process queued for the tape under `tape`, not written yet: what `queue_presence` was given,
    /// as a `T`.
    pub fn queued_presence<T: Any + Send + Sync>(&self, tape: &Key) -> Option<std::sync::Arc<T>> {
        self.pending::<T>(&queued_key(tape))
    }
}
