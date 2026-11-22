//! How a run keeps what it compiled (a prepared program, a plan choice, a combined program, a pipeline's steps and
//! runs) in the content store: each program as its image (decision 0173), a source by its bytes' digest, and the
//! facts that name them.
//!
//! A program was once kept as its `.rerail` text too (decisions 0155, 0162), read back into a tree. The tree is gone
//! (decision 0276 step 6), and so is that form: a program kept as text alone, by a store written before images, is
//! not read, and the run that wanted it compiles it again.

use rerail_blocks::vector::Vector;
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
    /// Its bytes as they are held: an image where its version holds it, never copied (decision 0464).
    pub bytes: crate::content_store::record::Bytes,
}

/// A program's version as the store keeps it: a base's bytes as they are (a prepared program's version,
/// `Version::base_bytes`, decision 0281), any other version folded into one first (Typing-support's `store_version`,
/// taken at the merge of e829e72a7); `None` where the fold fails.
pub fn encode_version(v: &crate::code_rule_dsl::logic::program_image::Version) -> Option<Encoded> {
    let folded;
    let image = match v.base_bytes() {
        Some(_) => v.own_image()?,
        None => {
            folded = v.fold().ok()?;
            folded.own_image()?
        }
    };
    let bytes = image.bytes();
    crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, bytes.len() as u64);
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    // A run building the engine's bundle (`RERAIL_EXPORT_IMAGES`, decision 0264) ships it.
    crate::code_rule_dsl::logic::rr::images::export(image_key(&digest), bytes);
    Some(Encoded { digest, image: true, bytes: image.clone() })
}

/// Programs `encode_version` made, and a facts entry after them, as a record's item carries them (decision 0306): their
/// digests and kinds in its body, and as its pieces each program's bytes as held (decision 0464), then the entry's.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Packed {
    programs: Vec<([u8; 32], bool, usize)>,
}

/// Packs `programs` and `entry` (`Packed`): the body, and the pieces (each program's bytes as held, then the
/// entry's), nothing copied.
pub fn pack(programs: Vec<Encoded>, entry: Vec<u8>) -> (Packed, Vec<crate::content_store::record::Bytes>) {
    let mut packed = Packed { programs: Vec::with_capacity(programs.len()) };
    let mut pieces: Vec<crate::content_store::record::Bytes> = Vec::with_capacity(programs.len() + 1);
    for p in programs {
        packed.programs.push((p.digest, p.image, (*p.bytes).as_ref().len()));
        pieces.push(p.bytes);
    }
    pieces.push(std::sync::Arc::new(entry));
    (packed, pieces)
}

/// Keeps the programs `pack` packed: their images placed in one segment and read there (`store_images`), a text
/// as the source it was (`store_encoded`); the sources written, and the entry's bytes.
pub fn store_packed<'b>(store: &ContentStore, packed: &Packed, pieces: &'b [crate::content_store::record::Bytes]) -> StoreResult<(Vec<SourceId>, &'b [u8])> {
    let n = packed.programs.len();
    if pieces.len() != n + 1 {
        return Err(crate::content_store::StoreError::Invalid("a record's programs: not their pieces".into()));
    }
    let mut images: Vec<([u8; 32], &[u8])> = Vec::new();
    let mut sources = Vec::new();
    for ((digest, image, _), bytes) in packed.programs.iter().zip(pieces) {
        if *image {
            images.push((*digest, (**bytes).as_ref()));
        } else {
            sources.extend(store_encoded(store, digest, false, (**bytes).as_ref())?);
        }
    }
    sources.extend(store_images(store, &images)?);
    Ok((sources, (*pieces[n]).as_ref()))
}

/// Keeps images by their digests, placed in one segment, uncompressed, so a read maps them there (decision 0464, as
/// 0394 places a stored run's); one the store holds placed keeps its place, one it holds inline is moved there. The
/// sources, which the caller names so collection keeps them.
pub fn store_images(store: &ContentStore, images: &[([u8; 32], &[u8])]) -> StoreResult<Vec<SourceId>> {
    if images.is_empty() {
        return Ok(Vec::new());
    }
    let aliases: Vec<[HashKey; 1]> = images.iter().map(|(d, _)| [image_key(d)]).collect();
    let placed: Vec<(&[u8], HashKey, &[HashKey])> = images.iter().zip(&aliases).map(|((d, b), a)| (*b, HashKey::from_sha256_digest(*d), &a[..])).collect();
    store.put_sources_placed(&placed)
}

