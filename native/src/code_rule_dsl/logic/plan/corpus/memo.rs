//! A corpus run's file-local results, memoised per file (decision 0052): what the chunks hand on for one document
//! (its rows of each exported relation and its file-local outputs' rows) under its content, its path and the run's
//! plan. A later run whose plan is the same reads a file's entry instead of parsing and evaluating it.
//!
//! The rows are replayable as they are because their values are stable across runs: a document's key is a function
//! of its path (`doc_keys`), a node is `key << 32 | index` in its tape (a function of the content and the grammar),
//! a kind is its grammar code, and a symbol is stored as its string (decision 0240 §4, `image`), and read as the
//! reading process's atom of it. The plan names everything else the rows depend on: the engine build, the
//! grammar version and the file-local program (its constants as atoms). It is one per run, so the run's manifest names
//! it (decision 0053); the document key is in the entry, and an entry under another key is a miss.

use crate::content_store::{ContentStore, HashKey, SourceId, StoreError, StoreResult};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

mod image;

/// The memo's fact schema: bumped when an entry's meaning changes.
/// When it changes, the old number joins `RETIRED_CORPUS_MEMO_SCHEMAS` only if its rows could outlive their plans
/// (they cannot since decision 0053: the plan names the engine build, and dead plans are collected).
const SCHEMA: u32 = crate::content_store::live_fact_schema(8);

/// The entry's key for a base relation's rows (`rows`), kept for the cross-file part that reads them: a memoised
/// file hands on its base rows as an evaluated one does.
pub(super) fn base_key(rel: &str) -> String {
    format!("base:{rel}")
}

/// The entry's key for the identities of the relations it holds (decision 0225 §2): two words of each export's and
/// file-local output's identity (`identities`) it was made under. A relation's rows in an entry are its rows only
/// when its identity now is among them; a relation with no rows for the file has no key in `rows`, so whether it
/// is held is read here, not from `rows`. An `own:` key: it holds no symbol. An entry whose rows stand as stored
/// takes more of them kept over it (`identities_of`, decision 0280).
pub(super) const COVERED: &str = "own:covered";

/// What a hit read for its outputs alone decodes besides its outputs (decision 0117 step 1): its `own:` rows (its tree
/// counts and `COVERED`), not its export or base rows, which the kept inputs hold. An entry is one image (decision
/// 0240 §4), so it is held whole and only these are decoded; `Found::whole` decodes the rest if the kept inputs fail.
pub(super) fn outputs_alone(rel: &str) -> bool {
    rel.starts_with("own:")
}

/// The prefix of an entry's `lines` keys that hold its guests' requests (decision 0225 §2): the embed slice's output
/// lines of each round that ask at a location of the file (`guests::asked_at`), under `guests:<round>:<relation>`. A
/// chunk of partial hits whose entries hold them under the slice's identities makes its guests from them, without
/// running the slice. Not an output: they are no relation of the file-local part.
pub(super) const GUESTS: &str = "guests:";

/// An entry's `outputs` key for round `round`'s lines of slice relation `rel`.
pub(super) fn guest_key(round: usize, rel: &str) -> String {
    format!("{GUESTS}{round}:{rel}")
}

/// The round and slice relation of a `guest_key`, `None` for an output's key.
pub(super) fn guest_round(key: &str) -> Option<(usize, &str)> {
    let (round, rel) = key.strip_prefix(GUESTS)?.split_once(':')?;
    Some((round.parse().ok()?, rel))
}

/// Whether `a` and `b` hold the same guests' requests (`GUESTS`).
pub(super) fn same_guests(a: &Entry, b: &Entry) -> bool {
    let of = |e: &Entry| -> Vec<(String, Vec<Vec<String>>)> {
        e.outputs.iter().filter(|(k, _)| k.starts_with(GUESTS)).map(|(k, (_, r))| (k.clone(), r.shown())).filter(|(_, l)| !l.is_empty()).collect()
    };
    of(a) == of(b)
}

/// The words `identity` is held by in `COVERED`.
pub(super) fn covered_words(identity: &[u8; 32]) -> [i64; 2] {
    [i64::from_le_bytes(identity[..8].try_into().expect("8 bytes")), i64::from_le_bytes(identity[8..16].try_into().expect("8 bytes"))]
}

/// The identities `entry` holds (`COVERED`).
pub(super) fn covered(entry: &Entry) -> HashSet<[i64; 2]> {
    entry.rows.get(COVERED).map(|(_, v)| v.chunks_exact(2).map(|w| [w[0], w[1]]).collect()).unwrap_or_default()
}

/// Whether `a` and `b` hold the same rows (exported or output) of relation `rel`, in any order.
pub(super) fn same_relation(a: &Entry, b: &Entry, rel: &str) -> bool {
    let rows = |e: &Entry| -> Option<(usize, Vec<Vec<i64>>)> {
        e.rows.get(rel).filter(|(_, v)| !v.is_empty()).map(|(w, v)| {
            let mut rows: Vec<Vec<i64>> = v.chunks_exact((*w).max(1)).map(<[i64]>::to_vec).collect();
            rows.sort_unstable();
            (*w, rows)
        })
    };
    // An output's rows by their values (`Rows::shown`, sorted): two entries' texts are numbered apart.
    let lines = |e: &Entry| -> Option<Vec<Vec<String>>> { e.outputs.get(rel).filter(|(_, r)| !r.is_empty()).map(|(_, r)| r.shown()) };
    rows(a) == rows(b) && lines(a) == lines(b)
}

/// What is written of an entry whose rows stand as stored (decision 0280): the identities it holds now (`COVERED`),
/// kept over the stored entry, whose rows, lines and other identities they are.
fn identities_of(entry: &Entry) -> Entry {
    Entry { key: entry.key, rows: entry.rows.get(COVERED).map(|v| (COVERED.to_owned(), v.clone())).into_iter().collect(), ..Entry::default() }
}

/// Records that `entry` holds the relations of `identities`.
pub(super) fn cover<'a>(entry: &mut Entry, identities: impl Iterator<Item = &'a [u8; 32]>) {
    let mut words: Vec<[i64; 2]> = covered(entry).into_iter().chain(identities.map(covered_words)).collect();
    words.sort_unstable();
    words.dedup();
    entry.rows.insert(COVERED.to_owned(), (2, words.into_iter().flatten().collect()));
}

/// Each of `rels`' identity in the file-local program `local` (decision 0225 §2): `plan::identities`, a digest of its
/// stratum's rules and of what they read down to the inputs, with the inputs whose rows are not in the program's
/// text digested with their values where a relation's rules reach them: the run's options (`option_input`) and the
/// earlier phases' exports (`phased`), and the projection the base rows are made under (`base`,
/// `Projection::rows_digest`). So an export's identity changes when a rule it is derived from does, or what the
/// documents' rows are made of, and only then.
pub(in crate::code_rule_dsl) fn identities(
    local: &crate::code_rule_dsl::logic::plan::body::Placed,
    rels: &BTreeSet<String>,
    options: &serde_json::Value,
    phased: &BTreeMap<String, Vec<Vec<crate::code_rule_dsl::logic::front::Value>>>,
    base: &[u8; 32],
) -> Result<BTreeMap<String, [u8; 32]>, String> {
    Ok(strata(local, rels)?.identities(options, phased, base))
}

/// What a relation's memo identity is made of in a file-local program (`identities`): its strata's identity (a
/// digest of its stratum's rules and of what they read down to the inputs), and the relations whose rows its rules
/// may be read with as values: the inputs and option inputs they reach. A function of the program alone, so a
/// prepared program keeps it (decision 0246 §4): a run adds the options, the earlier phases' rows and the projection.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Strata {
    /// Each relation's strata identity, and the relations it reaches that may hold values (`identities`' leaves).
    pub by_rel: BTreeMap<String, ([u8; 32], Vec<String>)>,
}

