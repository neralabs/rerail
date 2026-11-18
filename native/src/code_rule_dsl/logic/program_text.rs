//! How a run keeps what it compiled (a prepared program, a plan choice, a combined program, a pipeline's steps and
//! runs) in the content store: each program as its image (decision 0173), a source by its bytes' digest, and the
//! facts that name them.
//!
//! A program was once kept as its `.rr` text too (decisions 0155, 0162), read back into a tree. The tree is gone
//! (decision 0276 step 6), and so is that form: a program kept as text alone, by a store written before images, is
//! not read, and the run that wanted it compiles it again.

use super::front::Value;
use super::program_facts::{self, Facts};
use crate::content_store::{ContentStore, HashKey, SourceId, StoreResult};
use sha2::{Digest as _, Sha256};
use crate::code_rule_dsl::logic::program_image::manager::VersionAnalyses as _;

fn bad(what: impl std::fmt::Display) -> String {
    format!("program text: {what}")
}

/// The alias the store keeps the image of the program whose text has sha256 `digest` under: the program's identity
/// stays its text's digest, which the rows that name a program hold, and the image's source is its own bytes' sha256,
/// as every source's is. The format is in the key, so another format's image is never found for it.
pub fn image_key(digest: &[u8; 32]) -> HashKey {
    let format = crate::code_rule_dsl::logic::program_image::FORMAT;
    HashKey::sha256(&[format!("rerail program image {format}\0").as_bytes(), digest.as_slice()].concat())
}

/// A program as the store keeps it, made where the program is (the run's thread, decision 0306), kept by the store's
/// writer (`store_encoded`): its image (decision 0173). The digest is of its bytes, which is the source's own hash.
pub struct Encoded {
    pub digest: [u8; 32],
    /// Whether the bytes are an image: always, for what is made now. A record written before (`Packed`) can name a
    /// text, which is kept as the source it was.
    pub image: bool,
    pub bytes: Vec<u8>,
}

/// A program's version as the store keeps it: a base's bytes as they are (a prepared program's version,
/// `Version::base_bytes`, decision 0281), any other version folded into one first (Typing-support's `store_version`,
/// taken at the merge of e829e72a7); `None` where the fold fails.
pub fn encode_version(v: &crate::code_rule_dsl::logic::program_image::Version) -> Option<Encoded> {
    let folded;
    let bytes = match v.base_bytes() {
        Some(bytes) => bytes,
        None => {
            folded = v.fold().ok()?;
            folded.base_bytes()?
        }
    };
    crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, bytes.len() as u64);
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    // A run building the engine's bundle (`RERAIL_EXPORT_IMAGES`, decision 0264) ships it.
    crate::code_rule_dsl::logic::rr::images::export(image_key(&digest), bytes);
    Some(Encoded { digest, image: true, bytes: bytes.to_vec() })
}

/// Programs `encode_version` made, and a facts entry after them, as a record's item carries them (decision 0306): their
/// digests and kinds in its body, their bytes, then the entry's, in its byte area.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Packed {
    programs: Vec<([u8; 32], bool, usize)>,
}

/// Packs `programs` and `entry` (`Packed`): the body and the byte area.
pub fn pack(programs: Vec<Encoded>, entry: Vec<u8>) -> (Packed, Vec<u8>) {
    let mut bytes = Vec::with_capacity(programs.iter().map(|p| p.bytes.len()).sum::<usize>() + entry.len());
    let mut packed = Packed { programs: Vec::with_capacity(programs.len()) };
    for p in programs {
        packed.programs.push((p.digest, p.image, p.bytes.len()));
        bytes.extend_from_slice(&p.bytes);
    }
    bytes.extend_from_slice(&entry);
    (packed, bytes)
}

