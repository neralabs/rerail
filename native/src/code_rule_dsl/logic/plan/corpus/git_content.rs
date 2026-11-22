//! Byte ranges from verified Git objects in the run's ordinary source store.
use super::{git, Late, Result};
use crate::content_store::{ContentStore, HashKey, SourceId};
use std::collections::HashMap;

pub const REQUEST: &str = "git_payload_request";
pub const ANSWER: &str = "git_payload_of";

struct Payload {
    kind: String,
    handle: String,
    size: i64,
    source: SourceId,
    storage: String,
}

pub struct Answer {
    pub kind: String,
    pub handle: String,
    pub size: i64,
    pub bytes: String,
    pub status: &'static str,
    pub storage: String,
}

pub fn source(store: &ContentStore, handle: &str) -> Result<Option<SourceId>> {
    let Some(gix::ObjectId::Sha256(hash)) = handle.strip_prefix("sha256:").and_then(|h| gix::ObjectId::from_hex(h.as_bytes()).ok()) else {
        return Ok(None);
    };
    Ok(store.source_by_hash(HashKey::from_sha256_digest(hash))?)
}

#[derive(Default)]
pub struct Reader(HashMap<(String, String), Payload>);

impl Reader {
    pub fn read(&mut self, git: &git::Git, store: &ContentStore, late: &Late, repo: &str, oid: &str, offset: i64, length: i64) -> Result<Answer> {
        let error = |reason| Answer {
            kind: String::new(), handle: String::new(), size: 0, bytes: String::new(), status: reason, storage: reason.to_owned(),
        };
        if offset < 0 || length < 0 { return Ok(error("invalid-range")); }
        let key = (repo.to_owned(), oid.to_owned());
        let stamp = git.observation(git::OBJECT, repo, oid);
        if !self.0.get(&key).is_some_and(|p| p.storage.starts_with("readable:") && Some(&p.storage) == stamp.as_ref()) {
            let (kind, bytes, storage) = match git.payload(repo, oid) {
                Ok(payload) => payload,
                Err(reason) => return Ok(error(reason)),
            };
            crate::code_rule_dsl::logic::profile::decision("Git payload verified", || oid.to_owned());
            let canonical = HashKey::sha256(&bytes);
            let handle = format!("sha256:{}", crate::content_store::observe::hex(&canonical.sha256_bytes().expect("SHA-256")));
            let source = match store.source_by_hash(canonical)? {
                Some(id) => id,
                None => store.put_source_hashed(&bytes, canonical, &[])?,
            };
            late.lock().expect("run sources").push(source);
            self.0.insert(key.clone(), Payload { kind, handle, size: bytes.len() as i64, source, storage });
        }
        let p = &self.0[&key];
        let bytes = if offset >= p.size || length == 0 {
            String::new()
        } else {
            let data = store.get_source(p.source)?;
            let end = offset.saturating_add(length).min(p.size) as usize;
            git::hex(&data[offset as usize..end])
        };
        Ok(Answer { kind: p.kind.clone(), handle: p.handle.clone(), size: p.size, bytes, status: "ok", storage: p.storage.clone() })
    }
}