/// `rels`' `Strata` in the file-local program `local`.
pub(in crate::code_rule_dsl) fn strata(local: &crate::code_rule_dsl::logic::plan::body::Placed, rels: &BTreeSet<String>) -> Result<Strata, String> {
    // Read from the program's version (t36 item 7): its components, dependencies first, and each one's rules by head.
    let sccs = local.components().map_err(|e| e.to_string())?;
    let ids = crate::code_rule_dsl::logic::plan::Leaves::of_placed(local.clone()).identities_by_heads(&sccs).map_err(|e| e.to_string())?;
    let v = &local.v;
    let at: HashMap<&str, usize> = sccs.iter().enumerate().flat_map(|(i, scc)| scc.iter().map(move |r| (r.as_str(), i))).collect();
    let mut reads: Vec<Vec<&str>> = vec![Vec::new(); sccs.len()];
    for k in 0..local.len() {
        let rule = local.rule(k);
        if let Some(&i) = at.get(v.rel_name(rule.head().rel())) {
            reads[i].extend(rule.reads().map(|a| v.rel_name(a.rel())));
        }
    }
    // The leaves each stratum reaches, in the strata's order: an option input, or a relation its program does not
    // derive (an input; an earlier phase's export is one), where `identities` asks which hold values; through every
    // other derived relation.
    let mut reach: HashMap<&str, BTreeSet<&str>> = HashMap::new();
    for (scc, reads) in sccs.iter().zip(&reads) {
        let mut leaves: BTreeSet<&str> = BTreeSet::new();
        for &rel in reads {
            if crate::code_rule_dsl::typing::file_local::option_input(rel) || !at.contains_key(rel) {
                leaves.insert(rel);
            } else if let Some(more) = reach.get(rel) {
                leaves.extend(more.iter().copied());
            }
        }
        for rel in scc {
            reach.insert(rel.as_str(), leaves.clone());
        }
    }
    let by_rel = rels
        .iter()
        .filter_map(|rel| {
            let id = *ids.get(rel)?;
            Some((rel.clone(), (id, reach.get(rel.as_str()).map(|l| l.iter().map(|r| (*r).to_owned()).collect()).unwrap_or_default())))
        })
        .collect();
    Ok(Strata { by_rel })
}

impl Strata {
    /// Each relation's identity (decision 0225 §2): its strata's, with the inputs whose rows are not in the program's
    /// text digested with their values where its rules reach them: the run's options (`option_input`) and the earlier
    /// phases' exports (`phased`), and the projection the base rows are made under (`base`,
    /// `Projection::rows_digest`). So an export's identity changes when a rule it is derived from does, or what the
    /// documents' rows are made of, and only then.
    pub(in crate::code_rule_dsl) fn identities(&self, options: &serde_json::Value, phased: &BTreeMap<String, Vec<Vec<crate::code_rule_dsl::logic::front::Value>>>, base: &[u8; 32]) -> BTreeMap<String, [u8; 32]> {
        let mut out = BTreeMap::new();
        for (rel, (id, reached)) in &self.by_rel {
            let mut hash = Sha256::new();
            hash.update(b"rerail.corpus-local-identity/v2\0");
            hash.update(id);
            hash.update(base);
            if reached.iter().any(|r| crate::code_rule_dsl::typing::file_local::option_input(r)) {
                hash.update(options.to_string().as_bytes());
            }
            let by_rows: Vec<&str> = reached.iter().map(String::as_str).filter(|r| phased.contains_key(*r)).collect();
            out.insert(rel.clone(), with_rows(hash.finalize().into(), by_rows.iter().map(|r| (*r, &phased[*r]))));
        }
        out
    }
}

/// Which column of a base relation's row (in `input_columns` order) names the file the row is kept for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Owner {
    /// A node or a document key: the row is its document's (`document_of`).
    Node(usize),
    /// A string: the row is each file's whose text holds it, as a text test's row is (`strings::TEXT_COLUMNS`).
    Text(usize),
}

/// The owner column of `rel`'s rows. Most base relations lead with their node or document; a pattern's spans and
/// matches lead with the pattern (`rx_span(p, c, …)`, `ast_match(p, n, …)`), so their node is the second column;
/// `value_type(v, t)` is keyed by the value itself, a string some file's `value(n, v)` holds.
pub(super) fn owner(rel: &str) -> Owner {
    match rel {
        "rx_span" | "ast_match" => Owner::Node(1),
        "value_type" => Owner::Text(0),
        _ if rel.starts_with(crate::code_rule_dsl::logic::strings::RELATION) => Owner::Text(0),
        _ => Owner::Node(0),
    }
}

/// What the chunks handed on for one document.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    /// The document key the rows were made under (`doc_keys`): their node ids are `key << 32 | index`.
    pub key: i64,
    /// The document's nodes (for the run's report).
    pub nodes: u64,
    /// Per exported relation: its width and its rows, flat.
    pub rows: BTreeMap<String, (usize, Vec<i64>)>,
    /// Per file-local output: its node columns and its rows, typed as the run handed them.
    pub outputs: BTreeMap<String, (Vec<usize>, crate::code_rule_dsl::logic::plan::output::Rows)>,
}

/// Each path's document key: from its hash, in `1..2^30`, the next free one on a collision. A key is in each entry,
/// so a key a collision moved only misses the memo. Guests (decision 0055) take `2^30..2^31`, so every key's
/// `key << 32` stays positive.
pub fn doc_keys(paths: &[String]) -> Vec<i64> {
    const SPAN: u64 = (1 << 30) - 1;
    let mut taken = HashSet::with_capacity(paths.len());
    paths
        .iter()
        .map(|path| {
            let hash: [u8; 32] = Sha256::digest(path.as_bytes()).into();
            let mut key = u64::from_le_bytes(hash[..8].try_into().expect("8 bytes")) % SPAN + 1;
            while !taken.insert(key) {
                key = key % SPAN + 1;
            }
            key as i64
        })
        .collect()
}

/// The memo's frames' program part (decision 0225 §2): the phase that runs, and the names of the file-local part's
/// relations (those an entry holds, those its rules derive, its inputs), not their rules, which each entry's
/// `COVERED` identities name. Two programs of other relations (a phase's, a nested run's) keep their entries apart;
/// a rule's edit keeps the space, and misses only the relations it reaches.
pub(super) fn space_digest(ids: &BTreeMap<String, [u8; 32]>, local: &crate::code_rule_dsl::logic::plan::body::Placed, phase: Option<&str>) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"rerail.corpus-local-space/v2\0");
    hash.update(phase.unwrap_or("").as_bytes());
    // Read from the program's version (t36 item 7).
    let v = &local.v;
    let derived: BTreeSet<&str> = (0..local.len()).map(|k| v.rel_name(local.rule(k).head().rel())).collect();
    let inputs: BTreeSet<&str> = v.inputs().map(|a| v.text(a)).collect();
    for part in [ids.keys().map(String::as_str).collect::<BTreeSet<&str>>(), derived, inputs] {
        hash.update(b"\x01");
        for rel in part {
            hash.update(rel.as_bytes());
            hash.update(b"\0");
        }
    }
    hash.finalize().into()
}

/// Where the memo's space `space` names the whole file-local program whose entries it holds last (`set_last_program`).
/// Under the space's plan (`plan_digest`: the engine build, the grammar, the node ids' scheme), as its entries are
/// (`local_space`): another build's entries are not this build's hits, so its last program, and the statistics kept
/// under it, are not this build's either (Q102). Before, a new build read the old build's keeper, whose counts
/// trusted entries it could not read, and gave it up at the edited file.
fn last_key(space: &[u8; 32]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"rerail.corpus-local-last/v2\0");
    hash.update(plan_digest(space));
    hash.finalize().into()
}

/// The whole file-local program whose run last wrote the entries of space `space` (decision 0225 §2), if what is kept
/// of every file together (the kept statistics) is still of its entries: its run ended, or a run of it under way
/// started from what an ended run of it kept (`set_under_way`). Of no other program's.
pub(super) fn last_program(store: &ContentStore, space: &[u8; 32]) -> Option<[u8; 32]> {
    let b = store.stat_blob(last_key(space)).ok().flatten()?;
    let whole: [u8; 32] = match b.len() {
        32 => b.try_into().ok()?,
        33 if b[32] == UNDER_WAY => b[..32].try_into().ok()?,
        _ => return None,
    };
    (whole != [0; 32]).then_some(whole)
}

/// `last_program`'s mark of a run of `whole` under way that started from what an ended run of it kept.
const UNDER_WAY: u8 = 1;

/// Records `whole` as `last_program` of `space`, queued behind the run's writes; `None`: a run is under way that
/// did not start from what is kept (another program's, or one after it): a run that stops leaves none of it valid.
pub(super) fn set_last_program(store: &ContentStore, space: &[u8; 32], whole: Option<[u8; 32]>) {
    store.queue_stat_blob(last_key(space), whole.unwrap_or([0; 32]).to_vec());
}

/// Records a run of `whole` under way in `space`, one that started from what an ended run of `whole` kept: if it
/// stops, what is kept stays valid for the next (Q71). Its entries are of `whole` and of the contents it read, and the
/// contents the kept statistics count are still in the memo (its newest `MEMO_CONTENTS` of a path); a content whose
/// entry went gives the keeper up, as an entry dropped otherwise does.
pub(super) fn set_under_way(store: &ContentStore, space: &[u8; 32], whole: [u8; 32]) {
    let mut b = whole.to_vec();
    b.push(UNDER_WAY);
    store.queue_stat_blob(last_key(space), b);
}