/// Keeps the programs `pack` packed (`store_encoded`): the sources written, and the entry's bytes.
pub fn store_packed<'b>(store: &ContentStore, packed: &Packed, bytes: &'b [u8]) -> StoreResult<(Vec<SourceId>, &'b [u8])> {
    let (mut at, mut sources) = (0, Vec::new());
    for (digest, image, len) in &packed.programs {
        let program = bytes.get(at..at + len).ok_or_else(|| crate::content_store::StoreError::Invalid("a record's program: past its bytes".into()))?;
        sources.extend(store_encoded(store, digest, *image, program)?);
        at += len;
    }
    Ok((sources, &bytes[at..]))
}

/// Image `bytes` kept in `store` under the digest of its bytes (decision 0246 §6), on the store's writer (an item's apply,
/// decision 0306): the digest, and the source, which the item names so collection keeps it. No pin (0306 step 4).
fn store_image(store: &ContentStore, bytes: &[u8]) -> StoreResult<([u8; 32], Vec<SourceId>)> {
    crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, bytes.len() as u64);
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    let ids = store_encoded(store, &digest, true, bytes)?;
    Ok((digest, ids))
}

/// Keeps a program `encode` made in `store`, a source by its bytes' sha256 (an image by `image_key` too): the sources
/// written, which the caller names so collection keeps them.
pub fn store_encoded(store: &ContentStore, digest: &[u8; 32], image: bool, bytes: &[u8]) -> StoreResult<Vec<SourceId>> {
    if image {
        let key = image_key(digest);
        let id = match store.source_by_alias(key)? {
            Some(id) => id,
            None => store.put_source(bytes, &[key])?,
        };
        return Ok(vec![id]);
    }
    let hash = HashKey::sha256(bytes);
    let source = match store.source_by_hash(hash)? {
        Some(id) => id,
        None => store.put_source_hashed(bytes, hash, &[])?,
    };
    Ok(vec![source])
}

/// The images `store` keeps of the programs of sha256 `digests` (`store`), by digest, with their sources: for a caller
/// that must read before its run's waves (decision 0104) and open them later. One the store has lost, or kept as text
/// alone, is left out.
pub fn stored(store: &ContentStore, digests: &[[u8; 32]]) -> StoreResult<std::collections::HashMap<[u8; 32], (Vec<u8>, Vec<SourceId>)>> {
    let hashes: Vec<HashKey> = digests.iter().map(|d| HashKey::from_sha256_digest(*d)).collect();
    let text_ids = store.sources_by_hashes(&hashes)?;
    let mut out = std::collections::HashMap::new();
    for (k, d) in digests.iter().enumerate() {
        match store.source_by_alias(image_key(d))? {
            Some(id) => {
                let bytes = store.get_source(id)?;
                // The image's digest is its bytes' (`store`), so it is its own source too: named once.
                out.insert(*d, (bytes, text_ids[k].into_iter().filter(|t| *t != id).chain([id]).collect()));
            }
            // A program the engine ships (decision 0264): its image from the bundle, kept by no source of the store.
            None if let Some(bytes) = crate::code_rule_dsl::logic::rr::images::shipped().and_then(|b| b.get(image_key(d))) => {
                out.insert(*d, (bytes.to_vec(), text_ids[k].into_iter().collect()));
            }
            None => {}
        }
    }
    Ok(out)
}

/// The sources `stored` names for `digests` (each program's image, and a text a store written before images kept),
/// with none read: a run that holds the programs in memory names what it would have read (the manifest's), so
/// collection keeps them while it keeps the run.
pub fn sources(store: &ContentStore, digests: &[[u8; 32]]) -> StoreResult<Vec<SourceId>> {
    let hashes: Vec<HashKey> = digests.iter().map(|d| HashKey::from_sha256_digest(*d)).collect();
    let mut out: Vec<SourceId> = store.sources_by_hashes(&hashes)?.into_iter().flatten().collect();
    for d in digests {
        if let Some(id) = store.source_by_alias(image_key(d))? {
            out.push(id);
        }
    }
    Ok(out)
}