/// Keeps a program `encode` made in `store`, a source by its bytes' sha256 (an image by `image_key` too, placed:
/// `store_images`): the sources written, which the caller names so collection keeps them.
pub fn store_encoded(store: &ContentStore, digest: &[u8; 32], image: bool, bytes: &[u8]) -> StoreResult<Vec<SourceId>> {
    if image {
        return store_images(store, &[(*digest, bytes)]);
    }
    let hash = HashKey::sha256(bytes);
    let source = match store.source_by_hash(hash)? {
        Some(id) => id,
        None => store.put_source_hashed(bytes, hash, &[])?,
    };
    Ok(vec![source])
}

/// The images `store` keeps of the programs of sha256 `digests` (`store`), by digest, with their sources: for a caller
/// that must read before its run's waves (decision 0104) and open them later. Each is read where it is held (decision
/// 0464): mapped from its segment, loaded from its row at 16 bytes, or in the shipped bundle's mapping; never copied.
/// One the store has lost, or kept as text alone, is left out.
pub fn stored(store: &ContentStore, digests: &[[u8; 32]]) -> StoreResult<std::collections::HashMap<[u8; 32], (crate::code_rule_dsl::logic::program_image::SharedBytes, Vec<SourceId>)>> {
    let hashes: Vec<HashKey> = digests.iter().map(|d| HashKey::from_sha256_digest(*d)).collect();
    let text_ids = store.sources_by_hashes(&hashes)?;
    let keys: Vec<HashKey> = digests.iter().map(image_key).collect();
    let ids = store.sources_by_aliases(&keys)?;
    let mut bytes = store.sources_by_alias_in_place(&keys)?.into_iter();
    let mut out = std::collections::HashMap::new();
    for (k, d) in digests.iter().enumerate() {
        match (ids[k], bytes.next().flatten()) {
            (Some(id), Some(b)) => {
                // The image's digest is its bytes' (`store`), so it is its own source too: named once.
                out.insert(*d, (std::sync::Arc::new(b) as _, text_ids[k].into_iter().filter(|t| *t != id).chain([id]).collect()));
            }
            // A program the engine ships (decision 0264): its image in the bundle, kept by no source of the store.
            _ if let Some(b) = crate::code_rule_dsl::logic::rr::images::shipped_image(keys[k]) => {
                out.insert(*d, (b, text_ids[k].into_iter().collect()));
            }
            _ => {}
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

/// Whether the next process can read the program sha256 `digest` names from its image: a current image under its
/// alias is the bytes of that digest, a shipped one is checked by its bytes' digest, and any other must open.
pub fn available(store: &ContentStore, digest: &[u8; 32]) -> StoreResult<bool> {
    let key = image_key(digest);
    if let Some(source) = store.source_by_alias(key)? {
        if store.source_by_hash(HashKey::from_sha256_digest(*digest))? == Some(source) {
            return Ok(true);
        }
    } else if let Some(bytes) = crate::code_rule_dsl::logic::rr::images::shipped().and_then(|bundle| bundle.get(key))
        && Sha256::digest(bytes).as_slice() == digest
    {
        return Ok(true);
    }
    Ok(load_image(store, digest)?.is_some())
}

/// The version of the program `store` keeps of sha256 `digest`, opened from its image, and the sources it is kept as;
/// `None` when the store has lost it, keeps it as text alone, or its image does not open.
pub fn load_image(store: &ContentStore, digest: &[u8; 32]) -> StoreResult<Option<(std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>, Vec<SourceId>)>> {
    let mut found = stored(store, &[*digest])?;
    let Some((bytes, sources)) = found.remove(digest) else { return Ok(None) };
    Ok(crate::code_rule_dsl::logic::program_image::Version::open_shared(bytes).ok().map(|v| (v, sources)))
}

/// A keyed pipeline's steps (decision 0281 §6), kept in `store` as facts beside the program images they name. A
/// step is two relations in 0103's form, its key and the step it names: `pipeline_step_key(key, pass, version)` (the
/// pass's version is the tag) and `pipeline_step(key, image, size, digest)`, with its decisions
/// (`pipeline_step_decision(key, k, pass, what, detail)`, 0269). They are one source under its key's alias, and the
/// output is an image as a prepared program's is (`store_encoded`), so a step whose output another step or program
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
        let step = id;
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
        let Some(image_id) = self.0.source_by_alias(image_key(&image)).map_err(e)? else { return Ok(None) };
        // The run that finds it names its sources in its manifest, as the run that kept it did, so collection between
        // two runs keeps them (Q146): an item of its record, named as the writer applies it.
        fn named(cx: &crate::content_store::record::Apply<'_>, body: crate::content_store::record::codec::Body) -> StoreResult<()> {
            let ids: Vec<SourceId> = crate::content_store::record::codec::from_body(STEP_NAMED, body)?;
            cx.name(ids);
            Ok(())
        }
        self.0.record(STEP_NAMED, Vec::new(), &[step, image_id], None, named);
        let mut decided: Vec<(i64, Decided)> = Vec::new();
        for r in rows("pipeline_step_decision") {
            decided.push((int(&r[1])?, Decided { pass: text(&r[2])?.into(), what: text(&r[3])?.into(), detail: text(&r[4])? }));
        }
        decided.sort_by_key(|(k, _)| *k);
        Ok(Some(StepRecord { pass, version, image, output: (int(&out[2])? as usize, int(&out[3])? as u64), decided: decided.into_iter().map(|(_, d)| d).collect() }))
    }

    fn open(&self, image: &[u8; 32]) -> Result<Option<std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>>, String> {
        let e = |e: crate::content_store::StoreError| e.to_string();
        // Read where the store holds it (decision 0464), not copied.
        let Some(bytes) = self.0.source_by_alias_in_place(image_key(image)).map_err(e)? else { return Ok(None) };
        crate::code_rule_dsl::logic::program_image::Version::open_shared(std::sync::Arc::new(bytes)).map(Some)
    }

    fn keep(&self, key: &[u8; 32], record: &crate::code_rule_dsl::logic::program_image::manager::StepRecord, output: &crate::code_rule_dsl::logic::program_image::Version) -> Result<(), String> {
        let folded;
        let held = match output.base_bytes() {
            Some(_) => output.own_image().ok_or("a base holds its image")?,
            None => {
                folded = output.fold()?;
                folded.own_image().filter(|_| folded.base_bytes().is_some()).ok_or("a folded version is a base")?
            }
        };
        let bytes = held.bytes();
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
        // An item of the run's record (decision 0306): its pieces the image as the version holds it and the step's
        // facts (decision 0464); the image placed in a segment (`store_images`) and the facts kept, each unless the
        // store holds it already (an image another step or program stored), and both named in the run's manifest
        // (Q146).
        fn apply(cx: &crate::content_store::record::Apply<'_>, body: crate::content_store::record::codec::Body) -> StoreResult<()> {
            let (key, image): ([u8; 32], [u8; 32]) = crate::content_store::record::codec::from_body(STEP, body)?;
            let [image_bytes, facts] = cx.pieces else { return Err(crate::content_store::StoreError::Invalid("a step's record: not its two pieces".into())) };
            let mut ids = store_images(cx.store, &[(image, (**image_bytes).as_ref())])?;
            let alias = step_alias(&key);
            ids.push(match cx.store.source_by_alias(alias)? {
                Some(id) => id,
                None => cx.store.put_source((**facts).as_ref(), &[alias])?,
            });
            cx.name(ids);
            Ok(())
        }
        let pieces: Vec<crate::content_store::record::Bytes> = vec![held.clone(), std::sync::Arc::new(program_facts::encode(&facts))];
        self.0.record_held(STEP, Vec::new(), &(*key, image), pieces, apply);
        Ok(())
    }
}

/// A pipeline step's item of the run's record (`StoredSteps::keep`).
const STEP: &str = "a pipeline step";
/// A found step's item of the run's record: its sources, named (`StoredSteps::find`, Q146).
const STEP_NAMED: &str = "a pipeline step's sources, named";

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

fn text_ids(v: &Value) -> Result<Vector<u32>, String> {
    let Value::Str(s) = v else { return Err(bad("a stored run's ids are not text")) };
    if s.is_empty() {
        return Ok(rerail_blocks::vector::Vector::new());
    }
    s.split(',').map(|x| x.parse::<u32>().map_err(bad)).collect()
}

/// A run as a store keeps it (`store_run`).
struct KeptRun {
    /// The images the store holds no copy of yet, as their versions hold them (decision 0464).
    images: Vec<std::sync::Arc<crate::code_rule_dsl::logic::program_image::Image>>,
    /// The digests of those it was opened from: named, not copied (decision 0393).
    held: Vector<[u8; 32]>,
    /// Its analyses' rows, one source apart from its facts, each analysis's encoded on its own (decision 0385): a run
    /// opened decodes those a pass asks for, not all.
    rows: Vec<u8>,
    facts: Vec<u8>,
}

/// The alias a run's analyses' rows are kept under: their bytes' sha256, with the image format.
fn rows_alias(digest: &[u8; 32]) -> HashKey {
    let format = crate::code_rule_dsl::logic::program_image::FORMAT;
    HashKey::sha256(&[format!("rerail pipeline run rows {format}\0").as_bytes(), digest.as_slice()].concat())
}

/// A run's images, analyses' rows and facts as a store keeps them (`KeptRun`).
fn kept_run(run: &crate::code_rule_dsl::logic::program_image::manager::Run) -> KeptRun {
    use crate::code_rule_dsl::logic::program_image::manager::StoredVersion;
    let parts = run.parts();
    let mut images: Vec<std::sync::Arc<crate::code_rule_dsl::logic::program_image::Image>> = Vec::new();
    let mut held: Vector<[u8; 32]> = rerail_blocks::vector::Vector::new();
    let mut image = |at: usize, im: &std::sync::Arc<crate::code_rule_dsl::logic::program_image::Image>| -> [u8; 32] {
        let kept = parts.kept.get(at).copied().flatten();
        match kept {
            Some(d) => {
                held.push(d);
                d
            }
            None => {
                images.push(im.clone());
                Sha256::digest(im.bytes()).into()
            }
        }
    };
    let mut facts = Facts::new();
    let int = |n: i64| Value::Int(n);
    facts.entry("pipeline_run".into()).or_default().push([int(parts.input as i64), int(parts.output as i64)].into());
    for (i, v) in parts.versions.iter().enumerate() {
        let row: [Value; 5] = match v {
            StoredVersion::Base(im) => {
                let image = image(i, im);
                [int(i as i64), int(0), int(-1), str_value(&hex32(&image)), str_value("")]
            }
            StoredVersion::Overlay { parent, delta } => {
                let image = image(i, delta);
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
    for (j, s) in parts.steps.iter().enumerate() {
        let head = int(j as i64);
        facts.entry("pipeline_run_step_plan".into()).or_default().extend(s.plan.iter().map(|r| std::iter::once(head.clone()).chain(r.iter().cloned()).collect::<Vec<_>>().into()));
        facts.entry("pipeline_run_step_said".into()).or_default().extend(s.said.iter().map(|((topic, text), n)| [head.clone(), str_value(topic), str_value(text), int(*n as i64)].into()));
        // The request record after the step (decision 0393): its requests, and each request rule's readers.
        if let Some(a) = &s.asked {
            facts.entry("pipeline_run_step_req".into()).or_default().extend(a.reqs.iter().map(|r| [head.clone(), str_value(r)].into()));
            facts.entry("pipeline_run_step_asked".into()).or_default().extend(a.by.iter().flat_map(|(id, rs)| rs.iter().map(|r| [head.clone(), int(i64::from(*id)), str_value(r)].into()).collect::<Vec<_>>()));
        }
    }
    for (j, d) in parts.decided.iter().enumerate() {
        facts.entry("pipeline_run_decision".into()).or_default().push([int(j as i64), str_value(&d.pass), str_value(&d.what), str_value(&d.detail)].into());
    }
    // Each analysis's rows encoded on its own, at its place in the rows' source.
    let mut all_rows: Vec<u8> = Vec::new();
    for (i, name, tag, rows) in &parts.analyses {
        let one: Facts = std::iter::once(("rows".to_owned(), rows.iter().map(|r| r.clone().into()).collect())).collect();
        let bytes = program_facts::encode(&one);
        facts.entry("pipeline_run_analysis_at".into()).or_default().push([int(*i as i64), str_value(name), int(i64::from(*tag)), int(all_rows.len() as i64), int(bytes.len() as i64)].into());
        all_rows.extend_from_slice(&bytes);
    }
    if !all_rows.is_empty() {
        let digest: [u8; 32] = Sha256::digest(&all_rows).into();
        facts.entry("pipeline_run_rows".into()).or_default().push([str_value(&hex32(&digest))].into());
    }
    drop(image);
    KeptRun { images, held, rows: all_rows, facts: program_facts::encode(&facts) }
}

/// Keeps a run (`kept_run`'s images and facts) under `key`: each image under its digest, the facts under the run's
/// alias (once). The sources, which the caller names so collection keeps them (decision 0306 step 4: no pins).
///
/// Every image, the rows and the facts are placed in one segment of the store's objects, uncompressed, so `open`
/// reads them there, mapped (decision 0394); an image the store holds already (`held`, or the same bytes kept by
/// another path) inline is moved there, its bytes read once.
fn store_run(store: &ContentStore, key: &[u8; 32], images: &[&[u8]], held: &[[u8; 32]], rows: &[u8], facts: &[u8]) -> StoreResult<Vec<SourceId>> {
    let held_bytes = store.sources_by_alias_in_place(&held.iter().map(image_key).collect::<Vec<_>>())?;
    let inline: Vec<crate::content_store::ObjectBytes> = held_bytes.into_iter().flatten().filter(|b| !b.in_place()).collect();
    let mut items: Vec<(&[u8], [u8; 32], HashKey)> = Vec::new();
    for bytes in images.iter().copied().chain(inline.iter().map(|b| &**b)) {
        crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, bytes.len() as u64);
        let digest: [u8; 32] = Sha256::digest(bytes).into();
        items.push((bytes, digest, image_key(&digest)));
    }
    if !rows.is_empty() {
        let digest: [u8; 32] = Sha256::digest(rows).into();
        items.push((rows, digest, rows_alias(&digest)));
    }
    // A key's facts are kept once (`run_alias`): a run kept again under it leaves the first.
    let kept_facts = store.source_by_alias(run_alias(key))?;
    if kept_facts.is_none() {
        let digest: [u8; 32] = Sha256::digest(facts).into();
        items.push((facts, digest, run_alias(key)));
    }
    let aliases: Vec<[HashKey; 1]> = items.iter().map(|i| [i.2]).collect();
    let placed: Vec<(&[u8], HashKey, &[HashKey])> = items.iter().zip(&aliases).map(|(i, a)| (i.0, HashKey::from_sha256_digest(i.1), &a[..])).collect();
    let mut ids = store.put_sources_placed(&placed)?;
    for d in held {
        ids.extend(store.source_by_alias(image_key(d))?);
    }
    ids.extend(kept_facts);
    Ok(ids)
}

/// The sources a pipeline run kept under `key` lives in (`store_run`'s): its versions' images and its facts. `None`
/// when the store does not hold it whole. A run that finds its prepared program names a kept run's sources by these,
/// as the run that kept it named them (decision 0333).
pub fn run_sources(store: &ContentStore, key: &[u8; 32]) -> Result<Option<Vec<SourceId>>, String> {
    let e = |e: crate::content_store::StoreError| e.to_string();
    let Some(id) = store.source_by_alias(run_alias(key)).map_err(e)? else { return Ok(None) };
    let facts = program_facts::decode(&store.get_source(id).map_err(e)?)?;
    let mut ids = Vec::new();
    for r in facts.get("pipeline_run_version").into_iter().flatten() {
        let (Some(Value::Int(kind)), Some(Value::Str(image))) = (r.get(1), r.get(3)) else { return Err(bad("a stored run's version is not one")) };
        if !matches!(kind, 0 | 1) {
            continue;
        }
        let image = unhex32(image).ok_or_else(|| bad("a stored run's image is not a digest"))?;
        let Some(image_id) = store.source_by_alias(image_key(&image)).map_err(e)? else { return Ok(None) };
        ids.push(image_id);
    }
    // Its analyses' rows, a source of their own (decision 0385), which the run that kept it named with the rest.
    if let Some(r) = facts.get("pipeline_run_rows").and_then(|r| r.first()) {
        let Some(Value::Str(digest)) = r.first() else { return Err(bad("a stored run's rows are not a digest")) };
        let digest = unhex32(digest).ok_or_else(|| bad("a stored run's rows are not a digest"))?;
        let Some(rows_id) = store.source_by_alias(rows_alias(&digest)).map_err(e)? else { return Ok(None) };
        ids.push(rows_id);
    }
    ids.push(id);
    Ok(Some(ids))
}

/// One run of `record_runs`' item: its key, and how many of the item's pieces are its images (then its rows' and its
/// facts' pieces).
#[derive(serde::Serialize, serde::Deserialize)]
struct RecordedRun {
    key: [u8; 32],
    images: Vector<usize>,
    /// The images the store holds already, by digest: named, not copied.
    #[serde(default)]
    held: Vector<[u8; 32]>,
    /// Its analyses' rows' length (`KeptRun::rows`).
    #[serde(default)]
    rows: usize,
    facts: usize,
}

/// Keeps pipeline runs behind the run, as an item of its record (decision 0306): each encoded here (`kept_run`, on
/// the run's thread), stored by the writer, the sources named by the run's manifest. Replaces Typing-support's job of
/// code on the writer (`StoredRuns::keep` in a closure), at the merge of e829e72a7.
pub fn record_runs(store: &ContentStore, runs: &[([u8; 32], &crate::code_rule_dsl::logic::program_image::manager::Run)]) {
    let mut body = Vec::new();
    let mut pieces: Vec<crate::content_store::record::Bytes> = Vec::new();
    for (key, run) in runs {
        let k = kept_run(run);
        body.push(RecordedRun { key: *key, images: k.images.iter().map(|i| i.bytes().len()).collect(), held: k.held, rows: k.rows.len(), facts: k.facts.len() });
        // Its images as their versions hold them (decision 0464), then its rows and its facts: nothing copied.
        pieces.extend(k.images.into_iter().map(|i| i as crate::content_store::record::Bytes));
        pieces.push(std::sync::Arc::new(k.rows));
        pieces.push(std::sync::Arc::new(k.facts));
    }
    fn apply(cx: &crate::content_store::record::Apply<'_>, body: crate::content_store::record::codec::Body) -> StoreResult<()> {
        let runs: Vec<RecordedRun> = crate::content_store::record::codec::from_body("the pipelines' runs", body)?;
        let mut next = cx.pieces.iter().map(|p| (**p).as_ref());
        let mut take = || next.next().ok_or_else(|| crate::content_store::StoreError::Invalid("the pipelines' runs: past their pieces".into()));
        for r in runs {
            let images: Vec<&[u8]> = r.images.iter().map(|_| take()).collect::<StoreResult<_>>()?;
            let rows = take()?;
            let facts = take()?;
            cx.name(store_run(cx.store, &r.key, &images, &r.held, rows, facts)?);
        }
        Ok(())
    }
    store.record_held("the pipelines' runs", Vec::new(), &body, pieces, apply);
}

impl crate::code_rule_dsl::logic::program_image::manager::Runs for StoredRuns<'_> {
    fn keep(&self, key: &[u8; 32], run: &crate::code_rule_dsl::logic::program_image::manager::Run) -> Result<(), String> {
        let k = kept_run(run);
        store_run(self.0, key, &k.images.iter().map(|i| i.bytes()).collect::<Vec<_>>(), &k.held, &k.rows, &k.facts).map(drop).map_err(|e| e.to_string())
    }

    fn open(&self, key: &[u8; 32], name: &'static str) -> Result<Option<crate::code_rule_dsl::logic::program_image::manager::Run>, String> {
        Ok(self.open_counted(key, name)?.map(|(run, _)| run))
    }
}

impl StoredRuns<'_> {
    /// `Runs::open`, with how the run was read (`Opened`).
    pub fn open_counted(&self, key: &[u8; 32], name: &'static str) -> Result<Option<(crate::code_rule_dsl::logic::program_image::manager::Run, Opened)>, String> {
        use crate::code_rule_dsl::logic::program_image::manager::{Decided, Form, Run, StoredRule, StoredStep};
        use crate::code_rule_dsl::logic::program_image::Version;
        let e = |e: crate::content_store::StoreError| e.to_string();
        let store = self.0;
        let span = |what: &'static str| crate::code_rule_dsl::logic::profile::span(move || format!("a stored run: {what}"));
        let s = span("its facts read and decoded");
        let Some(blob) = store.sources_by_alias_in_place(&[run_alias(key)]).map_err(e)?.pop().flatten() else { return Ok(None) };
        let facts = program_facts::decode(&blob)?;
        drop(s);
        // Read in place, not copied: the analyses' rows are most of a run (decision 0385).
        let rows = |rel: &str| facts.get(rel).map_or(&[][..], |r| r.as_slice());
        let int = |v: &Value| match v {
            Value::Int(n) => Ok(*n),
            _ => Err(bad("a stored run's number is not one")),
        };
        let text = |v: &Value| match v {
            Value::Str(s) => Ok(s.to_string()),
            _ => Err(bad("a stored run's text is not one")),
        };
        let s = span("its versions opened");
        let mut by_index: Vec<&crate::code_rule_dsl::logic::front::List<Value>> = rows("pipeline_run_version").iter().collect();
        by_index.sort_by_key(|r| int(&r[0]).unwrap_or(i64::MAX));
        // Every image, and the analyses' rows, in one read: where the store placed them (`store_run`), read there,
        // mapped, and opened over the mapping, not decompressed, hashed or copied (decision 0394).
        let digests: Vec<Option<[u8; 32]>> = by_index
            .iter()
            .map(|r| if matches!(r[1], Value::Int(2)) { Ok(None) } else { text(&r[3]).and_then(|t| unhex32(&t).ok_or_else(|| bad("a stored run's image is not a digest"))).map(Some) })
            .collect::<Result<_, String>>()?;
        let rows_digest = match rows("pipeline_run_rows").first() {
            Some(r) => Some(unhex32(&text(&r[0])?).ok_or_else(|| bad("a stored run's rows are not a digest"))?),
            None => None,
        };
        let mut wanted: Vec<HashKey> = digests.iter().flatten().map(image_key).collect();
        wanted.extend(rows_digest.as_ref().map(rows_alias));
        let mut read = {
            let _s = span("its images and rows read");
            store.sources_by_alias_in_place(&wanted).map_err(e)?.into_iter()
        };
        let mut opened_in_place = 0;
        let mut versions: Vec<std::sync::Arc<Version>> = Vec::new();
        for (r, image) in by_index.iter().zip(&digests) {
            let (kind, parent) = (int(&r[1])?, int(&r[2])?);
            let parent = || usize::try_from(parent).ok().and_then(|p| versions.get(p).cloned()).ok_or_else(|| bad("a stored run's version names no parent before it"));
            let bytes = match image {
                Some(_) => match read.next().flatten() {
                    Some(b) => Some(b),
                    None => return Ok(None),
                },
                None => None,
            };
            let opened = || span("an image opened");
            let v = match (kind, bytes) {
                (0 | 1, Some(b)) => {
                    let _s = opened();
                    let in_place = b.in_place();
                    // Mapped from its segment, or loaded from its row at 16 bytes (decision 0464): opened where it is.
                    let v = match kind {
                        0 => Version::open_shared(std::sync::Arc::new(b))?,
                        _ => Version::over_shared(&parent()?, std::sync::Arc::new(b))?,
                    };
                    opened_in_place += usize::from(in_place);
                    v
                }
                (2, None) => Version::view_without(&parent()?, &text_ids(&r[4])?)?,
                _ => return Err(bad("a stored run's version is of no kind")),
            };
            if let Some(d) = image {
                v.analyses().set_kept_image(*d);
            }
            versions.push(v);
        }
        drop(s);
        let _s = span("its analyses' rows and its steps");
        // Each version's analyses, by name and tag, handed to it to read before computing one.
        // Each version's analyses, by name and tag: where its rows are in the rows' source, decoded when asked for
        // (decision 0385), not here.
        let mut analyses: std::collections::BTreeMap<i64, std::collections::HashMap<(String, u32), (usize, usize)>> = Default::default();
        for r in rows("pipeline_run_analysis_at") {
            let (at, name, tag) = (int(&r[0])?, text(&r[1])?, u32::try_from(int(&r[2])?).map_err(bad)?);
            let (from, len) = (usize::try_from(int(&r[3])?).map_err(bad)?, usize::try_from(int(&r[4])?).map_err(bad)?);
            analyses.entry(at).or_default().insert((name, tag), (from, len));
        }
        let all_rows: std::sync::Arc<crate::content_store::ObjectBytes> = std::sync::Arc::new(match rows_digest {
            Some(_) => match read.next().flatten() {
                Some(b) => b,
                None => return Ok(None),
            },
            None => crate::content_store::ObjectBytes::Heap(std::sync::Arc::new(Vec::new())),
        });
        let images = digests.iter().flatten().count();
        let opened = Opened { images, in_place: opened_in_place, facts_in_place: blob.in_place(), rows_in_place: rows_digest.is_none() || all_rows.in_place() };
        crate::code_rule_dsl::logic::profile::decision("a stored run, opened", || {
            let deepest = versions.iter().map(|v| v.depth()).max().unwrap_or(0);
            let place = |b: bool| if b { "in place" } else { "decompressed" };
            format!(
                "{} versions ({images} images, {} read in place, deepest {deepest}), its facts {} bytes ({}), its analyses' rows {} bytes ({})",
                versions.len(),
                opened.in_place,
                blob.len(),
                place(opened.facts_in_place),
                all_rows.len(),
                place(opened.rows_in_place)
            )
        });
        for (at, held) in analyses {
            let v = usize::try_from(at).ok().and_then(|i| versions.get(i)).ok_or_else(|| bad("a stored analysis names no version"))?;
            let all_rows = all_rows.clone();
            v.analyses().read_stored(std::sync::Arc::new(move |name: &str, tag: u32| {
                let &(from, len) = held.get(&(name.to_owned(), tag))?;
                let one = program_facts::decode(all_rows.get(from..from + len)?).ok()?;
                Some(one.get("rows").map_or_else(Vec::new, |rows| rows.iter().map(|r| r.to_vec()).collect()))
            }));
        }
        let mut steps: Vec<(i64, StoredStep)> = Vec::new();
        for r in rows("pipeline_run_step") {
            let kind = int(&r[5])?;
            let rule = (kind >= 0).then(|| -> Result<StoredRule, String> {
                let asked = text(&r[7])?;
                Ok(StoredRule {
                    select: (u8::try_from(kind).map_err(bad)?, text_ids(&r[6])?),
                    asked: if asked.is_empty() { Vec::new() } else { asked.split(',').map(str::to_owned).collect() },
                    own_end: u32::try_from(int(&r[8])?).map_err(bad)?,
                })
            });
            let form = Form::of_code(u8::try_from(int(&r[4])?).map_err(bad)?).ok_or_else(|| bad("a stored step's form is not one"))?;
            steps.push((int(&r[0])?, StoredStep { pass: text(&r[1])?, input: int(&r[2])? as usize, output: int(&r[3])? as usize, form, rule: rule.transpose()?, plan: Vec::new(), said: Vec::new(), asked: None }));
        }
        steps.sort_by_key(|(j, _)| *j);
        for r in rows("pipeline_run_step_plan") {
            let j = int(&r[0])?;
            if let Ok(at) = steps.binary_search_by_key(&j, |(k, _)| *k) {
                steps[at].1.plan.push(r[1..].to_vec());
            }
        }
        for r in rows("pipeline_run_step_said") {
            let j = int(&r[0])?;
            if let Ok(at) = steps.binary_search_by_key(&j, |(k, _)| *k) {
                steps[at].1.said.push(((text(&r[1])?, text(&r[2])?), u64::try_from(int(&r[3])?).map_err(bad)?));
            }
        }
        for r in rows("pipeline_run_step_req") {
            let j = int(&r[0])?;
            if let Ok(at) = steps.binary_search_by_key(&j, |(k, _)| *k) {
                steps[at].1.asked.get_or_insert_with(Default::default).reqs.push(text(&r[1])?);
            }
        }
        for r in rows("pipeline_run_step_asked") {
            let j = int(&r[0])?;
            let Ok(at) = steps.binary_search_by_key(&j, |(k, _)| *k) else { continue };
            let a = steps[at].1.asked.as_mut().ok_or_else(|| bad("a stored step's request record has readers and no requests"))?;
            let id = u32::try_from(int(&r[1])?).map_err(bad)?;
            match a.by.iter_mut().find(|(at, _)| *at == id) {
                Some((_, rs)) => rs.push(text(&r[2])?),
                None => a.by.push((id, vec![text(&r[2])?])),
            }
        }
        let mut decided: Vec<(i64, Decided)> = Vec::new();
        for r in rows("pipeline_run_decision") {
            decided.push((int(&r[0])?, Decided { pass: text(&r[1])?.into(), what: text(&r[2])?.into(), detail: text(&r[3])? }));
        }
        decided.sort_by_key(|(j, _)| *j);
        let Some(head) = rows("pipeline_run").first() else { return Ok(None) };
        // No pin (0306 step 4): the run that reads it keeps its own run, which its record names (`record_runs`).
        Run::assemble(name, &versions, int(&head[0])? as usize, int(&head[1])? as usize, steps.into_iter().map(|(_, s)| s).collect(), decided.into_iter().map(|(_, d)| d).collect()).map(|run| Some((run, opened)))
    }
}

/// How a stored run was read (`StoredRuns::open_counted`, decision 0394): its images, how many were read where the
/// store placed them (mapped, not decompressed, hashed or copied), and whether its facts and its analyses' rows were.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Opened {
    pub images: usize,
    pub in_place: usize,
    pub facts_in_place: bool,
    pub rows_in_place: bool,
}

/// `facts` as a `.rerail` document: the schema's declarations, `extra`'s, and every row as a fact. `rr::parse_version`
/// reads it back as the same rows.
pub fn document(facts: &Facts, extra: &[(&str, &[(&str, &str)])]) -> Result<String, String> {
    use crate::code_rule_dsl::logic::program_image::{Builder, fields::FieldsMut as _};
    let mut b = Builder::base();
    for (rel, cols) in program_facts::schema() {
        let cols: Vec<(&str, &str)> = cols.iter().map(|(c, t)| (c.as_str(), t.as_str())).collect();
        b.fields().declare(rel.as_str(), &cols);
    }
    for (rel, cols) in extra {
        b.fields().declare(rel, cols);
    }
    for (rel, rows) in facts {
        b.fields().set_facts(rel, rows);
    }
    let (v, _) = b.finish()?;
    super::rr::print_version(&v).map_err(|e| format!("program facts: {e:?}"))
}