/// The run's plan: the engine build, the grammar version and `program` (a digest of the file-local part as lowered,
/// its atoms included).
pub(super) fn plan_digest(program: &[u8; 32]) -> [u8; 32] {
    let mut hash = Sha256::new();
    // v5: an entry is one image in a pack (0240 §4), its outputs' rows typed, not their lines (0239).
    hash.update(b"rerail.corpus-local/v5\0");
    hash.update(crate::code_rule_dsl::build_ids::engine().as_bytes());
    hash.update(crate::code_rule_dsl::build_ids::grammar_version().to_le_bytes());
    // A memoised row's node ids are those of the scheme it was made under: preorder, or per definition.
    if crate::code_rule_dsl::facts::vtab::syntax::definition_ids() {
        hash.update(b"node-ids:definition\0");
    }
    hash.update(program);
    hash.finalize().into()
}

/// A digest of a program part: its rules, declarations, inputs and outputs as lowered, every fact, those the engine
/// reads (a guest policy) as well as those its rules read, and the spans and patterns the batch matches for it, all
/// of which its version's image holds. The version's digest (decision 0173 §7), kept as the version was made: the
/// key reads no rule. A sum, so it does not see the rules' order, which a part's rows do not depend on.
pub(in crate::code_rule_dsl) fn program_digest(v: &crate::code_rule_dsl::logic::program_image::Version) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"rerail.corpus-local-program/v4\0");
    hash.update(v.digest().to_le_bytes());
    let digest: [u8; 32] = hash.finalize().into();
    // `RERAIL_MEMO_PROGRAM_DUMP=dir`: what the digest is of, a file a digest (to find what moved a memo's key).
    if let Some(dir) = std::env::var_os("RERAIL_MEMO_PROGRAM_DUMP") {
        let name: String = digest[..6].iter().map(|b| format!("{b:02x}")).collect();
        let text = match v.program(&crate::code_rule_dsl::logic::front::Lists::default()) {
            Ok(p) => format!(
                "rules {:#?}\ndecls {:#?}\nfacts {:#?}\ninputs {:#?}\noutputs {:#?}\nrx_spans {:#?}\nast_patterns {:#?}\n",
                p.rules, p.decls, p.facts, p.inputs, p.outputs, p.rx_spans, p.ast_patterns
            ),
            Err(e) => format!("no program: {e}\n"),
        };
        let _ = std::fs::write(std::path::Path::new(&dir).join(format!("{name}.txt")), text);
    }
    digest
}

/// A local memo's stored statistics are a projection demanded by its consumers.
/// Include the projection in the key so a wider consumer cannot reuse a narrower frame.
pub(super) fn with_tree_demand(digest: [u8; 32], tree: bool) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"rerail.corpus-local-tree-demand/v1\0");
    hash.update(digest);
    hash.update([u8::from(tree)]);
    hash.finalize().into()
}

/// `digest` with the modules named by package roots, not paths, folded in (`corpus::package_root_modules`): a file's
/// `doc` row names its module, which an `__init__.py` added or removed elsewhere in the tree changes. None leaves the
/// digest as it was.
pub(super) fn with_modules(digest: [u8; 32], modules: &BTreeMap<String, String>) -> [u8; 32] {
    if modules.is_empty() {
        return digest;
    }
    let mut hash = Sha256::new();
    hash.update(b"rerail.corpus-local-modules/v1\0");
    hash.update(digest);
    for (path, module) in modules {
        hash.update(path.as_bytes());
        hash.update(b"\0");
        hash.update(module.as_bytes());
        hash.update(b"\0");
    }
    hash.finalize().into()
}

/// `digest` with the run's options folded in: the key of a file-local part that reads them (`option_item`).
pub(super) fn with_options(digest: [u8; 32], options: &serde_json::Value) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"rerail.corpus-local-options/v1\0");
    hash.update(digest);
    hash.update(options.to_string().as_bytes());
    hash.finalize().into()
}

/// `digest` with rows folded in, by relation: the key of a file-local part that reads relations whose rows are not in
/// its text (a phase's exports). The rows' order is not part of it.
pub(super) fn with_rows<'a>(digest: [u8; 32], rels: impl Iterator<Item = (&'a str, &'a Vec<Vec<crate::code_rule_dsl::logic::front::Value>>)>) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"rerail.corpus-local-rows/v1\0");
    hash.update(digest);
    for (rel, rows) in rels {
        let mut lines: Vec<String> = rows.iter().map(|r| format!("{r:?}")).collect();
        lines.sort_unstable();
        hash.update(rel.as_bytes());
        hash.update([0]);
        for l in &lines {
            hash.update(l.as_bytes());
            hash.update([0]);
        }
    }
    hash.finalize().into()
}

/// The memo is the tape cache's: a build without `tape-cache` has no store to keep it in, so the runner never reads or
/// writes it, and saying so here, where the compiler sees it, leaves the store's zstd frames unlinked.
pub(super) fn off() {
    if !cfg!(feature = "tape-cache") {
        unreachable!("the corpus memo is off in a build without `tape-cache`");
    }
}

/// The stored source of `text`, stored if it is not.
pub(super) fn source(store: &ContentStore, text: &str) -> StoreResult<SourceId> {
    off();
    let hash = HashKey::sha256(text.as_bytes());
    // Held (O95): a source the run finds is pinned with it, as one it stores is.
    match store.source_by_hash(hash)? {
        Some(id) => Ok(id),
        None => store.put_source_hashed(text.as_bytes(), hash, &[]),
    }
}

/// The source of `text` when the store holds it: one read, nothing written (the run's collection lease keeps it until
/// the manifest names it, decision 0306 step 4). `None`: not stored yet; `source`, run on the writer, stores it.
pub(super) fn stored_source(store: &ContentStore, text: &str) -> StoreResult<Option<SourceId>> {
    off();
    Ok(store.sources_by_hashes(&[HashKey::sha256(text.as_bytes())])?.pop().flatten())
}

/// The source of `text` for the run's manifest (`late`, which the manifest reads as the writer runs it), all of it
/// on the store's writer, behind the run (decision 0104): the source the store holds then, pinned, or, where `store`,
/// one it stores. Queued after a write that stores it (FIFO), it finds that one. `stored_by`: what that write is
/// queued under (`write_back::Key`), where only it stores the text.
pub(super) fn named_source(cache: &ContentStore, text: String, store: bool, stored_by: Option<&crate::content_store::write_back::Key>, late: &std::sync::Arc<std::sync::Mutex<Vec<SourceId>>>) {
    // Stored already (decision 0228): named now, and only its pin queued (a lock, as `stored_source` takes it). The
    // writer is for what a queued write stores: the lookup there was a write a warm run made.
    if let Some(id) = stored_source(cache, &text).ok().flatten() {
        late.lock().expect("the late sources").push(id);
        return;
    }
    // Not stored, and nothing queued stores it: the writer would find nothing either.
    if !store && stored_by.is_some_and(|key| !cache.queued(key)) {
        return;
    }
    // In the run's record (decision 0306): found, or stored, as the writer applies it.
    fn apply(cx: &crate::content_store::record::Apply<'_>, body: crate::content_store::record::codec::Body) -> StoreResult<()> {
        let (text, store): (String, bool) = crate::content_store::record::codec::from_body("a source the manifest names", body)?;
        let hash = HashKey::sha256(text.as_bytes());
        let found = match cx.store.source_by_hash(hash)? {
            Some(id) => Some(id),
            None if store => Some(cx.store.put_source_hashed(text.as_bytes(), hash, &[])?),
            None => None,
        };
        cx.name(found);
        Ok(())
    }
    cache.record("a source the manifest names", Vec::new(), &(text, store), None, apply);
}

/// The sources of contents the run named by their digests (`Content::Unstored`: read in the waves, where the store is
/// not asked, by files the stat index does not trust yet), for the run's manifest: looked up together in the run's
/// record (decision 0306), behind the writes queued before (a new content's tape stores its source), and named there
/// (`Apply::name`), kept from collection by the run's lease until the manifest names them (0306 step 4). A second run
/// in one process finds those files' tapes held and writes none, so only this names them (Q24).
pub(super) fn named_digests(cache: &ContentStore, digests: Vec<[u8; 32]>) {
    if digests.is_empty() {
        return;
    }
    fn apply(cx: &crate::content_store::record::Apply<'_>, body: crate::content_store::record::codec::Body) -> StoreResult<()> {
        let digests: Vec<[u8; 32]> = crate::content_store::record::codec::from_body("the sources the manifest names by digest", body)?;
        let hashes: Vec<HashKey> = digests.iter().map(|d| HashKey::from_sha256_digest(*d)).collect();
        let found: Vec<SourceId> = cx.store.sources_by_hashes(&hashes)?.into_iter().flatten().collect();
        cx.name(found);
        Ok(())
    }
    cache.record("the sources the manifest names by digest", Vec::new(), &digests, None, apply);
}

/// How many contents of one path the memo keeps (decision 0117): the newest, by their last use. Worktrees or
/// branches that take turns at a file keep theirs (decision 0146 §4); an edited file's earlier contents go.
const MEMO_CONTENTS: usize = 4;