/// The version of the program `store` keeps of sha256 `digest`, opened from its image, and the sources it is kept as;
/// `None` when the store has lost it, keeps it as text alone, or its image does not open.
pub fn load_image(store: &ContentStore, digest: &[u8; 32]) -> StoreResult<Option<(std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>, Vec<SourceId>)>> {
    let mut found = stored(store, &[*digest])?;
    let Some((bytes, sources)) = found.remove(digest) else { return Ok(None) };
    Ok(crate::code_rule_dsl::logic::program_image::Version::open(&bytes).ok().map(|v| (v, sources)))
}

/// A keyed pipeline's steps (decision 0281 §6), kept in `store` as facts beside the program images they name. A
/// step is two relations in 0103's form, its key and the step it names: `pipeline_step_key(key, pass, version)` (the
/// pass's version is the tag) and `pipeline_step(key, image, size, digest)`, with its decisions
/// (`pipeline_step_decision(key, k, pass, what, detail)`, 0269). They are one source under its key's alias, and the
/// output is an image as a prepared program's is (`store_image`), so a step whose output another step or program
/// stored already shares its image. A run's collection lease keeps what it reads and writes (decision 0306 step 4), and
/// a step is written by the store's writer, an item of the run's record (0306 step 2).
pub struct StoredSteps<'s>(pub &'s ContentStore);

/// The alias a step's facts are kept under: its key, with the image format, so another format's step is not found.
fn step_alias(key: &[u8; 32]) -> HashKey {
    let format = crate::code_rule_dsl::logic::program_image::FORMAT;
    HashKey::sha256(&[format!("rerail pipeline step {format}\0").as_bytes(), key.as_slice()].concat())
}

