//! A corpus run's file-local results, memoised per file (decision 0052): what the chunks hand on for one document
//! (its rows of each exported relation and its file-local output lines) under its content, its path and the run's
//! plan. A later run whose plan is the same reads a file's entry instead of parsing and evaluating it.
//!
//! The rows are replayable as they are because their values are stable across runs: a document's key is a function
//! of its path (`doc_keys`), a node is `key << 32 | index` in its tape (a function of the content and the grammar),
//! a kind is its grammar code, and a symbol is an atom of the persistent store, which never gives one out again
//! (decision 0049): the run interns into it. The plan names everything else the rows depend on: the engine build, the
//! grammar version and the file-local program (its constants as atoms). It is one per run, so the run's manifest names
//! it (decision 0053); the document key is in the entry, and an entry under another key is a miss.

use crate::content_store::{ContentStore, HashKey, SourceId, StoreError, StoreResult};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

mod numbering;
use numbering::Stored;

/// The memo's fact schema: bumped when an entry's meaning changes.
/// When it changes, the old number joins `RETIRED_CORPUS_MEMO_SCHEMAS` only if its rows could outlive their plans
/// (they cannot since decision 0053: the plan names the engine build, and dead plans are collected).
const SCHEMA: u32 = crate::content_store::live_fact_schema(7);

/// The entry's key for a base relation's rows (`rows`), kept for the cross-file part that reads them: a memoised
/// file hands on its base rows as an evaluated one does.
pub(super) fn base_key(rel: &str) -> String {
    format!("base:{rel}")
}

/// The entry's key for the identities of the relations it holds (decision 0225 §2): two words of each export's and
/// file-local output's identity (`identities`) it was made under. A relation's rows in an entry are its rows only
/// when its identity now is among them; a relation with no rows for the file has no key in `rows`, so whether it
/// is held is read here, not from `rows`. An `own:` key: it holds no symbol, and stays with the outputs' frame.
pub(super) const COVERED: &str = "own:covered";

/// The prefix of an entry's `lines` keys that hold its guests' requests (decision 0225 §2): the embed slice's output
/// lines of each round that ask at a location of the file (`guests::asked_at`), under `guests:<round>:<relation>`. A
/// chunk of partial hits whose entries hold them under the slice's identities makes its guests from them, without
/// running the slice. Not an output: they are no relation of the file-local part.
pub(super) const GUESTS: &str = "guests:";

/// An entry's `lines` key for round `round`'s lines of slice relation `rel`.
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
    let of = |e: &Entry| -> Vec<(String, Vec<String>)> {
        e.lines.iter().filter(|(k, _)| k.starts_with(GUESTS)).map(|(k, (_, l))| {
            let mut l = l.clone();
            l.sort_unstable();
            l.dedup();
            (k.clone(), l)
        }).filter(|(_, l)| !l.is_empty()).collect()
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

/// Whether `a` and `b` hold the same rows (or lines) of relation `rel`, in any order.
pub(super) fn same_relation(a: &Entry, b: &Entry, rel: &str) -> bool {
    let rows = |e: &Entry| -> Option<(usize, Vec<Vec<i64>>)> {
        e.rows.get(rel).filter(|(_, v)| !v.is_empty()).map(|(w, v)| {
            let mut rows: Vec<Vec<i64>> = v.chunks_exact((*w).max(1)).map(<[i64]>::to_vec).collect();
            rows.sort_unstable();
            (*w, rows)
        })
    };
    let lines = |e: &Entry| -> Option<Vec<String>> {
        e.lines.get(rel).filter(|(_, l)| !l.is_empty()).map(|(_, l)| {
            let mut l = l.clone();
            l.sort_unstable();
            l
        })
    };
    rows(a) == rows(b) && lines(a) == lines(b)
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
    /// Per file-local output: its node columns and its lines.
    pub lines: BTreeMap<String, (Vec<usize>, Vec<String>)>,
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
fn last_key(space: &[u8; 32]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"rerail.corpus-local-last/v1\0");
    hash.update(space);
    hash.finalize().into()
}

/// The whole file-local program whose run last wrote the entries of space `space`, if it ended (decision 0225 §2).
/// What is kept of every file together (the kept statistics) is of that program's entries, and of no other's.
pub(super) fn last_program(store: &ContentStore, space: &[u8; 32]) -> Option<[u8; 32]> {
    store.stat_blob(last_key(space)).ok().flatten().and_then(|b| b.try_into().ok()).filter(|w: &[u8; 32]| *w != [0; 32])
}

/// Records `whole` as `last_program` of `space`, queued behind the run's writes; `None`: a run is under way.
pub(super) fn set_last_program(store: &ContentStore, space: &[u8; 32], whole: Option<[u8; 32]>) {
    store.queue_stat_blob(last_key(space), whole.unwrap_or([0; 32]).to_vec());
}

/// The run's plan: the engine build, the grammar version and `program` (a digest of the file-local part as lowered,
/// its atoms included).
pub(super) fn plan_digest(program: &[u8; 32]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"rerail.corpus-local/v3\0");
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
    match store.held_source_by_hash(hash)? {
        Some(id) => Ok(id),
        None => store.put_source_hashed(text.as_bytes(), hash, &[]),
    }
}