/// A memo plan's symbol columns, by relation (decision 0170): its program's columns that hold atoms, by their domains
/// (`Name`). Made once by the run that reads or writes its entries, and carried with its program (`Program`).
///
/// Not its declaration's `sym` columns: a kind's or a field's is a `sym` that holds the grammar's code
/// (`core::kind_atom`, `syntax::field_atom`: `node`'s `kind` 1132, `field`'s `f` 65555), not an atom, and so is a
/// derived relation's column the domains find holds one. Taken as atoms, a forked process would map those codes as
/// this process's ids, and the check where an entry is made refused it (`symbol 65555 is no atom of this process's`).
#[derive(Clone, Default)]
pub(super) struct Syms(std::sync::Arc<HashMap<String, Vec<bool>>>);

impl Syms {
    pub(super) fn of(domains: &super::super::lower::Domains) -> Syms {
        Syms(std::sync::Arc::new(domains.iter().map(|(r, cols)| (r.clone(), cols.iter().map(|d| matches!(d, super::super::lower::Domain::Name)).collect())).collect()))
    }

    /// The symbol columns of an entry's rows under `key` (`Entry::rows`), `width` wide: a relation's own, a base
    /// relation's (`base:`); the `own:` rows hold none. None for a relation the plan does not declare.
    ///
    /// A posting (`own:postings:rel:col`) is a count of a program's integer constant (`Own::constants`) in a column,
    /// as the column's source encodes it: a kind's or a field's is the grammar's code (`field`'s `f`, 65555), not an
    /// atom, whatever the column is declared. Its value is no symbol, and is not mapped: a forked process's counts for
    /// a constant that is an atom are of its numbering (estimates, as the kept statistics' sketches are). Read as one,
    /// the posting's column was looked up by number, found none, and a forked run refused every entry holding one.
    /// `columns`, for statistics kept at the columns `names` (a base relation's: some of its columns, by name).
    pub(super) fn columns_named(&self, key: &str, names: &[String]) -> Option<Vec<bool>> {
        let Some(rel) = key.strip_prefix("base:") else { return self.columns(key, names.len()) };
        let (all, syms) = (crate::code_rule_dsl::logic::sql::select::input_columns(rel)?, self.0.get(rel)?);
        names.iter().map(|n| all.iter().position(|c| c == n).and_then(|i| syms.get(i).copied())).collect()
    }

    pub(super) fn columns(&self, key: &str, width: usize) -> Option<Vec<bool>> {
        if let Some(rel) = key.strip_prefix("base:") {
            return self.0.get(rel).cloned();
        }
        if key.starts_with("own:") {
            return Some(vec![false; width]);
        }
        self.0.get(key).cloned()
    }

}

/// A memo's program (its key, `plan_digest`'s input) with its symbol columns: what reads and writes its entries.
#[derive(Clone)]
pub(super) struct Program {
    pub(super) id: [u8; 32],
    pub(super) syms: Syms,
}

impl Program {
    pub(super) fn new(id: [u8; 32], domains: &super::super::lower::Domains) -> Program {
        Program { id, syms: Syms::of(domains) }
    }
}

impl std::ops::Deref for Program {
    type Target = [u8; 32];
    fn deref(&self) -> &[u8; 32] {
        &self.id
    }
}

/// The first symbol of `entries`' rows (by a memo plan's columns, `syms`) that is no atom of an arena of `atoms` atoms:
/// its key and value. An entry is made of the run's rows, so each symbol in it is this process's; one past its atoms
/// came from a read that gave the store's ids unmapped (decision 0170), and is caught here, where the entry is made,
/// not by the writer mapping it later.
fn foreign_symbol(entries: &[&Entry], syms: &Syms, atoms: usize) -> Option<(String, i64)> {
    for entry in entries {
        for (key, (w, rows)) in &entry.rows {
            let Some(cols) = syms.columns(key, *w) else { continue };
            let at: Vec<usize> = (0..(*w).min(cols.len())).filter(|&c| cols[c]).collect();
            if at.is_empty() || *w == 0 {
                continue;
            }
            if let Some(v) = rows.chunks_exact(*w).flat_map(|row| at.iter().map(move |&c| row[c])).find(|&v| v < 0 || v as u64 >= atoms as u64) {
                return Some((key.clone(), v));
            }
        }
    }
    None
}

/// The part an entry of content `digest` at `path` is kept under (0103 §8's `local_key(d, h, p)`, its `p` the space).
fn part_of(digest: &[u8; 32], path: &str) -> i64 {
    let mut hash = Sha256::new();
    hash.update(b"rerail.local-key/v1\0");
    hash.update(digest);
    hash.update(path.as_bytes());
    let h: [u8; 32] = hash.finalize().into();
    i64::from_le_bytes(h[..8].try_into().expect("8 bytes")) & i64::MAX
}

/// A memo entry as stored, not decoded: the document's path and key, its content's digest, and its bytes.
#[derive(Clone)]
pub struct Frame {
    pub path: String,
    key: i64,
    digest: [u8; 32],
    blob: std::sync::Arc<crate::content_store::MemoBlob>,
    /// The atoms its symbols are read as: its reader's store's.
    atoms: std::sync::Arc<crate::content_store::atoms::Atoms>,
}

impl Frame {
    fn of(blob: std::sync::Arc<crate::content_store::MemoBlob>, atoms: std::sync::Arc<crate::content_store::atoms::Atoms>) -> Frame {
        Frame { path: blob.path.clone(), key: blob.key, digest: blob.digest, blob, atoms }
    }

    fn entry(&self) -> StoreResult<Entry> {
        self.projected(&|_| true)
    }

    fn projected(&self, keep: &dyn Fn(&str) -> bool) -> StoreResult<Entry> {
        let mut entry = Entry { key: self.key, ..image::decode(&self.blob.bytes, keep, &self.atoms)? };
        // The identities a later run found its rows hold (decision 0280), kept over it: with those it was made under,
        // all true while it stands. Bytes kept over it that do not decode add none: the relations are a partial hit's.
        if let Some(bytes) = self.blob.over.as_ref().filter(|_| keep(COVERED))
            && let Ok(more) = image::decode(bytes, &|k| k == COVERED, &self.atoms)
        {
            let held: Vec<[i64; 2]> = covered(&more).into_iter().collect();
            if !held.is_empty() {
                let mut all: Vec<[i64; 2]> = covered(&entry).into_iter().chain(held).collect();
                all.sort_unstable();
                all.dedup();
                entry.rows.insert(COVERED.to_owned(), (2, all.into_iter().flatten().collect()));
            }
        }
        Ok(entry)
    }
}

/// Holds the atoms an entry's symbols name (`image::hold`), as it is read before a run's waves: none of an entry whose
/// symbols this process holds already. The profile says how many it read.
pub(super) fn hold<'a>(store: &ContentStore, blobs: impl IntoIterator<Item = &'a crate::content_store::MemoBlob>) -> StoreResult<()> {
    let bytes: Vec<_> = blobs.into_iter().map(|b| &b.bytes).collect();
    if bytes.is_empty() {
        return Ok(());
    }
    let read = image::hold(&bytes, &store.atoms_handle())?;
    crate::code_rule_dsl::prof::count("corpus.memo_entries_held_read", read);
    crate::code_rule_dsl::logic::profile::decision("the memo's entries' atoms, before the first wave (decision 0397)", || {
        format!("{read} of {} entries read; the others' symbols held already", bytes.len())
    });
    Ok(())
}

/// The newest corpus run of `root`: the relations its memo plan holds, by name, as recorded (`.decl` columns as
/// JSON), and its documents' frames, not decoded, by path. A frame's document key is `frame_key`, its entry
/// `decode_frame`.
pub fn read_run(store: &ContentStore, root: &str) -> StoreResult<Option<(BTreeMap<String, String>, Vec<Frame>)>> {
    let Some((plan, sources, relations)) = store.corpus_run_of(root)? else { return Ok(None) };
    let ids: Vec<SourceId> = sources.into_iter().map(SourceId::from_raw).collect();
    let contents: HashSet<[u8; 32]> = store.source_digests(&ids)?.into_iter().flatten().collect();
    let atoms = store.atoms_handle();
    let mut frames: Vec<Frame> = store
        .all_memo_entries(&crate::content_store::local_space(&plan))?
        .into_values()
        .map(|b| Frame::of(b, std::sync::Arc::clone(&atoms)))
        .filter(|f| contents.contains(&f.digest))
        .collect();
    frames.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(Some((relations.into_iter().collect(), frames)))
}

/// The document key of a frame's entry.
pub fn frame_key(frame: &Frame) -> StoreResult<i64> {
    Ok(frame.key)
}

/// A frame's entry.
pub fn decode_frame(frame: &Frame) -> StoreResult<Entry> {
    frame.entry()
}