fn hex32(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex32(text: &str) -> Option<[u8; 32]> {
    let mut out = [0u8; 32];
    if text.len() != 64 {
        return None;
    }
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(text.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

impl crate::code_rule_dsl::logic::program_image::manager::Steps for StoredSteps<'_> {
    fn find(&self, key: &[u8; 32]) -> Result<Option<crate::code_rule_dsl::logic::program_image::manager::StepRecord>, String> {
        use crate::code_rule_dsl::logic::program_image::manager::{Decided, StepRecord};
        let e = |e: crate::content_store::StoreError| e.to_string();
        let Some(id) = self.0.source_by_alias(step_alias(key)).map_err(e)? else { return Ok(None) };
        let facts = program_facts::decode(&self.0.get_source(id).map_err(e)?)?;
        let k = hex32(key);
        let rows = |rel: &str| facts.get(rel).into_iter().flatten().filter(|r| matches!(r.first(), Some(Value::Str(s)) if **s == *k));
        let int = |v: &Value| match v {
            Value::Int(n) => Ok(*n),
            _ => Err(bad("a step's number is not one")),
        };
        let text = |v: &Value| match v {
            Value::Str(s) => Ok(s.to_string()),
            _ => Err(bad("a step's text is not one")),
        };
        let Some(head) = rows("pipeline_step_key").next() else { return Ok(None) };
        let Some(out) = rows("pipeline_step").next() else { return Ok(None) };
        let (pass, version) = (text(&head[1])?, int(&head[2])? as u32);
        let image = unhex32(&text(&out[1])?).ok_or_else(|| bad("a step's image is not a digest"))?;
        // A step whose image the store lost is not found: the step runs again.
        if self.0.source_by_alias(image_key(&image)).map_err(e)?.is_none() {
            return Ok(None);
        }
        let mut decided: Vec<(i64, Decided)> = Vec::new();
        for r in rows("pipeline_step_decision") {
            decided.push((int(&r[1])?, Decided { pass: text(&r[2])?.into(), what: text(&r[3])?.into(), detail: text(&r[4])? }));
        }
        decided.sort_by_key(|(k, _)| *k);
        Ok(Some(StepRecord { pass, version, image, output: (int(&out[2])? as usize, int(&out[3])? as u64), decided: decided.into_iter().map(|(_, d)| d).collect() }))
    }

    fn open(&self, image: &[u8; 32]) -> Result<Option<std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>>, String> {
        let e = |e: crate::content_store::StoreError| e.to_string();
        let Some(id) = self.0.source_by_alias(image_key(image)).map_err(e)? else { return Ok(None) };
        crate::code_rule_dsl::logic::program_image::Version::open(&self.0.get_source(id).map_err(e)?).map(Some)
    }

    fn keep(&self, key: &[u8; 32], record: &crate::code_rule_dsl::logic::program_image::manager::StepRecord, output: &crate::code_rule_dsl::logic::program_image::Version) -> Result<(), String> {
        let folded;
        let bytes = match output.base_bytes() {
            Some(b) => b,
            None => {
                folded = output.fold()?;
                folded.base_bytes().ok_or("a folded version is a base")?
            }
        };
        crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, bytes.len() as u64);
        let image: [u8; 32] = Sha256::digest(bytes).into();
        // A run building the engine's bundle (`RERAIL_EXPORT_IMAGES`, decision 0264) ships it.
        crate::code_rule_dsl::logic::rr::images::export(image_key(&image), bytes);
        let k = Value::Str(hex32(key).into());
        let mut facts = Facts::new();
        facts.entry("pipeline_step_key".into()).or_default().push([k.clone(), str_value(&record.pass), Value::Int(i64::from(record.version))].into());
        facts.entry("pipeline_step".into()).or_default().push([k.clone(), Value::Str(hex32(&image).into()), Value::Int(record.output.0 as i64), Value::Int(record.output.1 as i64)].into());
        for (i, d) in record.decided.iter().enumerate() {
            facts.entry("pipeline_step_decision".into()).or_default().push([k.clone(), Value::Int(i as i64), str_value(&d.pass), str_value(&d.what), str_value(&d.detail)].into());
        }
        // An item of the run's record (decision 0306): the image's bytes, then the step's facts, in its byte area;
        // each stored by the writer unless the store holds it already (an image another step or program stored).
        fn apply(cx: &crate::content_store::record::Apply<'_>, body: crate::content_store::record::codec::Body) -> StoreResult<()> {
            let (key, image, at): ([u8; 32], [u8; 32], usize) = crate::content_store::record::codec::from_body(STEP, body)?;
            let (image_bytes, facts) = cx.bytes.split_at_checked(at).ok_or_else(|| crate::content_store::StoreError::Invalid("a step's record: past its bytes".into()))?;
            if cx.store.source_by_alias(image_key(&image))?.is_none() {
                cx.store.put_source(image_bytes, &[image_key(&image)])?;
            }
            let alias = step_alias(&key);
            if cx.store.source_by_alias(alias)?.is_none() {
                cx.store.put_source(facts, &[alias])?;
            }
            Ok(())
        }
        let mut area = bytes.to_vec();
        area.extend_from_slice(&program_facts::encode(&facts));
        self.0.record(STEP, Vec::new(), &(*key, image, bytes.len()), Some(area), apply);
        Ok(())
    }
}

/// A pipeline step's item of the run's record (`StoredSteps::keep`).
const STEP: &str = "a pipeline step";

fn str_value(s: &str) -> Value {
    Value::Str(s.to_owned().into())
}

/// A pipeline's last run (decision 0333 §2), kept in `store` as facts beside the images of its versions, under its
/// key (`Runs::keep`): `pipeline_run(input, output)`, `pipeline_run_version(i, kind, parent, image, removed)` (a base
/// or an overlay's delta by its image, a view by the rules it leaves out), `pipeline_run_step(j, pass, input, output,
/// form, select kind, select ids, asked, own end)`, `pipeline_run_decision(j, pass, what, detail)`, and each step
/// input's analyses, `pipeline_run_analysis(version, name, tag, …row)`, read with the run and handed to its version.
/// 0103's two rules: the key names the run, and the run its versions. A run whose image the store lost is not found.
pub struct StoredRuns<'s>(pub &'s ContentStore);

fn run_alias(key: &[u8; 32]) -> HashKey {
    let format = crate::code_rule_dsl::logic::program_image::FORMAT;
    HashKey::sha256(&[format!("rerail pipeline run {format}\0").as_bytes(), key.as_slice()].concat())
}

