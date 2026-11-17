//! A run's small writes, one record (decision 0306).
//!
//! A small write (the files' stats, the manifest, a pin, a kept plan) is an **item** of the run's record: a kind, its
//! body (owned data) and the keys a `queued` or `settle` asks about. Items wait beside the run's waves until the run
//! ends (or a `settle` of one of their keys), then the record is encoded as **one string**, with **one byte area** for
//! binary bodies, and queued as the last job of the run's last wave. The writer decodes it and applies its items in
//! order, each by the function its kind registered: the queue holds data, the registry code.
//!
//! The encoding is [`codec`]'s alone, so changing it (JSON now) changes that module.

use super::write_back::{Closed, Gather, Held, Key, Syms, Work};
use super::{ContentStore, SourceId, StoreError, StoreResult};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

/// How a record is written and read: one string, with a version of its own.
pub mod codec {
    use super::{StoreError, StoreResult};
    use serde::{Deserialize, Serialize, de::DeserializeOwned};

    /// The record's format: a record of another is not applied (the next run computes what it held again).
    pub const VERSION: u32 = 1;

    /// An item's body, as the codec holds it before the record is written.
    pub type Body = serde_json::Value;

    /// One item: its kind, its body, and the range of the record's byte area it reads, if any.
    #[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
    pub struct Item {
        pub kind: String,
        pub body: Body,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub bytes: Option<(usize, usize)>,
    }

    #[derive(Serialize, Deserialize)]
    struct Written<'a> {
        version: u32,
        items: std::borrow::Cow<'a, [Item]>,
    }

    pub fn body<T: Serialize>(value: &T) -> Body {
        serde_json::to_value(value).expect("a record's body is data")
    }

    pub fn from_body<T: DeserializeOwned>(kind: &str, body: Body) -> StoreResult<T> {
        serde_json::from_value(body).map_err(|e| StoreError::Invalid(format!("a record's {kind}: {e}")))
    }

    pub fn encode(items: &[Item]) -> String {
        serde_json::to_string(&Written { version: VERSION, items: items.into() }).expect("a record is data")
    }

    /// The items of `text`; `None` for a record of another format.
    pub fn decode(text: &str) -> StoreResult<Option<Vec<Item>>> {
        #[derive(Deserialize)]
        struct Head {
            version: u32,
        }
        let head: Head = serde_json::from_str(text).map_err(|e| StoreError::Invalid(format!("a record: {e}")))?;
        if head.version != VERSION {
            return Ok(None);
        }
        let written: Written<'static> = serde_json::from_str(text).map_err(|e| StoreError::Invalid(format!("a record: {e}")))?;
        Ok(Some(written.items.into_owned()))
    }
}

/// What an item's apply is given: the store, its bytes (the item's range of the record's byte area) and the run's
/// named sources, which the manifest's item names.
pub struct Apply<'a> {
    pub store: &'a ContentStore,
    pub bytes: &'a [u8],
}

impl Apply<'_> {
    /// Adds `ids` to the sources the run's manifest names.
    pub fn name(&self, ids: impl IntoIterator<Item = SourceId>) {
        self.store.name_sources(ids);
    }
}