/// The memoised entry of the document at `path` (key `key`, content `source`) under `program`.
pub(super) fn lookup(store: &ContentStore, source: SourceId, path: &str, key: i64, program: &Program) -> StoreResult<Option<Entry>> {
    off();
    match found_all(store, program, &[(source, path)])?.pop().flatten() {
        Some(found) => found.entry(key),
        None => Ok(None),
    }
}

/// `lookup` of many documents, each `(source, path, key)`, in their order: one statement whatever their number.
pub(super) fn lookup_all(store: &ContentStore, program: &Program, docs: &[(SourceId, &str, i64)]) -> StoreResult<Vec<Option<Entry>>> {
    off();
    let keys: Vec<(SourceId, &str)> = docs.iter().map(|(s, p, _)| (*s, *p)).collect();
    found_all(store, program, &keys)?.into_iter().zip(docs).map(|(f, (_, _, key))| f.map_or(Ok(None), |f| f.entry(*key))).collect()
}

/// A memo entry found before it is decoded: held in memory (a queued write's, or a session's), or its stored frame.
/// A run finds its documents' entries before its first wave and decodes each in its wave (decision 0104).
#[derive(Clone)]
pub(super) enum Found {
    Held(std::sync::Arc<Entry>),
    Frame(Frame),
}

impl Found {
    /// The entry, when it is the document's (`key`).
    pub(super) fn entry(&self, key: i64) -> StoreResult<Option<Entry>> {
        self.projected(key, &|_| true)
    }

    /// The whole entry this was found for: an entry is found whole (decision 0240 §4: no frame of its outputs alone).
    pub(super) fn whole(&self, _store: &ContentStore, _program: &Program) -> StoreResult<Option<Found>> {
        Ok(Some(self.clone()))
    }

    /// Decode only demanded relations. Skipped relations' rows are checked, not kept.
    pub(super) fn projected(&self, key: i64, keep: &dyn Fn(&str) -> bool) -> StoreResult<Option<Entry>> {
        let entry = match self {
            Found::Held(e) => Entry { key: e.key, nodes: e.nodes,
                rows: e.rows.iter().filter(|(r, _)| keep(r)).map(|(r, v)| (r.clone(), v.clone())).collect(),
                outputs: e.outputs.clone() },
            Found::Frame(f) => f.projected(keep)?,
        };
        Ok(Some(entry).filter(|e| e.key == key))
    }

}

/// `lookup_all`, each entry found and not decoded: the contents' digests in one read, their frames in one more. A
/// lookup writes nothing, and marks nothing (decisions 0104, 0227).
pub(super) fn found_all(store: &ContentStore, program: &Program, docs: &[(SourceId, &str)]) -> StoreResult<Vec<Option<Found>>> {
    off();
    let atoms = store.atoms_handle();
    let held: Vec<Option<Found>> = docs.iter().map(|(s, p)| pending(store, &queued(*s, p, program), &atoms)).collect();
    let asked: Vec<usize> = (0..docs.len()).filter(|i| held[*i].is_none()).collect();
    let digests = {
        let _s = crate::code_rule_dsl::logic::profile::span(|| "memo lookup: the contents' digests".into());
        store.source_digests(&asked.iter().map(|i| docs[*i].0).collect::<Vec<_>>())?
    };
    let wanted: Vec<(usize, [u8; 32])> = asked.iter().zip(&digests).filter_map(|(i, d)| d.map(|d| (*i, d))).collect();
    let paths: Vec<&str> = docs.iter().map(|(_, p)| *p).collect();
    let mut out: Vec<Option<Found>> = held;
    for (i, f) in by_digest(store, program, &paths, wanted)? {
        out[i] = Some(f);
    }
    Ok(out)
}

/// The entries of `wanted`, each `(at, content digest)` of the document at `paths[at]`: an entry's part is its
/// content's and path's, and the entry names both. No source is read. Before the waves, from the store (the
/// snapshot holds a run's entries only for what its waves look up: `admit_memo_entries`); in them, from the
/// snapshot, in memory. A hit marks nothing (decision 0227): an entry is kept while its plan is live and a kept
/// manifest names its content (`collect_unreachable_frames`), not by the runs that read it.
fn by_digest(store: &ContentStore, program: &Program, paths: &[&str], wanted: Vec<(usize, [u8; 32])>) -> StoreResult<Vec<(usize, Found)>> {
    let stored = !crate::code_rule_dsl::logic::profile::inside_waves();
    let space = crate::content_store::local_space(&plan_digest(program));
    let wanted: Vec<(usize, [u8; 32], i64)> = wanted.into_iter().map(|(i, d)| (i, d, part_of(&d, paths[i]))).collect();
    let parts: Vec<i64> = wanted.iter().map(|(_, _, p)| *p).collect();
    let mut blobs = {
        let _s = crate::code_rule_dsl::logic::profile::span(|| "memo lookup: the entries read".into());
        store.memo_entries(&space, &parts, stored)?
    };
    if stored {
        let _s = crate::code_rule_dsl::logic::profile::span(|| "memo lookup: the entries' atoms held".into());
        hold(store, blobs.values().map(|b| &**b))?;
    }
    let atoms = store.atoms_handle();
    let mut out = Vec::new();
    for (i, digest, part) in wanted {
        let Some(blob) = blobs.remove(&part) else { continue };
        // A part another content or path collides into is a miss.
        if blob.digest == digest && blob.path == paths[i] {
            out.push((i, Found::Frame(Frame::of(blob, std::sync::Arc::clone(&atoms)))));
        }
    }
    Ok(out)
}

/// A queued entry, as its write left it for this process's readers: its image in the segment the run wrote.
fn pending(store: &ContentStore, key: &crate::content_store::write_back::Key, atoms: &std::sync::Arc<crate::content_store::atoms::Atoms>) -> Option<Found> {
    store.pending::<crate::content_store::MemoBlob>(key).map(|blob| Found::Frame(Frame::of(blob, std::sync::Arc::clone(atoms))))
}

/// What a memo entry's write is queued under (`write_back`): the row's source, path and program.
fn queued(source: SourceId, path: &str, program: &[u8; 32]) -> crate::content_store::write_back::Key {
    crate::content_store::write_back::Key::of(&[b"facts_memo/corpus", &source.raw().to_le_bytes(), path.as_bytes(), program])
}

/// `queued`, for a content the store does not hold yet: by its digest.
fn queued_unstored(digest: &[u8; 32], path: &str, program: &[u8; 32]) -> crate::content_store::write_back::Key {
    crate::content_store::write_back::Key::of(&[b"facts_memo/corpus/unstored", digest, path.as_bytes(), program])
}

/// What an entry is queued under, by its content.
fn queued_content(content: Content, path: &str, program: &[u8; 32]) -> Option<crate::content_store::write_back::Key> {
    match content {
        Content::Stored(id) => Some(queued(id, path, program)),
        Content::Unstored(d) => Some(queued_unstored(&d, path, program)),
        Content::Held(_) => None,
    }
}

/// `source` of many texts, in their order: the stored ones found (and pinned) in one statement, the others stored.
pub(super) fn sources(store: &ContentStore, texts: &[&str]) -> StoreResult<Vec<SourceId>> {
    off();
    let hashes: Vec<HashKey> = texts.iter().map(|t| HashKey::sha256(t.as_bytes())).collect();
    let found = store.sources_by_hashes(&hashes)?;
    texts.iter().zip(hashes).zip(found).map(|((text, hash), id)| match id { Some(id) => Ok(id), None => store.put_source_hashed(text.as_bytes(), hash, &[]) }).collect()
}

/// The stored sources of many contents, by their SHA-256 digests, in their order: one read and nothing written (a
/// lookup writes nothing, decision 0104; the run's collection lease keeps them, decision 0306 step 4).
pub(super) fn found_sources(store: &ContentStore, digests: &[[u8; 32]]) -> StoreResult<Vec<Option<SourceId>>> {
    off();
    let hashes: Vec<HashKey> = digests.iter().map(|d| HashKey::from_sha256_digest(*d)).collect();
    store.sources_by_hashes(&hashes)
}

/// Memoises many documents' entries at once, each `(content, path, key, entry)` under `program`: their bytes in one
/// append to the store's pack and their rows in one transaction (decision 0240 §4). Queued behind the run (decision 0102): encoded and written by the store's
/// writer, each entry read from memory until it is written. A content is keyed by its digest, so one the store
/// does not hold yet is written with the others.
pub(super) fn remember_all(store: &ContentStore, program: &Program, entries: Vec<(Content, String, i64, Entry)>) -> StoreResult<()> {
    remember_all_but(store, program, entries.into_iter().map(|(c, p, k, e)| (c, p, k, e, false, None)).collect())
}