fn ids_text(ids: &[u32]) -> Value {
    str_value(&ids.iter().map(u32::to_string).collect::<Vec<_>>().join(","))
}

fn text_ids(v: &Value) -> Result<Vec<u32>, String> {
    let Value::Str(s) = v else { return Err(bad("a stored run's ids are not text")) };
    if s.is_empty() {
        return Ok(Vec::new());
    }
    s.split(',').map(|x| x.parse::<u32>().map_err(|e| bad(e))).collect()
}

fn kept_run(run: &crate::code_rule_dsl::logic::program_image::manager::Run) -> (Vec<Vec<u8>>, Vec<u8>) {
    use crate::code_rule_dsl::logic::program_image::manager::StoredVersion;
    let mut images: Vec<Vec<u8>> = Vec::new();
    let mut image = |bytes: &[u8]| -> [u8; 32] {
        images.push(bytes.to_vec());
        Sha256::digest(bytes).into()
    };
    let parts = run.parts();
    let mut facts = Facts::new();
    let int = |n: i64| Value::Int(n);
    facts.entry("pipeline_run".into()).or_default().push([int(parts.input as i64), int(parts.output as i64)].into());
    for (i, v) in parts.versions.iter().enumerate() {
        let row: [Value; 5] = match v {
            StoredVersion::Base(bytes) => {
                let image = image(bytes);
                [int(i as i64), int(0), int(-1), str_value(&hex32(&image)), str_value("")]
            }
            StoredVersion::Overlay { parent, delta } => {
                let image = image(delta);
                [int(i as i64), int(1), int(*parent as i64), str_value(&hex32(&image)), str_value("")]
            }
            StoredVersion::View { parent, removed } => [int(i as i64), int(2), int(*parent as i64), str_value(""), ids_text(removed)],
        };
        facts.entry("pipeline_run_version".into()).or_default().push(row.into());
    }
    for (j, s) in parts.steps.iter().enumerate() {
        let (kind, ids, asked, own_end) = match &s.rule {
            Some(r) => (i64::from(r.select.0), ids_text(&r.select.1), str_value(&r.asked.join(",")), i64::from(r.own_end)),
            None => (-1, str_value(""), str_value(""), -1),
        };
        facts.entry("pipeline_run_step".into()).or_default().push(
            [int(j as i64), str_value(&s.pass), int(s.input as i64), int(s.output as i64), int(i64::from(s.form.code())), int(kind), ids, asked, int(own_end)].into(),
        );
    }
    for (j, d) in parts.decided.iter().enumerate() {
        facts.entry("pipeline_run_decision".into()).or_default().push([int(j as i64), str_value(&d.pass), str_value(&d.what), str_value(&d.detail)].into());
    }
    for (i, name, tag, rows) in &parts.analyses {
        let head = [int(*i as i64), str_value(name), int(i64::from(*tag))];
        facts.entry("pipeline_run_analysis".into()).or_default().extend(rows.iter().map(|r| head.iter().cloned().chain(r.iter().cloned()).collect::<Vec<_>>().into()));
    }
    (images, program_facts::encode(&facts))
}

/// Keeps a run (`kept_run`'s images and facts) under `key`: each image under its digest, the facts under the run's
/// alias (once). The sources, which the caller names so collection keeps them (decision 0306 step 4: no pins).
fn store_run(store: &ContentStore, key: &[u8; 32], images: &[&[u8]], facts: &[u8]) -> StoreResult<Vec<SourceId>> {
    let mut ids = Vec::new();
    for bytes in images {
        ids.extend(store_image(store, bytes)?.1);
    }
    let alias = run_alias(key);
    ids.push(match store.source_by_alias(alias)? {
        Some(id) => id,
        None => store.put_source(facts, &[alias])?,
    });
    Ok(ids)
}

/// One run of `record_runs`' item: its key, and its images' and facts' lengths in the item's byte area.
#[derive(serde::Serialize, serde::Deserialize)]
struct RecordedRun {
    key: [u8; 32],
    images: Vec<usize>,
    facts: usize,
}