/// A kind's apply: its body, decoded by the kind (`codec::from_body`).
pub type ApplyFn = fn(&Apply<'_>, codec::Body) -> StoreResult<()>;

static KINDS: LazyLock<Mutex<HashMap<String, ApplyFn>>> = LazyLock::new(Default::default);

/// Registers `kind`'s apply (again: the same function). A kind is one function: a second under its name would apply
/// the first's items with the wrong body.
pub fn register(kind: &'static str, apply: ApplyFn) {
    let mut kinds = KINDS.lock().expect("the record's kinds");
    if let Some(had) = kinds.insert(kind.to_owned(), apply) {
        assert!(std::ptr::fn_addr_eq(had, apply), "two record kinds named {kind:?}: give each its own name");
    }
}

/// Applies `text` (and `bytes`, its byte area) in order. A record of another format is not applied; an item of a kind
/// not registered fails the record.
pub fn apply(store: &ContentStore, text: &str, bytes: &[u8]) -> StoreResult<()> {
    let Some(items) = codec::decode(text)? else { return Ok(()) };
    for item in items {
        let f = *KINDS.lock().expect("the record's kinds").get(&item.kind).ok_or_else(|| StoreError::Invalid(format!("a record's item of kind {}: none registered", item.kind)))?;
        let range = item.bytes.map_or(&[][..], |(at, len)| &bytes[at..at + len]);
        // Judged by its kind in a warm run, as a job of its own was (`profile::WARM_WRITES_ALLOWED`).
        super::hazards::warm_job(&item.kind);
        f(&Apply { store, bytes: range }, item.body)?;
    }
    Ok(())
}

/// What a run's record is queued as: no write of its own, judged in a warm run by its items' kinds, never by this name.
pub(super) const WHAT: &str = "the run's record";

/// A run's record while it is gathered (`write_back::Gather`): closed once, at the run's end.
#[derive(Default)]
pub(super) struct Record {
    items: Vec<codec::Item>,
    bytes: Vec<u8>,
    keys: Vec<Key>,
}

impl Record {
    fn push(&mut self, kind: &'static str, keys: &[Key], body: codec::Body, bytes: Option<Vec<u8>>) {
        let range = bytes.map(|b| {
            let at = self.bytes.len();
            self.bytes.extend_from_slice(&b);
            (at, b.len())
        });
        self.items.push(codec::Item { kind: kind.to_owned(), body, bytes: range });
        self.keys.extend_from_slice(keys);
    }

    /// Its weight in the queue: the bodies' and the byte area's.
    fn weight(&self) -> usize {
        self.bytes.len() + self.items.len() * 256
    }
}

impl Gather for Record {
    fn keys(&self) -> Vec<(Key, Option<Held>)> {
        self.keys.iter().map(|k| (*k, None)).collect()
    }
    fn held(&self, _key: &Key) -> Option<Held> {
        None
    }
    fn holds(&self, key: &Key) -> bool {
        self.keys.contains(key)
    }
    fn until_end(&self) -> bool {
        true
    }
    fn bytes(&self) -> usize {
        self.weight()
    }
    fn close(self: Box<Self>) -> Option<Closed> {
        if self.items.is_empty() {
            return None;
        }
        let bytes = self.weight();
        Some(Closed { what: WHAT, bytes, keys: Vec::new(), syms: Syms::None, job: Work::record(codec::encode(&self.items), self.bytes) })
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl ContentStore {
    /// Adds an item of `kind` (its apply `f`, registered) to this thread's run's record, under `keys`; outside a run,
    /// a record of its own, queued now.
    pub fn record<T: serde::Serialize>(&self, kind: &'static str, keys: Vec<Key>, body: &T, bytes: Option<Vec<u8>>, f: ApplyFn) {
        register(kind, f);
        let body = codec::body(body);
        let mut once = Some((body, bytes));
        // Asked of a warm run by the item's kind, as a job of its own is.
        let gathered = self.gather(kind, Record::default, |r| {
            let (body, bytes) = once.take().expect("added once");
            r.push(kind, &keys, body, bytes);
        });
        if gathered {
            return;
        }
        let (body, bytes) = once.take().expect("not gathered");
        self.queue_item(Syms::None, kind, keys, body, bytes, false);
    }

    /// Queues an item of `kind` (its apply `f`, registered) as a record of its own, never gathered: for a write that
    /// is not the run's record's (dropped where the queue is full when `droppable`, as a cache's; or holding symbols
    /// mapped as it is written, `syms`). Whether it was queued.
    #[allow(clippy::too_many_arguments)]
    pub fn queue_record<T: serde::Serialize>(&self, syms: Syms, kind: &'static str, keys: Vec<Key>, body: &T, bytes: Option<Vec<u8>>, f: ApplyFn, droppable: bool) -> bool {
        register(kind, f);
        self.queue_item(syms, kind, keys, codec::body(body), bytes, droppable)
    }

    fn queue_item(&self, syms: Syms, kind: &'static str, keys: Vec<Key>, body: codec::Body, bytes: Option<Vec<u8>>, droppable: bool) -> bool {
        let mut r = Record::default();
        r.push(kind, &[], body, bytes);
        let weight = r.weight();
        let work = Work::record(codec::encode(&r.items), r.bytes);
        let keys = keys.into_iter().map(|k| (k, None)).collect();
        if droppable { self.write_back_unless_full(syms, kind, weight, keys, work) } else { self.write_back_syms(syms, kind, weight, keys, work); true }
    }
}

/// The store's own kinds: a release of pins, queued under the name a warm run judges it by
/// (`profile::WARM_WRITES_ALLOWED`).
impl ContentStore {
    /// Releases `owner`'s pins and holds, in the run's record as `what`.
    pub fn record_release(&self, what: &'static str, owner: String) {
        fn apply(cx: &Apply<'_>, body: codec::Body) -> StoreResult<()> {
            let owner: String = codec::from_body("a release", body)?;
            cx.store.release_pins(&owner).map(drop)
        }
        self.record(what, Vec::new(), &owner, None, apply);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_reads_back_as_written_and_another_format_is_not_read() {
        let items = vec![
            codec::Item { kind: "a".into(), body: codec::body(&(1, "x")), bytes: None },
            codec::Item { kind: "b".into(), body: codec::body(&[1u8; 3]), bytes: Some((4, 2)) },
        ];
        let text = codec::encode(&items);
        assert_eq!(codec::decode(&text).unwrap(), Some(items));
        let other = text.replacen(&format!("\"version\":{}", codec::VERSION), "\"version\":999", 1);
        assert_eq!(codec::decode(&other).unwrap(), None);
    }
    static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

    fn logged(cx: &Apply<'_>, body: codec::Body) -> StoreResult<()> {
        let said: String = codec::from_body("a test's item", body)?;
        if said == "names" {
            cx.name([SourceId::from_raw(5), SourceId::from_raw(6)]);
        }
        let said = if said == "takes" { format!("takes {:?}", cx.store.take_named_sources()) } else { said };
        LOG.lock().unwrap().push(format!("{said} ({} bytes)", cx.bytes.len()));
        Ok(())
    }

    /// A run's items are applied in their order after its other writes, as one record at its end; the sources an
    /// item names reach a later one (the manifest's); a `settle` of an item's key applies the record at once.
    #[test]
    fn a_runs_record_is_applied_in_order_at_its_end_or_when_a_key_is_settled() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::open(&dir.path().join("store.sqlite")).unwrap();
        {
            let _waves = super::super::write_back::run_waves();
            store.record("test/log", vec![], &"names", None, logged);
            store.record("test/log", vec![], &"bytes", Some(vec![1, 2, 3]), logged);
            store.write_back("a wave's job", 0, vec![], Work::test(Box::new(|_| {
                LOG.lock().unwrap().push("a wave's job".into());
                Ok(())
            })));
            super::super::write_back::end_wave(super::super::write_back::Boundary::Chunk);
            store.record("test/log", vec![], &"takes", None, logged);
            store.written().unwrap();
            assert_eq!(*LOG.lock().unwrap(), ["a wave's job", "names (0 bytes)", "bytes (3 bytes)", "takes [SourceId(5), SourceId(6)] (0 bytes)"]);
        }
        LOG.lock().unwrap().clear();
        {
            let _waves = super::super::write_back::run_waves();
            let key = Key::of(&[b"a test's key"]);
            store.record("test/log", vec![key], &"settled", None, logged);
            assert!(store.queued(&key));
            assert!(LOG.lock().unwrap().is_empty(), "held until the run's end");
            store.settle(&key);
            assert_eq!(*LOG.lock().unwrap(), ["settled (0 bytes)"]);
            assert!(!store.queued(&key));
        }
    }
}