/// `remember_all`, each entry with whether its rows stand as stored (decision 0225 §2): its relations were derived
/// again and found as they were, so only the identities they now hold are written, kept over the stored entry
/// (decision 0280). Each with its content's digest where the run has it: in the waves, a stored source's digest
/// is not read (decision 0104).
pub(super) fn remember_all_but(store: &ContentStore, program: &Program, entries: Vec<(Content, String, i64, Entry, bool, Option<[u8; 32]>)>) -> StoreResult<()> {
    let standing: HashSet<(String, i64)> = entries.iter().filter(|e| e.4).map(|e| (e.1.clone(), e.2)).collect();
    let known: HashMap<String, [u8; 32]> = entries.iter().filter_map(|e| Some((e.1.clone(), e.5?))).collect();
    let entries: Vec<(Content, String, i64, Entry)> = entries.into_iter().map(|(c, p, k, e, _, _)| (c, p, k, e)).collect();
    off();
    let plan = plan_digest(program);
    // The atom table the entries' symbols are numbered by: the writer reads their strings from it.
    let made_under = store.atoms_handle();
    let atoms = made_under.len();
    if let Some((key, v)) = foreign_symbol(&entries.iter().map(|e| &e.3).collect::<Vec<_>>(), &program.syms, atoms) {
        return Err(StoreError::Invalid(format!("the memo's entry for {key}: symbol {v} is no atom of this process's (it holds {atoms}): a read gave the store's ids unmapped")));
    }
    if crate::code_rule_dsl::logic::profile::on() {
        let _t = crate::code_rule_dsl::logic::profile::span(|| "tally the entries (profiling)".into());
        tally(entries.iter().map(|e| &e.3));
    }
    let _queue = crate::code_rule_dsl::logic::profile::span(|| "write the entries into their segments".into());
    let held: Vec<(Content, String, Entry)> = entries.into_iter().filter(|e| e.0.in_store()).map(|(content, path, key, entry)| (content, path, Entry { key, ..entry })).collect();
    // Nothing to keep is no job: an empty one still takes the writer's transaction (`RERAIL_WARM`).
    if held.is_empty() {
        return Ok(());
    }
    let estimate: usize = held.iter().map(|(_, path, e)| if standing.contains(&(path.clone(), e.key)) { identities_of(e).bytes() } else { e.bytes() } * 2 + 4096).sum();
    if estimate > 64 << 20 && crate::code_rule_dsl::logic::profile::on() {
        let mut by: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
        for (_, _, e) in &held {
            for (r, (w, v)) in &e.rows {
                let at = by.entry(r.as_str()).or_default();
                at.0 += v.len() / (*w).max(1);
                at.1 += v.len() * 8;
            }
            for (r, (_, rows)) in &e.outputs {
                by.entry(r.as_str()).or_default().1 += rows.bytes();
            }
        }
        let mut by: Vec<_> = by.into_iter().collect();
        by.sort_by(|a, b| b.1.1.cmp(&a.1.1));
        let top: Vec<String> = by.iter().take(12).map(|(r, (n, b))| format!("{r}: {n} rows, {:.1} MB", *b as f64 / 1048576.0)).collect();
        eprintln!("rerail: the memo's entries: {:.1} MB over {} documents; by relation: {}", estimate as f64 / 2097152.0, held.len(), top.join("; "));
    }
    // Each content's digest, which names its entry's part: the run's, else a stored source's read, in one statement
    // (before the waves only). A source collected since the run found it leaves the entry unwritten: the next run
    // misses it.
    let ids: Vec<SourceId> = held.iter().filter(|(_, p, _)| !known.contains_key(p)).filter_map(|(c, _, _)| match c { Content::Stored(id) => Some(*id), _ => None }).collect();
    let mut stored = store.source_digests(&ids)?.into_iter();
    let digests: Vec<Option<[u8; 32]>> = held
        .iter()
        .map(|(c, p, _)| match c {
            Content::Stored(_) => match known.get(p) {
                Some(d) => Some(*d),
                None => stored.next().flatten(),
            },
            Content::Unstored(d) => Some(*d),
            Content::Held(_) => None,
        })
        .collect();
    // Written here, by the run (decision 0295 §1): each entry's image made into the segment's mapped pages, its
    // symbols as their strings, read from the table it was made under (no numbering to map, forked or not). Of an
    // entry whose rows stand, only the identities it now holds, kept over the stored entry (decision 0280). The
    // decoded entries go once written: the run's readers of a queued entry read its image (`Found::Frame`).
    let mut w = crate::content_store::SegmentWriter::new(&store.memo_dir(), digests.iter().flatten().count(), estimate);
    let mut rows: Vec<crate::content_store::MemoRow> = Vec::with_capacity(held.len());
    let mut contents: Vec<Content> = Vec::with_capacity(held.len());
    for ((content, path, e), digest) in held.into_iter().zip(digests) {
        let Some(digest) = digest else { continue };
        let over = standing.contains(&(path.clone(), e.key));
        let key = e.key;
        let image = if over { image::encode_parts(&identities_of(&e), &program.syms, &made_under)? } else { image::encode_parts(&e, &program.syms, &made_under)? };
        drop(e);
        w.put(image.len(), key, &digest, |out| image.write(out))?;
        rows.push(crate::content_store::MemoRow { part: part_of(&digest, &path), path, digest, key, over });
        contents.push(content);
    }
    let segments = w.seal();
    if rows.is_empty() {
        return Ok(());
    }
    // What a queued entry's readers find (`pending`): its image where it is. Of an entry kept over the stored one,
    // the stored entry with the new identities over it, as the store reads it once written; where the stored entry
    // is not found (collected meanwhile), nothing: the write is dropped, and a reader misses.
    let space = crate::content_store::local_space(&plan);
    let over_parts: Vec<i64> = rows.iter().filter(|r| r.over).map(|r| r.part).collect();
    let mut under = if over_parts.is_empty() { HashMap::new() } else { store.memo_entries(&space, &over_parts, !crate::code_rule_dsl::logic::profile::inside_waves())? };
    let keys: Vec<(crate::content_store::write_back::Key, Option<std::sync::Arc<dyn std::any::Any + Send + Sync>>)> = rows
        .iter()
        .zip(&contents)
        .enumerate()
        .filter_map(|(i, (r, c))| {
            let bytes = segments.entry(i);
            let blob = if !r.over {
                Some(crate::content_store::MemoBlob { path: r.path.clone(), digest: r.digest, key: r.key, bytes, over: None })
            } else {
                under.remove(&r.part).filter(|b| b.digest == r.digest && b.key == r.key && b.path == r.path).map(|b| crate::content_store::MemoBlob { path: b.path.clone(), digest: b.digest, key: b.key, bytes: b.bytes.clone(), over: Some(bytes) })
            };
            Some((queued_content(*c, &r.path, program)?, blob.map(|b| std::sync::Arc::new(b) as std::sync::Arc<dyn std::any::Any + Send + Sync>)))
        })
        .collect();
    // The queue's measure: what the job holds in memory, its rows; the images are the OS's pages.
    let bytes = rows.iter().map(|r| r.path.len() + 96).sum();
    // A path's earlier contents: its newest `MEMO_CONTENTS` kept (an edited file's entry each edit, 15 MB for one file
    // of the 20, was kept 32 runs: 450 MB of one space).
    store.queue_memo_segments("the memo's entries", bytes, keys, space, segments, rows, MEMO_CONTENTS);
    Ok(())
}

impl Entry {
    /// The bytes the entry holds (its rows and lines), about.
    fn bytes(&self) -> usize {
        self.rows.iter().map(|(r, (_, v))| r.len() + v.len() * 8).sum::<usize>() + self.outputs.iter().map(|(r, (c, rows))| r.len() + c.len() * 8 + rows.bytes()).sum::<usize>()
    }
}

/// What the run's memo entries held, by relation (rows, values), and the documents' nodes: counted when profiling,
/// so the profile names what makes the memo large (t31 A2). Read and cleared by `sizes`.
static SIZES: std::sync::Mutex<(u64, BTreeMap<String, (u64, u64)>)> = std::sync::Mutex::new((0, BTreeMap::new()));

fn tally<'a>(entries: impl Iterator<Item = &'a Entry>) {
    let mut sizes = SIZES.lock().expect("the memo's sizes");
    for entry in entries {
        sizes.0 += entry.nodes;
        for (rel, (width, values)) in &entry.rows {
            let at = sizes.1.entry(rel.clone()).or_default();
            at.0 += (values.len() / (*width).max(1)) as u64;
            at.1 += values.len() as u64;
        }
    }
}

/// The nodes of the documents remembered, and each relation's (rows, values) over their entries, since the last call.
pub(super) fn sizes() -> (u64, BTreeMap<String, (u64, u64)>) {
    std::mem::take(&mut *SIZES.lock().expect("the memo's sizes"))
}