/// Keeps pipeline runs behind the run, as an item of its record (decision 0306): each encoded here (`kept_run`, on
/// the run's thread), stored by the writer, the sources named by the run's manifest. Replaces Typing-support's job of
/// code on the writer (`StoredRuns::keep` in a closure), at the merge of e829e72a7.
pub fn record_runs(store: &ContentStore, runs: &[([u8; 32], &crate::code_rule_dsl::logic::program_image::manager::Run)]) {
    let mut body = Vec::new();
    let mut bytes = Vec::new();
    for (key, run) in runs {
        let (images, facts) = kept_run(run);
        body.push(RecordedRun { key: *key, images: images.iter().map(Vec::len).collect(), facts: facts.len() });
        images.iter().for_each(|i| bytes.extend_from_slice(i));
        bytes.extend_from_slice(&facts);
    }
    fn apply(cx: &crate::content_store::record::Apply<'_>, body: crate::content_store::record::codec::Body) -> StoreResult<()> {
        let runs: Vec<RecordedRun> = crate::content_store::record::codec::from_body("the pipelines' runs", body)?;
        let mut at = 0;
        let mut take = |n: usize| {
            let b = &cx.bytes[at..at + n];
            at += n;
            b
        };
        for r in runs {
            let images: Vec<&[u8]> = r.images.iter().map(|&n| take(n)).collect();
            let facts = take(r.facts);
            cx.name(store_run(cx.store, &r.key, &images, facts)?);
        }
        Ok(())
    }
    store.record("the pipelines' runs", Vec::new(), &body, Some(bytes), apply);
}