/// The source of `text` when the store holds it: one read, and its pin (O95) queued on the store's writer (decision
/// 0104). `None`: not stored yet; `source`, run on the writer, stores it.
pub(super) fn stored_source(store: &ContentStore, text: &str) -> StoreResult<Option<SourceId>> {
    off();
    let found = store.sources_by_hashes(&[HashKey::sha256(text.as_bytes())])?.pop().flatten();
    if let Some(id) = found {
        store.write_back_syms(crate::content_store::write_back::Syms::None, "a source's pin", 0, Vec::new(), Box::new(move |s| s.pin_held_sources(&[id])));
    }
    Ok(found)
}

/// The source of `text` for the run's manifest (`late`, which the manifest reads as the writer runs it), all of it
/// on the store's writer, behind the run (decision 0104): the source the store holds then, pinned, or, where `store`,
/// one it stores. Queued after a write that stores it (FIFO), it finds that one. `stored_by`: what that write is
/// queued under (`write_back::Key`), where only it stores the text.
pub(super) fn named_source(cache: &ContentStore, text: String, store: bool, stored_by: Option<&crate::content_store::write_back::Key>, late: &std::sync::Arc<std::sync::Mutex<Vec<SourceId>>>) {
    // Stored already (decision 0223): named now, and only its pin queued (a lock, as `stored_source` takes it). The
    // writer is for what a queued write stores: the lookup there was a write a warm run made.
    if let Some(id) = stored_source(cache, &text).ok().flatten() {
        late.lock().expect("the late sources").push(id);
        return;
    }
    // Not stored, and nothing queued stores it: the writer would find nothing either.
    if !store && stored_by.is_some_and(|key| !cache.queued(key)) {
        return;
    }
    let late = std::sync::Arc::clone(late);
    cache.write_back_syms(
        crate::content_store::write_back::Syms::None,
        "a source the manifest names",
        text.len(),
        Vec::new(),
        Box::new(move |s| {
            let hash = HashKey::sha256(text.as_bytes());
            let found = match s.held_source_by_hash(hash)? {
                Some(id) => Some(id),
                None if store => Some(s.put_source_hashed(text.as_bytes(), hash, &[])?),
                None => None,
            };
            late.lock().expect("the late sources").extend(found);
            Ok(())
        }),
    );
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

    /// The store's ids an entry's frame names in its symbol columns, as the frame holds them: those its reader holds
    /// before a run's waves (`ContentStore::hold_frame_symbols`), so a wave decodes an entry another process stored
    /// without reading the store.
    pub(super) fn read(&self, frame: &crate::content_store::AdmittedFrame, out: &mut Vec<i64>) -> StoreResult<()> {
        let Ok(entry) = Stored::decoded(&frame.rows()?, &|_| true) else { return Ok(()) };
        for (key, (w, rows)) in entry.rows() {
            let Some(cols) = self.columns(key, *w) else { continue };
            for row in rows.chunks_exact((*w).max(1)) {
                out.extend(row.iter().zip(&cols).filter(|(_, s)| **s).map(|(v, _)| *v));
            }
        }
        Ok(())
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

/// Maps the symbols of `entries`' rows with `f` (decision 0170), by a memo plan's columns (`syms`), in one call: every
/// symbol of the batch gathered, mapped, and put back. `strict` (forked): an entry read without its plan's columns, or
/// of a relation it does not declare, is refused, its rows may name ids of the other numbering; else such rows are
/// left as they are.
fn map_entries(entries: &mut [Entry], syms: Option<&Syms>, strict: bool, f: impl FnOnce(&mut [i64]) -> StoreResult<()>) -> StoreResult<()> {
    // The refusal names what was not mapped: a key whose columns the plan does not register is a miss of every
    // entry holding it, for as long as the run is forked.
    let refused = |what: &str| StoreError::Invalid(format!("{} ({what})", crate::content_store::atoms::DIVERGED_TEXT));
    let Some(syms) = syms else {
        return if strict { Err(refused("read without the plan's symbol columns")) } else { Ok(()) };
    };
    // Each key's symbol columns, in the entries' order, gathered once and put back in the same order.
    let mut columns: Vec<Option<Vec<usize>>> = Vec::new();
    let mut values: Vec<i64> = Vec::new();
    for entry in entries.iter() {
        for (key, (w, rows)) in entry.rows.iter() {
            let Some(cols) = syms.columns(key, *w) else {
                if strict {
                    return Err(refused(&format!("{key}, {w} wide: no registered columns")));
                }
                columns.push(None);
                continue;
            };
            if cols.len() != *w {
                return Err(refused(&format!("{key}: {w} wide, its columns {}", cols.len())));
            }
            let at: Vec<usize> = (0..*w).filter(|&c| cols[c]).collect();
            if *w > 0 {
                for row in rows.chunks_exact(*w) {
                    values.extend(at.iter().map(|&c| row[c]));
                }
            }
            columns.push(Some(at));
        }
    }
    if values.is_empty() {
        return Ok(());
    }
    f(&mut values)?;
    let (mut columns, mut values) = (columns.into_iter(), values.into_iter());
    for entry in entries.iter_mut() {
        for (_, (w, rows)) in entry.rows.iter_mut() {
            let Some(at) = columns.next().expect("one a key") else { continue };
            if *w > 0 {
                for row in rows.chunks_exact_mut(*w) {
                    for &c in &at {
                        row[c] = values.next().expect("one a symbol");
                    }
                }
            }
        }
    }
    Ok(())
}

/// The memo's relation in its plan's space: one frame per entry (decision 0146).
pub(super) fn entry_rel(plan: &[u8; 32]) -> crate::content_store::FrameRel {
    crate::content_store::FrameRel { name: "local".to_owned(), ident: *plan }
}

/// Where an entry's outputs are kept beside it, under the same part (decision 0117 step 1): its nodes, its tree
/// counts (`own:` rows) and its lines. A run whose kept inputs hold an unchanged file's export and base rows reads
/// only this, not the entry's rows.
fn outputs_rel(plan: &[u8; 32]) -> crate::content_store::FrameRel {
    crate::content_store::FrameRel { name: "local_out".to_owned(), ident: *plan }
}

/// An entry's outputs (`outputs_rel`): its nodes, its tree counts and its lines.
fn outputs_of(e: &Entry) -> Entry {
    Entry { key: e.key, nodes: e.nodes, rows: e.rows.iter().filter(|(r, _)| r.starts_with("own:")).map(|(r, v)| (r.clone(), v.clone())).collect(), lines: e.lines.clone() }
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

/// What an entry's frame says it was derived from (its `reads`): its path, with its document key, and its content.
fn frame_reads(path: &str, key: i64, digest: &[u8; 32]) -> Vec<(String, u64)> {
    let mut reads = vec![(path.to_owned(), key as u64)];
    reads.extend(digest.chunks_exact(8).map(|c| ("sha256".to_owned(), u64::from_le_bytes(c.try_into().expect("8 bytes")))));
    reads
}

/// A memo entry as its stored frame holds it, not decoded: the document's path and key, and its content's digest.
#[derive(Clone)]
pub struct Frame {
    pub path: String,
    /// The symbol columns of the memo plan whose entry it is (`Program`); none read outside a run (`read_run`).
    syms: Option<Syms>,
    key: i64,
    digest: [u8; 32],
    frame: std::sync::Arc<crate::content_store::AdmittedFrame>,
    /// Its outputs' frame (`outputs_rel`), for the identities it holds (`COVERED`): a run that derived the entry's
    /// rows again and found them as they were writes only that frame (decision 0225 §2).
    outputs: Option<std::sync::Arc<crate::content_store::AdmittedFrame>>,
}

impl Frame {
    fn of(frame: std::sync::Arc<crate::content_store::AdmittedFrame>, syms: Option<Syms>) -> Option<Frame> {
        let reads = &frame.stored.reads;
        let (path, key) = reads.first()?.clone();
        if reads.len() != 5 || reads[1..].iter().any(|(n, _)| n != "sha256") {
            return None;
        }
        let mut digest = [0u8; 32];
        for (c, (_, v)) in digest.chunks_exact_mut(8).zip(&reads[1..]) {
            c.copy_from_slice(&v.to_le_bytes());
        }
        Some(Frame { path, syms, key: key as i64, digest, frame, outputs: None })
    }

    fn entry(&self) -> StoreResult<Entry> {
        self.projected(&|_| true)
    }

    fn projected(&self, keep: &dyn Fn(&str) -> bool) -> StoreResult<Entry> {
        // Read from the store (decision 0170): its symbols are the store's, this process's once mapped (another
        // process's past this one's end, or any once forked).
        let mut entry = Stored::decoded(&self.frame.rows()?, keep)?.to_process(self.syms.as_ref(), self.key, self.frame.store_atoms().as_deref())?;
        // The identities its rows hold: those it was written with, and those a later run found its rows hold too. A
        // frame's rows are of its content under each identity it names, so both are true while it stands.
        if let Some(outputs) = self.outputs.as_ref().filter(|_| keep(COVERED)) {
            let more = Stored::decoded(&outputs.rows()?, &|k| k == COVERED)?.to_process(self.syms.as_ref(), self.key, outputs.store_atoms().as_deref())?;
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

/// The newest corpus run of `root`: the relations its memo plan holds, by name, as recorded (`.decl` columns as
/// JSON), and its documents' frames, not decoded, by path. A frame's document key is `frame_key`, its entry
/// `decode_frame`.
pub fn read_run(store: &ContentStore, root: &str) -> StoreResult<Option<(BTreeMap<String, String>, Vec<Frame>)>> {
    let Some((plan, sources, relations)) = store.corpus_run_of(root)? else { return Ok(None) };
    let ids: Vec<SourceId> = sources.into_iter().map(SourceId::from_raw).collect();
    let contents: HashSet<[u8; 32]> = store.source_digests(&ids)?.into_iter().flatten().collect();
    let mut frames: Vec<Frame> = store
        .admit_all_parts(&crate::content_store::local_space(&plan), &entry_rel(&plan))?
        .into_iter()
        .filter_map(|(_, f)| Frame::of(f, None))
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

    /// The whole entry this was found for, when it is a frame of its outputs (`Memo::found_outputs_all`): its frame
    /// of rows, by its content's digest, read from the store. A held entry is whole already. It is asked only when the
    /// kept inputs were not taken, inside the waves; the waves' snapshot holds no entry of a file looked up before
    /// them (`admit_memo_entries`), so a read of the snapshot found none ("its memo entry is gone").
    pub(super) fn whole(&self, store: &ContentStore, program: &Program) -> StoreResult<Option<Found>> {
        match self {
            Found::Held(e) => Ok(Some(Found::Held(std::sync::Arc::clone(e)))),
            // From the store, inside the waves too: a hit read for its outputs alone was looked up before the first
            // wave, and the snapshot holds no entry of a path looked up then (`admit_memo_entries`). Read from the
            // snapshot, its entry was always missing when the kept inputs were not taken (O-store-single-program).
            Found::Frame(f) => {
                let plan = plan_digest(program);
                Ok(by_digest_from(store, program, &[f.path.as_str()], vec![(0, f.digest)], entry_rel(&plan), true)?.pop().map(|(_, found)| found))
            }
        }
    }

    /// Decode only demanded relations. Skipped relations' rows are checked, not kept.
    pub(super) fn projected(&self, key: i64, keep: &dyn Fn(&str) -> bool) -> StoreResult<Option<Entry>> {
        let entry = match self {
            Found::Held(e) => Entry { key: e.key, nodes: e.nodes,
                rows: e.rows.iter().filter(|(r, _)| keep(r)).map(|(r, v)| (r.clone(), v.clone())).collect(),
                lines: e.lines.clone() },
            // Forked (decision 0170), an entry whose symbols cannot be mapped (its relation's columns not
            // registered) is a miss: its rows may name the other numbering's ids.
            Found::Frame(f) => match f.projected(keep) {
                Err(e) if e.to_string().contains(crate::content_store::atoms::DIVERGED_TEXT) => {
                    static SAID: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
                    if std::env::var_os("RERAIL_STATS_KEEP_TRACE").is_some() && SAID.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 3 {
                        eprintln!("rerail: memo entry of {} refused: {e}", f.path);
                    }
                    return Ok(None);
                }
                entry => entry?,
            },
        };
        Ok(Some(entry).filter(|e| e.key == key))
    }

}

/// `lookup_all`, each entry found and not decoded: the contents' digests in one read, their frames in one more. A
/// lookup writes nothing, and marks nothing (decisions 0104, 0227).
pub(super) fn found_all(store: &ContentStore, program: &Program, docs: &[(SourceId, &str)]) -> StoreResult<Vec<Option<Found>>> {
    off();
    let held: Vec<Option<std::sync::Arc<Entry>>> = docs.iter().map(|(s, p)| store.pending::<Entry>(&queued(*s, p, program))).collect();
    let asked: Vec<usize> = (0..docs.len()).filter(|i| held[*i].is_none()).collect();
    let digests = store.source_digests(&asked.iter().map(|i| docs[*i].0).collect::<Vec<_>>())?;
    let wanted: Vec<(usize, [u8; 32])> = asked.iter().zip(&digests).filter_map(|(i, d)| d.map(|d| (*i, d))).collect();
    let paths: Vec<&str> = docs.iter().map(|(_, p)| *p).collect();
    let mut out: Vec<Option<Found>> = held.into_iter().map(|h| h.map(Found::Held)).collect();
    for (i, f) in by_digest(store, program, &paths, wanted)? {
        out[i] = Some(f);
    }
    Ok(out)
}

/// The frames of `wanted`, each `(at, content digest)` of the document at `paths[at]`: a frame's part is its
/// content's and path's, and the frame names both. No source is read: in the waves, the frames are in memory.
fn by_digest(store: &ContentStore, program: &Program, paths: &[&str], wanted: Vec<(usize, [u8; 32])>) -> StoreResult<Vec<(usize, Found)>> {
    by_digest_in(store, program, paths, wanted, entry_rel(&plan_digest(program)))
}

/// `by_digest`, of the frames of `rel`: the entries' (`entry_rel`) or their outputs' (`outputs_rel`).
fn by_digest_in(store: &ContentStore, program: &Program, paths: &[&str], wanted: Vec<(usize, [u8; 32])>, rel: crate::content_store::FrameRel) -> StoreResult<Vec<(usize, Found)>> {
    let plan = &plan_digest(program);
    // An entry (not its outputs) looked up before the waves is read from the store by its key: the snapshot holds a
    // run's entries only for what its waves look up (`admit_memo_entries`, decision 0117).
    let stored = rel.name == entry_rel(plan).name && !crate::code_rule_dsl::logic::profile::inside_waves();
    by_digest_from(store, program, paths, wanted, rel, stored)
}

/// `by_digest_in`, from the store (`stored`) or from the run's snapshot.
fn by_digest_from(store: &ContentStore, program: &Program, paths: &[&str], wanted: Vec<(usize, [u8; 32])>, rel: crate::content_store::FrameRel, stored: bool) -> StoreResult<Vec<(usize, Found)>> {
    let plan = &plan_digest(program);
    let wanted: Vec<(usize, [u8; 32], i64)> = wanted.into_iter().map(|(i, d)| (i, d, part_of(&d, paths[i]))).collect();
    let docs: Vec<((), &str)> = paths.iter().map(|p| ((), *p)).collect();
    let space = crate::content_store::local_space(plan);
    let parts: Vec<i64> = wanted.iter().map(|(_, _, p)| *p).collect();
    let mut frames = if stored {
        // Read from the store, its entries' symbols are held as they are read (decision 0170): a wave decodes them
        // without reading the store. The snapshot's are held as they are admitted (`admit_memo_entries`).
        let frames = store.admit_parts_stored(&space, &rel, &parts)?;
        store.hold_frame_symbols(frames.values(), &|f, out| program.syms.read(f, out))?;
        frames
    } else {
        store.admit_parts_of(&space, &rel, &parts)?
    };
    // A hit marks nothing (decision 0227): an entry is kept while its plan is live and a kept manifest names its
    // content (`collect_unreachable_frames`), not by the runs that read it.
    // An entry's outputs' frame beside it, for the identities a later run found its rows hold (`Frame::outputs`).
    let mut outputs = if rel.name != entry_rel(plan).name {
        HashMap::new()
    } else if stored {
        store.admit_parts_stored(&space, &outputs_rel(plan), &parts)?
    } else {
        store.admit_parts_of(&space, &outputs_rel(plan), &parts)?
    };
    let mut out = Vec::new();
    for (i, digest, part) in wanted {
        let Some(mut frame) = frames.remove(&part).and_then(|f| Frame::of(f, Some(program.syms.clone()))) else { continue };
        frame.outputs = outputs.remove(&part).and_then(|f| Frame::of(f, None)).filter(|o| o.digest == digest && o.path == frame.path).map(|o| o.frame);
        // A part another content or path collides into is a miss.
        if frame.digest == digest && frame.path == docs[i].1 {
            out.push((i, Found::Frame(frame)));
        }
    }
    Ok(out)
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
    let found = store.held_sources_by_hash(&hashes)?;
    texts.iter().zip(hashes).zip(found).map(|((text, hash), id)| match id { Some(id) => Ok(id), None => store.put_source_hashed(text.as_bytes(), hash, &[]) }).collect()
}

/// The stored sources of many contents, by their SHA-256 digests, in their order: one read and nothing written (a
/// lookup writes nothing, decision 0104). The sources found are pinned under the store's holder by its writer.
pub(super) fn found_sources(store: &ContentStore, digests: &[[u8; 32]]) -> StoreResult<Vec<Option<SourceId>>> {
    off();
    let hashes: Vec<HashKey> = digests.iter().map(|d| HashKey::from_sha256_digest(*d)).collect();
    let found = store.sources_by_hashes(&hashes)?;
    let pins: Vec<SourceId> = found.iter().flatten().copied().collect();
    if !pins.is_empty() {
        store.write_back_syms(crate::content_store::write_back::Syms::None, "the pins of the sources the run read", pins.len() * 8, Vec::new(), Box::new(move |store| store.pin_held_sources(&pins)));
    }
    Ok(found)
}

/// Memoises many documents' entries at once, each `(content, path, key, entry)` under `program`: their frames in
/// one transaction (decision 0146). Queued behind the run (decision 0102): encoded and written by the store's
/// writer, each entry read from memory until it is written. A content is keyed by its digest, so one the store
/// does not hold yet is written with the others.
pub(super) fn remember_all(store: &ContentStore, program: &Program, entries: Vec<(Content, String, i64, Entry)>) -> StoreResult<()> {
    remember_all_but(store, program, entries.into_iter().map(|(c, p, k, e)| (c, p, k, e, false)).collect())
}

/// `remember_all`, each entry with whether its rows frame stands as stored (decision 0225 §2): its rows were derived
/// again and found as they were, so only its outputs' frame is written, with the identities they now hold.
pub(super) fn remember_all_but(store: &ContentStore, program: &Program, entries: Vec<(Content, String, i64, Entry, bool)>) -> StoreResult<()> {
    let standing: HashSet<(String, i64)> = entries.iter().filter(|e| e.4).map(|e| (e.1.clone(), e.2)).collect();
    let entries: Vec<(Content, String, i64, Entry)> = entries.into_iter().map(|(c, p, k, e, _)| (c, p, k, e)).collect();
    off();
    let plan = plan_digest(program);
    // The atom table the entries' symbols are numbered by: the writer maps them with it, or refuses them (below).
    let made_under = store.atoms_handle();
    let atoms = made_under.len();
    if let Some((key, v)) = foreign_symbol(&entries.iter().map(|e| &e.3).collect::<Vec<_>>(), &program.syms, atoms) {
        return Err(StoreError::Invalid(format!("the memo's entry for {key}: symbol {v} is no atom of this process's (it holds {atoms}): a read gave the store's ids unmapped")));
    }
    if crate::code_rule_dsl::logic::profile::on() {
        let _t = crate::code_rule_dsl::logic::profile::span(|| "tally the entries (profiling)".into());
        tally(entries.iter().map(|e| &e.3));
    }
    let _queue = crate::code_rule_dsl::logic::profile::span(|| "queue the entries to the store".into());
    let held: Vec<(Content, String, std::sync::Arc<Entry>)> = entries.into_iter().filter(|e| e.0.in_store()).map(|(content, path, key, entry)| (content, path, std::sync::Arc::new(Entry { key, ..entry }))).collect();
    // Nothing to keep is no job: an empty one still takes the writer's transaction (`RERAIL_WARM`).
    if held.is_empty() {
        return Ok(());
    }
    // What the writer writes, the queue's measure: of an entry whose rows frame stands, its outputs alone. The rest of
    // it stays held for this process's readers (`pending`) until it is written; it was the chunk's already.
    let bytes: usize = held.iter().map(|(_, path, e)| if standing.contains(&(path.clone(), e.key)) { e.outputs_bytes() } else { e.bytes() }).sum();
    if bytes > 32 << 20 && crate::code_rule_dsl::logic::profile::on() {
        let mut by: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
        for (_, _, e) in &held {
            for (r, (w, v)) in &e.rows {
                let at = by.entry(r.as_str()).or_default();
                at.0 += v.len() / (*w).max(1);
                at.1 += v.len() * 8;
            }
            for (r, (_, l)) in &e.lines {
                by.entry(r.as_str()).or_default().1 += l.iter().map(|t| t.len() + 24).sum::<usize>();
            }
        }
        let mut by: Vec<_> = by.into_iter().collect();
        by.sort_by(|a, b| b.1.1.cmp(&a.1.1));
        let top: Vec<String> = by.iter().take(12).map(|(r, (n, b))| format!("{r}: {n} rows, {:.1} MB", *b as f64 / 1048576.0)).collect();
        eprintln!("rerail: the memo's entries: {:.1} MB over {} documents; by relation: {}", bytes as f64 / 1048576.0, held.len(), top.join("; "));
    }
    let keys = held.iter().filter_map(|(content, path, e)| Some((queued_content(*content, path, program)?, Some(std::sync::Arc::clone(e) as std::sync::Arc<dyn std::any::Any + Send + Sync>)))).collect();
    let syms = program.syms.clone();
    store.write_back_syms(
        crate::content_store::write_back::Syms::Mapped,
        "the memo's entries",
        bytes,
        keys,
        Box::new(move |store| {
            // The entries' symbols are of the table they were made under: mapped by another (a second table of this
            // store in the process), they would name other strings, or no atom (`symbol 4291 is no atom of this
            // process's (it holds 2765)`, seen twice in concurrent cold runs). Refused, never mapped by the wrong one.
            let writes_with = store.atoms_handle();
            if !std::sync::Arc::ptr_eq(&made_under, &writes_with) {
                return Err(StoreError::Invalid(format!(
                    "the memo's entries were made under another atom table of this store than its writer's ({} atoms, forked {}; the writer's {} atoms, forked {})",
                    made_under.len(), made_under.forked(), writes_with.len(), writes_with.forked()
                )));
            }
            let ids: Vec<SourceId> = held.iter().filter_map(|(c, _, _)| match c { Content::Stored(id) => Some(*id), _ => None }).collect();
            let mut stored = store.source_digests(&ids)?.into_iter();
            // Each entry's symbols as the store numbers them, in the writer's transaction (decision 0170): every
            // entry's in one call, where this process forked from the store.
            // Of an entry whose rows frame stands, only its outputs are written: only they are mapped and encoded.
            let written: Vec<std::borrow::Cow<'_, Entry>> = held
                .iter()
                .map(|(_, path, e)| if standing.contains(&(path.clone(), e.key)) { std::borrow::Cow::Owned(outputs_of(e)) } else { std::borrow::Cow::Borrowed(&**e) })
                .collect();
            let mapped = numbering::to_store(&written.iter().map(|e| &**e).collect::<Vec<_>>(), &syms, store)?;
            let mut puts = Vec::with_capacity(held.len());
            for ((content, path, _), e) in held.iter().zip(&mapped) {
                let digest = match content {
                    Content::Stored(_) => stored.next().flatten(),
                    Content::Unstored(d) => Some(*d),
                    Content::Held(_) => None,
                };
                // A source collected since the run found it leaves the entry unwritten: the next run misses it.
                let Some(digest) = digest else { continue };
                let part = part_of(&digest, path);
                if !standing.contains(&(path.clone(), e.key())) {
                    puts.push(crate::content_store::FramePut { rel: entry_rel(&plan), part, digest: e.key() as u64, reads: frame_reads(path, e.key(), &digest), rows: e.encode() });
                }
                puts.push(crate::content_store::FramePut { rel: outputs_rel(&plan), part, digest: e.key() as u64, reads: frame_reads(path, e.key(), &digest), rows: e.outputs().encode() });
            }
            let space = crate::content_store::local_space(&plan);
            store.keep_frames(&space, &puts, &[], None)?;
            // A path's earlier contents: its newest `MEMO_CONTENTS` kept (an edited file's entry each edit, 15 MB for
            // one file of the 20, was kept 32 runs: 450 MB of one space).
            let mut keys: Vec<u64> = puts.iter().map(|p| p.digest).collect();
            keys.sort_unstable();
            keys.dedup();
            for rel in [entry_rel(&plan), outputs_rel(&plan)] {
                store.keep_newest_by_path(&space, &rel.name, MEMO_CONTENTS, &keys)?;
            }
            Ok(())
        }),
    );
    Ok(())
}

impl Entry {
    /// The bytes the entry holds (its rows and lines), about.
    fn bytes(&self) -> usize {
        self.rows.iter().map(|(r, (_, v))| r.len() + v.len() * 8).sum::<usize>() + self.lines.iter().map(|(r, (c, l))| r.len() + c.len() * 8 + l.iter().map(|t| t.len() + 24).sum::<usize>()).sum::<usize>()
    }

    /// `bytes` of `outputs_of(self)`, the part written of an entry whose rows frame stands.
    fn outputs_bytes(&self) -> usize {
        self.rows.iter().filter(|(r, _)| r.starts_with("own:")).map(|(r, (_, v))| r.len() + v.len() * 8).sum::<usize>() + self.lines.iter().map(|(r, (c, l))| r.len() + c.len() * 8 + l.iter().map(|t| t.len() + 24).sum::<usize>()).sum::<usize>()
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
                        out[i] = store.pending::<Entry>(&queued_unstored(d, p, program)).map(Found::Held);
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

    /// `found_all`, of the entries' outputs alone (`outputs_rel`): a stored memo's frames of them, else (a held
    /// entry, queued by this process or a session's) the entry. `None` where the store keeps no outputs frame (an
    /// entry written before them): the caller finds the whole entry.
    pub(super) fn found_outputs_all(self, program: &Program, docs: &[(Content, &str)]) -> StoreResult<Vec<Option<Found>>> {
        let Memo::Stored(store) = self else { return self.found_all(program, docs) };
        off();
        let plan = plan_digest(program);
        let mut out: Vec<Option<Found>> = docs.iter().map(|(c, p)| queued_content(*c, p, program).and_then(|k| store.pending::<Entry>(&k)).map(Found::Held)).collect();
        let stored: Vec<usize> = (0..docs.len()).filter(|i| out[*i].is_none() && matches!(docs[*i].0, Content::Stored(_))).collect();
        let digests = store.source_digests(&stored.iter().map(|i| docs[*i].0.stored().expect("stored")).collect::<Vec<_>>())?;
        let mut wanted: Vec<(usize, [u8; 32])> = stored.iter().zip(&digests).filter_map(|(i, d)| d.map(|d| (*i, d))).collect();
        wanted.extend((0..docs.len()).filter(|i| out[*i].is_none()).filter_map(|i| match docs[i].0 { Content::Unstored(d) => Some((i, d)), _ => None }));
        let paths: Vec<&str> = docs.iter().map(|(_, p)| *p).collect();
        for (i, f) in by_digest_in(store, program, &paths, wanted, outputs_rel(&plan))? {
            out[i] = Some(f);
        }
        Ok(out)
    }

    pub(super) fn lookup(self, content: Content, path: &str, key: i64, program: &Program) -> StoreResult<Option<Entry>> {
        match (self, content) {
            (Memo::Stored(store), Content::Stored(id)) => lookup(store, id, path, key, program),
            (Memo::Stored(store), Content::Unstored(d)) => match store.pending::<Entry>(&queued_unstored(&d, path, program)) {
                Some(e) => Ok(Some((*e).clone()).filter(|e| e.key == key)),
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
        self.remember_all_but(program, entries.into_iter().map(|(c, p, k, e)| (c, p, k, e, false)).collect())
    }

    /// `remember_all`, each with whether its stored rows frame stands (`remember_all_but`).
    pub(super) fn remember_all_but(self, program: &Program, entries: Vec<(Content, String, i64, Entry, bool)>) -> StoreResult<()> {
        let entries: Vec<(Content, String, i64, Entry, bool)> = entries;
        if let Memo::Stored(store) = self {
            return remember_all_but(store, program, entries);
        }
        let entries: Vec<(Content, String, i64, Entry)> = entries.into_iter().map(|(c, p, k, e, _)| (c, p, k, e)).collect();
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
    use super::*;

    #[test]
    fn projection_preserves_outputs_and_validates_skipped_payloads() {
        let mut entry = Entry { key: 5, nodes: 100, ..Entry::default() };
        entry.rows.insert("wanted".into(), (2, vec![5,9]));
        entry.rows.insert("unused".into(), (2, vec![1;2000]));
        entry.lines.insert("diagnostic".into(), (vec![0], vec!["5\tbad".into()]));
        let keep = |r: &str| r == "wanted";
        let rows = Stored::unmapped(entry.clone()).encode();
        let entry_copy = entry.clone();
        let projected = Entry { key: 5, ..Stored::decoded(&rows, &keep).unwrap().into_unmapped() };
        assert_eq!(projected.rows.len(),1);
        assert_eq!(projected.rows["wanted"],entry.rows["wanted"]);
        assert_eq!(projected.lines,entry.lines);
        assert_eq!(projected.nodes,100);
        let held = Found::Held(std::sync::Arc::new(entry));
        assert_eq!(held.projected(5,&keep).unwrap(),Some(projected));
        assert!(held.projected(6,&keep).unwrap().is_none());
        let mut cut = Stored::unmapped(entry_copy.clone()).encode();
        cut.rows.truncate(cut.rows.len() - 1);
        assert!(Stored::decoded(&cut,&keep).is_err(),"a truncated unused payload is still corrupt");
        assert_ne!(with_tree_demand([0;32],false),with_tree_demand([0;32],true),"wider consumers cannot hit narrower frames");
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
        map_entries(std::slice::from_mut(&mut entry), Some(&syms), true, |v| {
            v.iter_mut().for_each(|x| *x += 100);
            Ok(())
        })
        .expect("a posting's columns are known");
        assert_eq!(entry.rows["own:postings:field:f"].1, vec![65_555, 10], "not mapped");
        assert_eq!(foreign_symbol(&[&entry], &syms, 5), None, "nor taken for an atom");
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
        // It is an `own:` key: the outputs' frame keeps it, and it names no symbol.
        assert!(outputs_of(&entry).rows.contains_key(COVERED));
        assert_eq!(Syms::default().columns(COVERED, 2), Some(vec![false, false]));
    }

    #[test]
    fn an_entry_round_trips_and_keys_are_stable() {
        let mut entry = Entry { key: 5, nodes: 7, ..Entry::default() };
        entry.rows.insert("fl1_violation_at".into(), (3, vec![5, 1, 2, 5, 3, 4]));
        entry.lines.insert("violation".into(), (vec![1], vec!["r\t21474836480\tx".into()]));
        entry.rows.insert("wide".into(), (11, (0..22).collect()));
        entry.rows.insert("empty".into(), (2, Vec::new()));
        entry.lines.insert("a name longer than one row of text holds, which is fifty-six bytes".into(), (vec![], vec![String::new(), "ünïcode".into()]));
        let all = |_: &str| true;
        assert_eq!(Stored::decoded(&Stored::unmapped(entry.clone()).encode(), &all).map(|e| Entry { key: 5, ..e.into_unmapped() }).unwrap(), entry);
        let mut cut = Stored::unmapped(entry.clone()).encode();
        cut.rows.truncate(cut.rows.len() - 1);
        assert!(Stored::decoded(&cut, &all).is_err());
        let paths = ["a.py".to_owned(), "b/c.py".to_owned()];
        let keys = doc_keys(&paths);
        assert_eq!(keys, doc_keys(&paths));
        assert_eq!(doc_keys(&paths[1..]), keys[1..], "a key does not depend on the other paths");
        assert!(keys.iter().all(|&k| (1..1 << 30).contains(&k)));
        assert_eq!(document_of(keys[0] << 32 | 9), keys[0]);
        assert_eq!(document_of(keys[1]), keys[1]);
    }
}