/// Where a run's memo is: the tape cache's store (decision 0052), or a session's memory (`session::Session`), whose
/// rows are under the session store's atoms and live as long as the session.
#[derive(Clone, Copy)]
pub(super) enum Memo<'a> {
    Stored(&'a ContentStore),
    Held(&'a Held),
}

/// A session's memo: each entry under its content's digest, its path and the file-local program's digest.
pub(super) type Held = std::sync::Mutex<std::collections::HashMap<([u8; 32], String, [u8; 32]), Entry>>;

/// A file's content as the memo knows it: its stored source; a content the store does not hold yet, by its digest
/// (its source is written by the store's writer, behind the run, with its tape: decision 0104); or (held) its digest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(in crate::code_rule_dsl) enum Content {
    Stored(SourceId),
    Unstored([u8; 32]),
    Held([u8; 32]),
}

impl Content {
    /// The stored source, which a run's manifest names (decision 0053).
    pub(super) fn stored(self) -> Option<SourceId> {
        match self {
            Content::Stored(id) => Some(id),
            Content::Unstored(_) | Content::Held(_) => None,
        }
    }

    /// Whether the store holds the content or will (its source is queued): a stored memo's content.
    fn in_store(self) -> bool {
        matches!(self, Content::Stored(_) | Content::Unstored(_))
    }

    /// The source on the store's writer, where a content queued before is written by now.
    pub(super) fn resolve(self, store: &ContentStore) -> StoreResult<Option<SourceId>> {
        match self {
            Content::Stored(id) => Ok(Some(id)),
            Content::Unstored(d) => store.source_by_hash(HashKey::from_sha256_digest(d)),
            Content::Held(_) => Ok(None),
        }
    }
}

/// The most entries a session holds: past it, the memo starts again empty (its entries are a cache).
const HELD_ENTRIES: usize = 4096;