impl crate::code_rule_dsl::logic::program_image::manager::Runs for StoredRuns<'_> {
    fn keep(&self, key: &[u8; 32], run: &crate::code_rule_dsl::logic::program_image::manager::Run) -> Result<(), String> {
        let (images, facts) = kept_run(run);
        store_run(self.0, key, &images.iter().map(Vec::as_slice).collect::<Vec<_>>(), &facts).map(drop).map_err(|e| e.to_string())
    }

    fn open(&self, key: &[u8; 32], name: &'static str) -> Result<Option<crate::code_rule_dsl::logic::program_image::manager::Run>, String> {
        use crate::code_rule_dsl::logic::program_image::manager::{Decided, Form, Run, StoredRule, StoredStep};
        use crate::code_rule_dsl::logic::program_image::Version;
        let e = |e: crate::content_store::StoreError| e.to_string();
        let store = self.0;
        let Some(id) = store.source_by_alias(run_alias(key)).map_err(e)? else { return Ok(None) };
        let facts = program_facts::decode(&store.get_source(id).map_err(e)?)?;
        let rows = |rel: &str| facts.get(rel).cloned().unwrap_or_default();
        let int = |v: &Value| match v {
            Value::Int(n) => Ok(*n),
            _ => Err(bad("a stored run's number is not one")),
        };
        let text = |v: &Value| match v {
            Value::Str(s) => Ok(s.to_string()),
            _ => Err(bad("a stored run's text is not one")),
        };
        let mut by_index: Vec<crate::code_rule_dsl::logic::front::List<Value>> = rows("pipeline_run_version");
        by_index.sort_by_key(|r| int(&r[0]).unwrap_or(i64::MAX));
        let mut versions: Vec<std::sync::Arc<Version>> = Vec::new();
        for r in &by_index {
            let (kind, parent) = (int(&r[1])?, int(&r[2])?);
            let parent = || usize::try_from(parent).ok().and_then(|p| versions.get(p).cloned()).ok_or_else(|| bad("a stored run's version names no parent before it"));
            let bytes = || -> Result<Option<Vec<u8>>, String> {
                let image = unhex32(&text(&r[3])?).ok_or_else(|| bad("a stored run's image is not a digest"))?;
                let Some(image_id) = store.source_by_alias(image_key(&image)).map_err(e)? else { return Ok(None) };
                Ok(Some(store.get_source(image_id).map_err(e)?))
            };
            let v = match kind {
                0 => match bytes()? {
                    Some(b) => Version::open(&b)?,
                    None => return Ok(None),
                },
                1 => match bytes()? {
                    Some(b) => Version::over(&parent()?, &b)?,
                    None => return Ok(None),
                },
                2 => Version::view_without(&parent()?, &text_ids(&r[4])?)?,
                _ => return Err(bad("a stored run's version is of no kind")),
            };
            versions.push(v);
        }
        // Each version's analyses, by name and tag, handed to it to read before computing one.
        let mut analyses: std::collections::BTreeMap<i64, std::collections::HashMap<(String, u32), Vec<Vec<Value>>>> = Default::default();
        for r in rows("pipeline_run_analysis") {
            let (at, name, tag) = (int(&r[0])?, text(&r[1])?, u32::try_from(int(&r[2])?).map_err(|e| bad(e))?);
            analyses.entry(at).or_default().entry((name, tag)).or_default().push(r[3..].to_vec());
        }
        for (at, held) in analyses {
            let v = usize::try_from(at).ok().and_then(|i| versions.get(i)).ok_or_else(|| bad("a stored analysis names no version"))?;
            let held = std::sync::Arc::new(held);
            v.analyses().read_stored(std::sync::Arc::new(move |name: &str, tag: u32| held.get(&(name.to_owned(), tag)).cloned()));
        }
        let mut steps: Vec<(i64, StoredStep)> = Vec::new();
        for r in rows("pipeline_run_step") {
            let kind = int(&r[5])?;
            let rule = (kind >= 0).then(|| -> Result<StoredRule, String> {
                let asked = text(&r[7])?;
                Ok(StoredRule {
                    select: (u8::try_from(kind).map_err(|e| bad(e))?, text_ids(&r[6])?),
                    asked: if asked.is_empty() { Vec::new() } else { asked.split(',').map(str::to_owned).collect() },
                    own_end: u32::try_from(int(&r[8])?).map_err(|e| bad(e))?,
                })
            });
            let form = Form::of_code(u8::try_from(int(&r[4])?).map_err(|e| bad(e))?).ok_or_else(|| bad("a stored step's form is not one"))?;
            steps.push((int(&r[0])?, StoredStep { pass: text(&r[1])?, input: int(&r[2])? as usize, output: int(&r[3])? as usize, form, rule: rule.transpose()? }));
        }
        steps.sort_by_key(|(j, _)| *j);
        let mut decided: Vec<(i64, Decided)> = Vec::new();
        for r in rows("pipeline_run_decision") {
            decided.push((int(&r[0])?, Decided { pass: text(&r[1])?.into(), what: text(&r[2])?.into(), detail: text(&r[3])? }));
        }
        decided.sort_by_key(|(j, _)| *j);
        let Some(head) = rows("pipeline_run").into_iter().next() else { return Ok(None) };
        // No pin (0306 step 4): the run that reads it keeps its own run, which its record names (`record_runs`).
        Run::assemble(name, &versions, int(&head[0])? as usize, int(&head[1])? as usize, steps.into_iter().map(|(_, s)| s).collect(), decided.into_iter().map(|(_, d)| d).collect()).map(Some)
    }
}

/// `facts` as a `.rr` document: the schema's declarations, `extra`'s, and every row as a fact. `rr::parse_version`
/// reads it back as the same rows.
pub fn document(facts: &Facts, extra: &[(&str, &[(&str, &str)])]) -> Result<String, String> {
    use crate::code_rule_dsl::logic::program_image::{Builder, fields::FieldsMut as _};
    let mut b = Builder::base();
    for (rel, cols) in program_facts::schema() {
        let cols: Vec<(&str, &str)> = cols.iter().map(|(c, t)| (c.as_str(), t.as_str())).collect();
        b.declare(rel.as_str(), &cols);
    }
    for (rel, cols) in extra {
        b.declare(rel, cols);
    }
    for (rel, rows) in facts {
        b.set_facts(rel, rows);
    }
    let (v, _) = b.finish()?;
    super::rr::print_version(&v).map_err(|e| format!("program facts: {e:?}"))
}