impl Memo<'_> {
    /// `text` as the memo knows it.
    pub(super) fn content(self, text: &str) -> StoreResult<Content> {
        match self {
            Memo::Stored(store) => source(store, text).map(Content::Stored),
            Memo::Held(_) => Ok(Content::Held(Sha256::digest(text.as_bytes()).into())),
        }
    }

    /// `content` of many texts, in their order (`sources`).
    pub(super) fn contents(self, texts: &[&str]) -> StoreResult<Vec<Content>> {
        match self {
            Memo::Stored(store) => Ok(sources(store, texts)?.into_iter().map(Content::Stored).collect()),
            Memo::Held(_) => Ok(texts.iter().map(|t| Content::Held(Sha256::digest(t.as_bytes()).into())).collect()),
        }
    }

    /// The contents of many texts as the memo knows them, by their SHA-256 digests, in their order: `None` for a
    /// text the store does not hold yet (its source is stored with its tape). Nothing is written (`found_sources`).
    pub(super) fn found(self, digests: &[[u8; 32]]) -> StoreResult<Vec<Option<Content>>> {
        match self {
            Memo::Stored(store) => Ok(found_sources(store, digests)?.into_iter().map(|id| id.map(Content::Stored)).collect()),
            Memo::Held(_) => Ok(digests.iter().map(|d| Some(Content::Held(*d))).collect()),
        }
    }

    /// `lookup` of many documents, each `(content, path, key)`, in their order: stored ones in one statement.
    pub(super) fn lookup_all(self, program: &Program, docs: &[(Content, &str, i64)]) -> StoreResult<Vec<Option<Entry>>> {
        match self {
            Memo::Stored(store) => {
                let mut out: Vec<Option<Entry>> = vec![None; docs.len()];
                let (at, stored): (Vec<usize>, Vec<(SourceId, &str, i64)>) = docs.iter().enumerate().filter_map(|(i, (c, p, k))| c.stored().map(|id| (i, (id, *p, *k)))).unzip();
                for (i, e) in at.into_iter().zip(lookup_all(store, program, &stored)?) {
                    out[i] = e;
                }
                Ok(out)
            }
            Memo::Held(_) => docs.iter().map(|(c, p, k)| self.lookup(*c, p, *k, program)).collect(),
        }
    }

    /// The entries of many documents, each `(content, path)`, found and not decoded (`Found`): stored ones in one
    /// statement, held ones from memory.
    pub(super) fn found_all(self, program: &Program, docs: &[(Content, &str)]) -> StoreResult<Vec<Option<Found>>> {
        match self {
            Memo::Stored(store) => {
                let mut out: Vec<Option<Found>> = (0..docs.len()).map(|_| None).collect();
                let (at, stored): (Vec<usize>, Vec<(SourceId, &str)>) = docs.iter().enumerate().filter_map(|(i, (c, p))| c.stored().map(|id| (i, (id, *p)))).unzip();
                for (i, e) in at.into_iter().zip(found_all(store, program, &stored)?) {
                    out[i] = e;
                }
                // A content not known to be stored: one this process queued, else a frame of its digest's.
                let mut by_frame = Vec::new();
                for (i, (c, p)) in docs.iter().enumerate() {
                    if let Content::Unstored(d) = c {
                        out[i] = pending(store, &queued_unstored(d, p, program), &store.atoms_handle());
                        if out[i].is_none() {
                            by_frame.push((i, *d));
                        }
                    }
                }
                if !by_frame.is_empty() {
                    let paths: Vec<&str> = docs.iter().map(|(_, p)| *p).collect();
                    for (i, f) in by_digest(store, program, &paths, by_frame)? {
                        out[i] = Some(f);
                    }
                }
                Ok(out)
            }
            Memo::Held(held) => {
                let held = held.lock().expect("the session memo");
                Ok(docs
                    .iter()
                    .map(|(c, p)| match c {
                        Content::Held(digest) => held.get(&(*digest, (*p).to_owned(), program.id)).map(|e| Found::Held(std::sync::Arc::new(e.clone()))),
                        Content::Stored(_) | Content::Unstored(_) => None,
                    })
                    .collect())
            }
        }
    }

    /// `found_all`, for a file whose outputs alone are read (decision 0117 step 1): an entry is read whole, its
    /// other relations not decoded (`Found::projected`). Its outputs were a frame of their own (`local_out`), a
    /// second copy of them written with every entry; decision 0240 §4.
    pub(super) fn found_outputs_all(self, program: &Program, docs: &[(Content, &str)]) -> StoreResult<Vec<Option<Found>>> {
        self.found_all(program, docs)
    }

    pub(super) fn lookup(self, content: Content, path: &str, key: i64, program: &Program) -> StoreResult<Option<Entry>> {
        match (self, content) {
            (Memo::Stored(store), Content::Stored(id)) => lookup(store, id, path, key, program),
            (Memo::Stored(store), Content::Unstored(d)) => match pending(store, &queued_unstored(&d, path, program), &store.atoms_handle()) {
                Some(found) => found.entry(key),
                // Found in the waves by its digest (`by_digest`): its frame.
                None => match by_digest(store, program, &[path], vec![(0, d)])?.pop() {
                    Some((_, found)) => found.entry(key),
                    None => Ok(None),
                },
            },
            (Memo::Held(held), Content::Held(digest)) => {
                Ok(held.lock().expect("the session memo").get(&(digest, path.to_owned(), program.id)).filter(|e| e.key == key).cloned())
            }
            _ => Ok(None),
        }
    }

    /// `remember` of many documents, stored ones written as one wave (`remember_all`), behind the run.
    pub(super) fn remember_all(self, program: &Program, entries: Vec<(Content, String, i64, Entry)>) -> StoreResult<()> {
        self.remember_all_but(program, entries.into_iter().map(|(c, p, k, e)| (c, p, k, e, false, None)).collect())
    }

    /// `remember_all`, each with whether its stored rows frame stands (`remember_all_but`).
    pub(super) fn remember_all_but(self, program: &Program, entries: Vec<(Content, String, i64, Entry, bool, Option<[u8; 32]>)>) -> StoreResult<()> {
        if let Memo::Stored(store) = self {
            return remember_all_but(store, program, entries);
        }
        let entries: Vec<(Content, String, i64, Entry)> = entries.into_iter().map(|(c, p, k, e, _, _)| (c, p, k, e)).collect();
        match self {
            Memo::Stored(store) => remember_all(store, program, entries),
            Memo::Held(_) => {
                for (content, path, key, entry) in &entries {
                    self.remember(*content, path, *key, program, entry)?;
                }
                Ok(())
            }
        }
    }

    pub(super) fn remember(self, content: Content, path: &str, key: i64, program: &Program, entry: &Entry) -> StoreResult<()> {
        match (self, content) {
            (Memo::Stored(store), c @ (Content::Stored(_) | Content::Unstored(_))) => remember_all(store, program, vec![(c, path.to_owned(), key, entry.clone())]),
            (Memo::Held(held), Content::Held(digest)) => {
                let mut held = held.lock().expect("the session memo");
                if held.len() >= HELD_ENTRIES {
                    held.clear();
                }
                held.insert((digest, path.to_owned(), program.id), Entry { key, ..entry.clone() });
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

/// The document a row's location value names: a node (`key << 32 | index`) or a document key.
pub(super) fn document_of(value: i64) -> i64 {
    if value >= 1 << 32 { value >> 32 } else { value }
}

#[cfg(test)]
pub mod tests {
    /// A relation of facts with no rows (a constant folded empty, `constants::fold`) survives its facts: a stored
    /// program read back is the program written, so its digests (the memo's key, the relations' identities) are the
    /// same. Without it, the first run after a rebuild and the second keyed the memo apart.
    #[test]
    fn an_empty_relation_of_facts_reads_back_as_present() {
        let mut prog = crate::code_rule_dsl::logic::rr::parse(".decl a(s: symbol)\n.decl b(s: symbol)\nb(\"x\").\n.output a\n").unwrap();
        prog.facts.insert("a".to_owned(), Vec::new());
        let mut facts = crate::code_rule_dsl::logic::program_facts::Facts::new();
        crate::code_rule_dsl::logic::program_facts::write(&mut facts, "p", &prog);
        let back = crate::code_rule_dsl::logic::program_facts::program(&facts, "p").unwrap();
        assert_eq!(back.facts, prog.facts);
        let key = |p: &crate::code_rule_dsl::logic::front::Program| super::program_digest(&crate::code_rule_dsl::logic::plan::body::Placed::of_program(p).unwrap().v);
        assert_eq!(key(&back), key(&prog));
    }

    use super::*;
    use crate::code_rule_dsl::logic::plan::output::{Cell, col};

    /// Rows of columns typed `types`, of `rows`.
    pub fn rows_of(types: &[u8], rows: &[&[Cell<'_>]]) -> crate::code_rule_dsl::logic::plan::output::Rows {
        let mut out = crate::code_rule_dsl::logic::plan::output::Rows::new(types.to_vec());
        for r in rows {
            out.push(r);
        }
        out
    }

    #[test]
    fn a_held_entry_is_projected_and_keyed() {
        let mut entry = Entry { key: 5, nodes: 100, ..Entry::default() };
        entry.rows.insert("wanted".into(), (2, vec![5, 9]));
        entry.rows.insert("unused".into(), (2, vec![1; 2000]));
        entry.outputs.insert("diagnostic".into(), (vec![0], rows_of(&[col::NODE, col::TEXT], &[&[Cell::Int(5), Cell::Text("bad")]])));
        let keep = |r: &str| r == "wanted";
        let held = Found::Held(std::sync::Arc::new(entry.clone()));
        let projected = held.projected(5, &keep).unwrap().unwrap();
        assert_eq!(projected.rows.keys().collect::<Vec<_>>(), ["wanted"]);
        assert_eq!((projected.outputs, projected.nodes), (entry.outputs, 100));
        assert!(held.projected(6, &keep).unwrap().is_none());
        assert_ne!(with_tree_demand([0; 32], false), with_tree_demand([0; 32], true), "wider consumers cannot hit narrower frames");
    }

    /// Decision 0295 §1: the run writes its entries into their segment itself, and its readers of an entry still
    /// queued read it there (no decoded copy is held for them); once written, the store's row finds the same.
    #[test]
    fn a_queued_entry_is_read_from_the_segment_the_run_wrote() {
        use super::super::super::lower::Domain;
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::open(&dir.path().join("store.sqlite")).unwrap();
        let domains: BTreeMap<String, Vec<Domain>> = [("r".to_owned(), vec![Domain::Int, Domain::Int])].into();
        let program = Program::new([3; 32], &domains);
        let mut entry = Entry { key: 4, nodes: 9, ..Entry::default() };
        entry.rows.insert("r".into(), (2, vec![1, 2, 3, 4]));
        entry.outputs.insert("diagnostic".into(), (vec![0], rows_of(&[col::NODE, col::TEXT], &[&[Cell::Int(5), Cell::Text("bad")]])));
        let id = store.put_source(b"x = 1\n", &[]).unwrap();
        let found = |store: &ContentStore| Memo::Stored(store).found_all(&program, &[(Content::Stored(id), "a.py")]).unwrap().pop().flatten();
        let waves = crate::content_store::write_back::run_waves();
        remember_all(&store, &program, vec![(Content::Stored(id), "a.py".into(), 4, entry.clone())]).unwrap();
        let queued = found(&store).expect("queued, found");
        assert!(matches!(queued, Found::Frame(_)), "read from its segment, not held decoded");
        assert_eq!(queued.entry(4).unwrap(), Some(entry.clone()));
        assert_eq!(store.memo_pack_bytes().unwrap().1, 0, "not written yet");
        drop(waves);
        store.written().unwrap();
        assert!(store.memo_pack_bytes().unwrap().1 > 0, "its row written");
        assert_eq!(found(&store).expect("written, found").entry(4).unwrap(), Some(entry));
    }

    /// An entry symbol past the process's atoms is refused where the entry is made (decision 0170): a read gave the
    /// store's ids unmapped. It was found only by the writer, mapping it later (`symbol 2767 is no atom of this
    /// process's (it holds 2760)`), far from the read.
    #[test]
    fn an_entry_naming_no_atom_of_the_process_is_refused_where_it_is_made() {
        use super::super::super::lower::Domain;
        let domains: BTreeMap<String, Vec<Domain>> = [("named".to_owned(), vec![Domain::Int, Domain::Name])].into();
        let syms = Syms::of(&domains);
        let mut entry = Entry { key: 1, ..Entry::default() };
        entry.rows.insert("named".into(), (2, vec![9_999, 3, 10_000, 4]));
        assert_eq!(foreign_symbol(&[&entry], &syms, 5), None, "numbers past the atoms are not symbols");
        entry.rows.insert("named".into(), (2, vec![1, 3, 2, 5]));
        assert_eq!(foreign_symbol(&[&entry], &syms, 5), Some(("named".to_owned(), 5)));
        entry.rows.insert("base:named".into(), (2, vec![1, -1]));
        entry.rows.insert("named".into(), (2, vec![1, 3]));
        assert_eq!(foreign_symbol(&[&entry], &syms, 5), Some(("base:named".to_owned(), -1)));
    }

    /// A posting counts a constant as its column's source encodes it (`own:postings:field:f`: a grammar's field code,
    /// 65555, not an atom): its columns are known (a forked run refused every entry holding one when they were not),
    /// and none is a symbol, so none is mapped, whatever the relation declares.
    #[test]
    fn a_posting_and_a_grammar_code_hold_no_symbol() {
        use super::super::super::lower::Domain;
        let domains: BTreeMap<String, Vec<Domain>> = [("field".to_owned(), vec![Domain::Node, Domain::Field, Domain::Node]), ("named".to_owned(), vec![Domain::Kind, Domain::Name])].into();
        let syms = Syms::of(&domains);
        assert_eq!(syms.columns("own:postings:field:f", 2), Some(vec![false, false]));
        assert_eq!(syms.columns("base:field", 3), Some(vec![false, false, false]), "a field's code is no atom");
        assert_eq!(syms.columns("named", 2), Some(vec![false, true]), "a kind's neither; a name is");
        let mut entry = Entry { key: 1, ..Entry::default() };
        entry.rows.insert("own:postings:field:f".into(), (2, vec![65_555, 10]));
        assert_eq!(foreign_symbol(&[&entry], &syms, 5), None, "not taken for an atom");
    }

    #[test]
    fn an_entry_holds_the_identities_it_was_made_under_and_no_others() {
        let (a, b, c) = ([1u8; 32], [2u8; 32], [3u8; 32]);
        let mut entry = Entry { key: 7, ..Entry::default() };
        cover(&mut entry, [a, b].iter());
        cover(&mut entry, [b].iter());
        let held = covered(&entry);
        assert_eq!(held.len(), 2);
        assert!(held.contains(&covered_words(&a)) && held.contains(&covered_words(&b)) && !held.contains(&covered_words(&c)));
        // It is an `own:` key, and names no symbol: what is kept over an entry whose rows stand (decision 0280).
        assert_eq!(identities_of(&entry).rows.keys().collect::<Vec<_>>(), [COVERED]);
        assert_eq!(Syms::default().columns(COVERED, 2), Some(vec![false, false]));
    }

    #[test]
    fn keys_are_stable() {
        let paths = ["a.py".to_owned(), "b/c.py".to_owned()];
        let keys = doc_keys(&paths);
        assert_eq!(keys, doc_keys(&paths));
        assert_eq!(doc_keys(&paths[1..]), keys[1..], "a key does not depend on the other paths");
        assert!(keys.iter().all(|&k| (1..1 << 30).contains(&k)));
        assert_eq!(document_of(keys[0] << 32 | 9), keys[0]);
        assert_eq!(document_of(keys[1]), keys[1]);
    }
}
