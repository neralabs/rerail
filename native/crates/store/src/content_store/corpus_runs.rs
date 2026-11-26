//! Corpus runs' roots and the bounded history the store keeps of them (decision 0053).
//!
//! Each run of a corpus root records a manifest: the sources it read, the grammar version and tape format of their
//! tapes, and its memo plan. Collection keeps
//!
//! 1. each root's last `protected_runs` manifests of each lane (what the runs were of, `record_corpus_run_lane`),
//!    for the root's newest `protected_lanes` lanes;
//! 2. older manifests, newest first, while the bytes they alone hold (the history) stay under a budget: no limit
//!    below the high-water mark, the low-water mark once the history passes it;
//!
//! and drops a manifest whose sources all appear in newer kept ones, and every manifest of a root idle past the
//! backstop. What the kept manifests name is `corpus_live`, which the collector's tape and source queries honour;
//! the memo rows of plans no kept manifest names are deleted in batches. The bound is by size, not by age: a branch
//! can be churned whole many times a day.

#[allow(unused_imports)]
use crate::content_store::Cached as _;
use super::{CollectionReport, ContentStore, MAX_COLLECTION_BATCH, STRING_OWNER, SourceId, StoreError, StoreResult};
use super::rows::intern_string;

/// `object_pins`' kind for a memo plan a run holds (O95): its id is the plan digest's first eight bytes.
const OBJECT_PLAN: i64 = 3;

fn plan_object(plan: &[u8]) -> i64 {
    let mut first = [0u8; 8];
    first.copy_from_slice(&plan[..8]);
    i64::from_le_bytes(first)
}
use rusqlite::{OptionalExtension, params};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// What collection keeps of corpus runs.
#[derive(Clone, Copy, Debug)]
pub struct CorpusPolicy {
    /// Each root's last runs of a lane, always kept, their plans live.
    pub protected_runs: usize,
    /// The lanes of a root whose last runs are kept, its newest: the programs run over it one at a time (a policy set's,
    /// one process each) keep their plans while the others run. One edited, or gone, its lane is not run again.
    pub protected_lanes: usize,
    /// Above this many bytes of history, a pass evicts.
    pub high_water: u64,
    /// Eviction keeps history up to this many bytes.
    pub low_water: u64,
    /// A root not run for this many seconds keeps nothing, its last runs included.
    pub backstop_secs: i64,
    /// A root whose directory is gone and that has not run for this many seconds keeps nothing: no run can be warm
    /// there again (a test's or a replay's temporary roots).
    pub missing_secs: i64,
    /// With no root past its protected runs, a decision waits for this many runs since the last one…
    pub decide_every_runs: i64,
    /// …or this many seconds (for the backstop and gone roots).
    pub decide_every_secs: i64,
}

impl Default for CorpusPolicy {
    fn default() -> Self {
        CorpusPolicy {
            protected_runs: 2,
            protected_lanes: 64,
            high_water: 512 << 20,
            low_water: 384 << 20,
            backstop_secs: 30 * 86_400,
            missing_secs: 3_600,
            decide_every_runs: 256,
            decide_every_secs: 600,
        }
    }
}

impl CorpusPolicy {
    /// The policy a run collects under: the default, its history's bounds set by `RERAIL_CORPUS_HISTORY_MB`; `None`
    /// with `RERAIL_CORPUS_GC=off`.
    pub fn of_run() -> Option<CorpusPolicy> {
        if std::env::var("RERAIL_CORPUS_GC").is_ok_and(|v| v == "off") {
            return None;
        }
        let mut policy = CorpusPolicy::default();
        if let Some(mb) = std::env::var("RERAIL_CORPUS_HISTORY_MB").ok().and_then(|v| v.parse::<u64>().ok()) {
            (policy.high_water, policy.low_water) = (mb << 20, (mb << 20) / 4 * 3);
        }
        Some(policy)
    }
}

/// What one decision kept and dropped.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CorpusKept {
    pub manifests: usize,
    pub evicted: usize,
    /// Bytes the protected manifests hold.
    pub protected_bytes: u64,
    /// Bytes the other kept manifests hold alone, with the ended workspaces' aged bytes (decision 0450 step 3).
    pub history_bytes: u64,
    /// Entries of `corpus_live` removed.
    pub unlived: usize,
}

/// One pass: the decision (when one was due), memo rows of dead plans deleted, and the store's collection.
#[derive(Debug, Default)]
pub struct CorpusPass {
    pub kept: Option<CorpusKept>,
    pub plan_rows: usize,
    pub collection: CollectionReport,
    pub observer: Option<super::observe::Swept>,
}

impl CorpusPass {
    /// Whether the pass deleted anything (another may find more).
    pub fn did_work(&self) -> bool {
        let c = &self.collection;
        self.kept.as_ref().is_some_and(|k| k.evicted > 0 || k.unlived > 0) || self.plan_rows > 0 || c.tapes > 0 || c.sources > 0 || c.snapshots > 0 || c.snapshot_files > 0
    }
}

fn lapsed_plan_hold(connection: &rusqlite::Connection, now: i64) -> StoreResult<bool> {
    Ok(connection
        .query_row_cached("SELECT 1 FROM object_pins WHERE object_kind_id=?1 AND lease_until<?2 LIMIT 1", params![OBJECT_PLAN, now], |r| r.get::<_, i64>(0))
        .optional()?
        .is_some())
}

/// The holds (`object_pins`) of runs whose process is gone: a run killed (a watchdog, a crash) never releases its
/// hold, which kept its plans live, and their memos (decision 0146), for the lease's 6 hours. A run's owner names its
/// process (`corpus-run:<pid>:…`); a process of this host that no longer exists holds nothing. A pid used again by
/// another process keeps the hold to its lease, as before.
fn orphaned_holds(connection: &rusqlite::Connection) -> StoreResult<Vec<String>> {
    let mut st = connection.prepare_cached(
        "SELECT DISTINCT s.value FROM object_pins p JOIN strings s ON s.string_id = p.owner_string_id WHERE s.domain_id = ?1 AND s.value LIKE 'corpus-run:%'",
    )?;
    let owners = st.query_map([STRING_OWNER], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
    Ok(owners.into_iter().filter(|o| o.split(':').nth(1).and_then(|p| p.parse::<i32>().ok()).is_some_and(process_gone)).collect())
}

/// Whether no process `pid` exists on this host: never this process, and on Windows when the system knows no
/// running process of that id.
pub fn process_gone(pid: i32) -> bool {
    if pid <= 0 || pid as u32 == std::process::id() {
        return false;
    }
    #[cfg(unix)]
    {
        // SAFETY: signal 0 only checks that the process exists.
        let r = unsafe { libc::kill(pid, 0) };
        r != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, GetLastError, WAIT_OBJECT_0};
        use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};
        // An unknown PID gives ERROR_INVALID_PARAMETER. Access denied can mean a live protected process.
        unsafe {
            let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid as u32);
            if handle.is_null() { return GetLastError() == ERROR_INVALID_PARAMETER; }
            let exited = WaitForSingleObject(handle, 0) == WAIT_OBJECT_0;
            CloseHandle(handle);
            exited
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        false
    }
}

/// The least a memo pack holds before it is compacted: below it, its unnamed bytes are left.
const MEMO_PACK_FLOOR: u64 = 64 << 20;

/// Memo rows of dead plans deleted per pass.
const PLAN_ROWS_PER_PASS: i64 = 4096;

#[derive(Clone)]
struct Manifest {
    id: i64,
    root: i64,
    last_time: i64,
    grammar_version: i64,
    tape_format: i64,
    plan: Vec<u8>,
    /// What the run was of (`record_corpus_run_lane`); `None`, the root's one lane.
    lane: Option<Vec<u8>>,
    /// The lane's policy set (`record_corpus_run_policy`); `None`, a lane of no set.
    policy: Option<Vec<u8>>,
    sources: Vec<i64>,
}

/// Which manifests, met newest first, are among their lane's last `protected_runs` and their lane among the root's
/// newest `protected_lanes`.
///
/// A lane of a policy set (decision 0302) is protected only while it is its set's newest: an older one (the set's
/// programs before an edit) holds no protection and takes no place among the root's lanes.
#[derive(Default)]
struct Protection<'m> {
    runs: HashMap<(i64, Option<&'m [u8]>), usize>,
    lanes: HashMap<i64, Vec<Option<&'m [u8]>>>,
    /// Each root's policy sets' newest lanes.
    newest: HashMap<(i64, &'m [u8]), Option<&'m [u8]>>,
    /// The roots with a manifest that names its set: there, a lane with none is an older binary's (§4).
    named: HashSet<i64>,
}

impl<'m> Protection<'m> {
    fn over(manifests: &[Manifest]) -> Self {
        Protection { named: manifests.iter().filter(|m| m.policy.is_some()).map(|m| m.root).collect(), ..Protection::default() }
    }

    fn protects(&mut self, m: &'m Manifest, runs: usize, lanes: usize) -> bool {
        let lane = m.lane.as_deref();
        // Decision 0302 §4: this binary names every lane's set (a lane of no set is its own), so a lane with none
        // over a root it ran is an older binary's, which no newer lane of its set can supersede.
        if lane.is_some() && m.policy.is_none() && self.named.contains(&m.root) {
            return false;
        }
        if let Some(policy) = m.policy.as_deref()
            && *self.newest.entry((m.root, policy)).or_insert(lane) != lane
        {
            return false;
        }
        let seen = self.lanes.entry(m.root).or_default();
        let at = seen.iter().position(|l| *l == lane).unwrap_or_else(|| {
            seen.push(lane);
            seen.len() - 1
        });
        let n = self.runs.entry((m.root, lane)).or_default();
        *n += 1;
        at < lanes && *n <= runs
    }
}

impl ContentStore {
    /// Records the relations a memo plan's entries hold rows of, each with its declared columns (JSON).
    pub fn record_corpus_relations(&self, plan: [u8; 32], relations: &[(String, String)]) -> StoreResult<()> {
        let ((), written) = self.immediate_transaction(|tx| {
            self.corpus_write(tx, |c, w| {
                // Each relation's columns replaced, the plan's others kept.
                let mut all: BTreeMap<String, String> = c.relations(&plan).into_iter().collect();
                all.extend(relations.iter().cloned());
                w.relations(plan, &all.into_iter().collect::<Vec<_>>());
                Ok(())
            })
        })?;
        self.note_kept(&kept::space(), written);
        Ok(())
    }

    /// The newest run of `root` (as recorded, canonical): its memo plan, the sources it read, and the relations
    /// its plan's entries hold rows of with their declared columns (JSON). `None` if the root has no run.
    #[allow(clippy::type_complexity)]
    pub fn corpus_run_of(&self, root: &str) -> StoreResult<Option<([u8; 32], Vec<i64>, Vec<(String, String)>)>> {
        self.corpus_kept(|c| {
            let m = c.root(root).and_then(|r| c.manifests_of(r).into_iter().next())?;
            Some((m.plan, c.sources(m.id), c.relations(&m.plan)))
        })
    }

    /// The digests the newest run of `root` named besides its plan (`record_corpus_run_naming`'s `also`): among them,
    /// what it kept for the next run to find by them (its pipelines' runs, decision 0333).
    pub fn corpus_run_also(&self, root: &str) -> StoreResult<Vec<[u8; 32]>> {
        self.corpus_kept(|c| c.root(root).and_then(|r| c.manifests_of(r).into_iter().next()).map(|m| c.named(m.id)).unwrap_or_default())
    }

    /// `corpus_run_also` of the newest run of lane `lane` over `root`: what a run of the same programs named last.
    pub fn corpus_lane_also(&self, root: &str, lane: &[u8; 32]) -> StoreResult<Vec<[u8; 32]>> {
        self.corpus_kept(|c| {
            c.root(root).and_then(|r| c.manifests_of(r).into_iter().find(|m| m.lane.as_ref() == Some(lane))).map(|m| c.named(m.id)).unwrap_or_default()
        })
    }

    /// The sources the newest `runs` runs over `root` read, of any lane. A lane is the programs' texts, so a rule's
    /// edit starts a new one with no run: what a root's files and guests are does not change with the rules (Q22).
    pub fn root_sources(&self, root: &str, runs: usize) -> StoreResult<Vec<SourceId>> {
        self.corpus_kept(|c| {
            let manifests = c.root(root).map(|r| c.manifests_of(r)).unwrap_or_default();
            let out: std::collections::BTreeSet<i64> = manifests.iter().take(runs).flat_map(|m| c.sources(m.id)).collect();
            out.into_iter().map(SourceId).collect()
        })
    }

    /// Records a run of the corpus at `root` that read `sources` (tapes of `grammar_version` and `tape_format`) under
    /// memo plan `plan`, at unix second `now`. True if the manifest is new: a run matching an earlier manifest of its
    /// root only moves that one's last run.
    pub fn record_corpus_run(&self, root: &str, sources: &[SourceId], grammar_version: i64, tape_format: i64, plan: [u8; 32], now: i64) -> StoreResult<bool> {
        self.record_corpus_run_naming(root, sources, grammar_version, tape_format, plan, &[], now)
    }

    /// `record_corpus_run`, the manifest also naming `also`: other memo plans the run stored entries under (its
    /// prepared program's, decision 0073). They are live while a kept manifest names them, as its memo plan is, and
    /// their rows are collected once none does.
    #[allow(clippy::too_many_arguments)]
    pub fn record_corpus_run_naming(&self, root: &str, sources: &[SourceId], grammar_version: i64, tape_format: i64, plan: [u8; 32], also: &[[u8; 32]], now: i64) -> StoreResult<bool> {
        self.record_corpus_run_lane(root, sources, grammar_version, tape_format, plan, also, None, now)
    }

    /// `record_corpus_run_naming`, the run of lane `lane`: what it was of, whichever build ran it (its programs'
    /// texts). A root's last runs are protected by lane (`CorpusPolicy::protected_runs`): of policies run one process
    /// each over a root, each keeps its plan, where by root only the last two did, and the rest of the set ran cold the
    /// next time. A replaced build's plan is the same lane's, and dies as before. `None`: the root's one lane.
    #[allow(clippy::too_many_arguments)]
    pub fn record_corpus_run_lane(&self, root: &str, sources: &[SourceId], grammar_version: i64, tape_format: i64, plan: [u8; 32], also: &[[u8; 32]], lane: Option<[u8; 32]>, now: i64) -> StoreResult<bool> {
        self.record_corpus_run_policy(root, sources, grammar_version, tape_format, plan, also, lane, None, now)
    }

    /// `record_corpus_run_lane`, the lane of policy set `policy` (decision 0302): a newer lane of the set supersedes
    /// its older ones, which keep no protection (`Protection`). `None`: a lane of no set, protected as before.
    #[allow(clippy::too_many_arguments)]
    pub fn record_corpus_run_policy(&self, root: &str, sources: &[SourceId], grammar_version: i64, tape_format: i64, plan: [u8; 32], also: &[[u8; 32]], lane: Option<[u8; 32]>, policy: Option<[u8; 32]>, now: i64) -> StoreResult<bool> {
        self.record_scoped_corpus_run(root, sources, grammar_version, tape_format, plan, also, lane, policy, now, None)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_scoped_corpus_run(&self, root: &str, sources: &[SourceId], grammar_version: i64, tape_format: i64, plan: [u8; 32], also: &[[u8; 32]], lane: Option<[u8; 32]>, policy: Option<[u8; 32]>, now: i64, scope: Option<&str>) -> StoreResult<bool> {
        self.record_corpus_run_retaining(root, sources, grammar_version, tape_format, plan, also, lane, policy, now, scope, None)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_corpus_run_retaining(&self, root: &str, sources: &[SourceId], grammar_version: i64, tape_format: i64, plan: [u8; 32], also: &[[u8; 32]], lane: Option<[u8; 32]>, policy: Option<[u8; 32]>, now: i64, scope: Option<&str>, owner: Option<&super::analysis_scope::AnalysisOwner>) -> StoreResult<bool> {
        let mut also: Vec<[u8; 32]> = also.iter().copied().filter(|p| *p != plan).collect();
        also.sort_unstable();
        also.dedup();
        let mut ids: Vec<i64> = sources.iter().map(|s| s.0).collect();
        ids.sort_unstable();
        ids.dedup();
        let blob = encode_ids(&ids);
        let mut hash = Sha256::new();
        hash.update(b"rerail.corpus-manifest/v1\0");
        hash.update(grammar_version.to_le_bytes());
        hash.update(tape_format.to_le_bytes());
        hash.update(plan);
        // A manifest naming no other plan keeps the digest it had before `also` existed.
        for p in &also {
            hash.update(p);
        }
        hash.update(&blob);
        let digest: [u8; 32] = hash.finalize().into();
        let plans: Vec<[u8; 32]> = std::iter::once(plan).chain(also.iter().copied()).collect();
        let (new, written) = self.immediate_transaction(|tx| {
            // A run's first write acquires (decision 0547): the manifest's, applied by the writer behind the run.
            if let Some(owner) = owner {
                owner.acquire(tx)?;
                if !owner.current(tx)? {
                    return Ok((false, None));
                }
            }
            let scope = owner.map(|owner| owner.membership()).or(scope);
            self.corpus_write(tx, |c, w| {
                let root_id = match c.root(root) {
                    Some(id) => id,
                    None => {
                        let id = c.roots().iter().map(|r| r.0).max().unwrap_or(0) + 1;
                        w.root(id, Some(root));
                        id
                    }
                };
                let mut state = c.state();
                state.run += 1;
                let run = state.run;
                let manifests = c.manifests_of(root_id);
                // Its plans are live (a plan recorded before stays recorded).
                for p in &plans {
                    w.plan(*p, Some(true));
                }
                let set_lane = |m: &mut KeptManifest| -> rusqlite::Result<()> {
                    if let Some(scope) = scope {
                        let acquisition = match owner {
                            Some(owner) => owner.acquisition,
                            None => super::analysis_scope::next_acquisition(tx)?,
                        };
                        tx.execute_cached("INSERT INTO analysis_scope_manifest(scope,manifest_id,acquired_at,acquisition,recorded_run)
                            VALUES (?1,?2,?3,?4,?5) ON CONFLICT(scope,manifest_id) DO UPDATE SET
                              (acquired_at,acquisition,recorded_run)=(excluded.acquired_at,excluded.acquisition,excluded.recorded_run)
                            WHERE (acquired_at,acquisition,recorded_run)<(excluded.acquired_at,excluded.acquisition,excluded.recorded_run)",
                            (scope, m.id, owner.map_or(0, super::analysis_scope::AnalysisOwner::acquired_at), acquisition, run))?;
                    }
                    if lane.is_some() {
                        m.lane = lane;
                    }
                    if policy.is_some() {
                        m.policy = policy;
                    }
                    Ok(())
                };
                // A run matching an earlier manifest of its root only moves that one's last run.
                if let Some(mut m) = manifests.iter().find(|m| m.digest == digest).cloned() {
                    (m.last_run, m.last_time) = (run, now);
                    set_lane(&mut m)?;
                    w.manifest(m.id, Some(&m));
                    w.state(state);
                    return Ok(false);
                }
                // The sources the root's newest manifest of the same tapes already made live need no row again.
                let before: HashSet<i64> = manifests
                    .iter()
                    .find(|m| m.grammar_version == grammar_version && m.tape_format == tape_format)
                    .map(|m| c.sources(m.id).into_iter().collect())
                    .unwrap_or_default();
                let id = c.manifest_ids().last().copied().unwrap_or(0) + 1;
                let mut m = KeptManifest { id, root: root_id, last_run: run, last_time: now, grammar_version, tape_format, plan, digest, lane: None, policy: None };
                set_lane(&mut m)?;
                w.manifest(id, Some(&m));
                w.sources(id, &ids);
                w.named(id, &also);
                for &source in ids.iter().filter(|id| !before.contains(id)) {
                    let mut tapes = c.live_of(source);
                    if !tapes.contains(&(grammar_version, tape_format)) {
                        tapes.push((grammar_version, tape_format));
                        w.live(source, &tapes);
                    }
                }
                state.due = 1;
                w.state(state);
                Ok(true)
            })
        })?;
        self.note_kept(&kept::space(), written);
        Ok(new)
    }

    /// Holds memo plan `plan` for `owner` until unix second `lease_until`, and makes it live, in one transaction
    /// (O95). A run writes its memo rows before it records its manifest, and a plan an earlier, evicted manifest
    /// named may be dead, its rows being deleted. Held, it stays live and keeps its rows until the run's manifest
    /// names it. `release_pins(owner)` ends the hold; a run that dies leaves it to lapse with its lease.
    pub fn hold_corpus_plan(&self, plan: [u8; 32], owner: &str, lease_until: i64) -> StoreResult<()> {
        if owner.is_empty() || lease_until <= 0 {
            return Err(StoreError::Invalid("object pin is invalid".to_owned()));
        }
        self.immediate_transaction(|tx| {
            let owner_id = intern_string(tx, STRING_OWNER, owner)?;
            tx.execute_cached(
                "INSERT INTO object_pins(object_kind_id,object_id,owner_string_id,lease_until) VALUES (?1,?2,?3,?4) \
                 ON CONFLICT(object_kind_id,object_id,owner_string_id) DO UPDATE SET lease_until=excluded.lease_until",
                params![OBJECT_PLAN, plan_object(&plan), owner_id, lease_until],
            )?;
            Ok(self.corpus_write(tx, |_, w| {
                w.plan(plan, Some(true));
                Ok(())
            })?
            .1)
        })
        .map(|written| self.note_kept(&kept::space(), written))
    }

    /// The manifests a run whose every read is as it was would record again (a whole-run cutoff's, decision 0547),
    /// for `owner`: of `root`'s, per lane the newest manifest of each of `plans` (the recorded run's), its run and
    /// time moved and the owner's acquisition its membership, as an identical manifest's are when recorded
    /// (`record_corpus_run_retaining`). How many; none when they are gone (collected) or the owner may no longer
    /// publish.
    pub fn own_recorded_manifests(&self, root: &str, plans: &[[u8; 32]], owner: &super::analysis_scope::AnalysisOwner, now: i64) -> StoreResult<usize> {
        let (n, written) = self.immediate_transaction(|tx| {
            owner.acquire(tx)?;
            if !owner.current(tx)? {
                return Ok((0, None));
            }
            self.corpus_write(tx, |c, w| {
                let Some(root_id) = c.root(root) else { return Ok(0) };
                // Per plan and lane, the newest manifest (the greatest id).
                let mut newest: BTreeMap<([u8; 32], Option<[u8; 32]>), KeptManifest> = BTreeMap::new();
                for m in c.manifests_of(root_id).into_iter().filter(|m| plans.contains(&m.plan)) {
                    let e = newest.entry((m.plan, m.lane)).or_insert_with(|| m.clone());
                    if m.id > e.id {
                        *e = m;
                    }
                }
                let mut state = c.state();
                for m in newest.values_mut() {
                    state.run += 1;
                    (m.last_run, m.last_time) = (state.run, now);
                    w.manifest(m.id, Some(m));
                    tx.execute_cached(
                        "INSERT INTO analysis_scope_manifest(scope,manifest_id,acquired_at,acquisition,recorded_run) VALUES (?1,?2,?3,?4,?5) \
                         ON CONFLICT(scope,manifest_id) DO UPDATE SET (acquired_at,acquisition,recorded_run)=(excluded.acquired_at,excluded.acquisition,excluded.recorded_run) \
                         WHERE (acquired_at,acquisition,recorded_run)<(excluded.acquired_at,excluded.acquisition,excluded.recorded_run)",
                        (owner.membership(), m.id, owner.acquired_at(), owner.acquisition, state.run),
                    )?;
                }
                if !newest.is_empty() {
                    // The recorded plans among them live again.
                    let recorded: HashSet<[u8; 32]> = c.plans().into_iter().map(|p| p.0).collect();
                    for p in plans.iter().filter(|p| recorded.contains(*p)) {
                        w.plan(*p, Some(true));
                    }
                    w.state(state);
                }
                Ok(newest.len())
            })
        })?;
        self.note_kept(&kept::space(), written);
        Ok(n)
    }

    /// Whether corpus collection has work: a new manifest to decide on, or a decision's rows not yet collected.
    pub fn corpus_collection_due(&self) -> StoreResult<bool> {
        Ok(self.corpus_due()? != 0)
    }

    /// Whether a pass under `policy` at unix second `now` has work: a decision's rows being collected, or a decision
    /// due that can change something (`corpus_decision_worth`). A due decision that cannot is cleared here, so a run
    /// of fresh roots neither decides nor sweeps the store after each run.
    /// The plans a kept manifest names or a run holds (`corpus_plans`, live): what the observer's store keeps a
    /// whole-run cutoff's record for (`observe::collect`).
    pub fn live_plans(&self) -> StoreResult<HashSet<[u8; 32]>> {
        self.corpus_kept(|c| c.plans().into_iter().filter(|p| p.1).map(|p| p.0).collect())
    }

    /// The collection a run's owner held off while it ran (its lease, `AnalysisRun`), once it lets go: passes while
    /// they find work, for up to 200 ms, as a run's own (`record_run`). The passes and the decision, if one was made;
    /// `None` when none was wanted.
    pub fn collect_after(&self, now: i64) -> StoreResult<Option<(usize, Option<CorpusKept>)>> {
        let Some(policy) = CorpusPolicy::of_run() else { return Ok(None) };
        if !self.corpus_pass_wanted(&policy, now)? {
            return Ok(None);
        }
        let started = std::time::Instant::now();
        let (mut idle, mut passes, mut kept) = (0, 0, None);
        while idle < 2 && started.elapsed() < std::time::Duration::from_millis(200) {
            let pass = self.collect_corpus_pass(&policy, now)?;
            passes += 1;
            idle = if pass.did_work() { 0 } else { idle + 1 };
            kept = pass.kept.or(kept);
        }
        Ok(Some((passes, kept)))
    }

    pub fn corpus_pass_wanted(&self, policy: &CorpusPolicy, now: i64) -> StoreResult<bool> {
        if self.analysis_snapshot_due(now)? {
            return Ok(true);
        }
        let due = match self.corpus_due()? {
            0 if self.lapsed_plan_hold(now)? => 1,
            due => due,
        };
        match due {
            0 => Ok(false),
            1 if !self.corpus_decision_worth(policy, now)? => {
                self.corpus_state_with(|s| if s.due == 1 { s.due = 0 })?;
                Ok(false)
            }
            _ => Ok(true),
        }
    }

    /// Whether a keep decision can change anything now: a root recorded since the last decision has more manifests
    /// than its protected runs (history grows only so), or `decide_every_runs` runs or `decide_every_secs` seconds
    /// have passed since it (the backstop and gone roots). Reads the recent manifests through `last_run`'s index.
    fn corpus_decision_worth(&self, policy: &CorpusPolicy, now: i64) -> StoreResult<bool> {
        let connection = self.connection.lock().expect("content store connection unavailable");
        let (state, manifests) = kept::with_corpus(&self.database_path, &connection, |c| (c.state(), c.manifests()))?;
        if state.run - state.decided_run >= policy.decide_every_runs || now - state.decided_time >= policy.decide_every_secs {
            return Ok(true);
        }
        // A plan hold that lapsed: the plan may be dead now (a memo whose writer holds its plans, the slice's).
        if lapsed_plan_hold(&connection, now)? {
            return Ok(true);
        }
        let recent = || manifests.iter().filter(|m| m.last_run > state.decided_run);
        // A root recorded since past its protected runs of a lane.
        let mut per_lane: HashMap<(i64, Option<[u8; 32]>), usize> = HashMap::new();
        for m in &manifests {
            *per_lane.entry((m.root, m.lane)).or_default() += 1;
        }
        if recent().any(|m| per_lane[&(m.root, m.lane)] > policy.protected_runs) {
            return Ok(true);
        }
        // A policy set's newer lane (decision 0302): its older lanes lose their protection.
        Ok(recent().any(|m| m.policy.is_some() && manifests.iter().any(|o| o.root == m.root && o.policy == m.policy && o.lane != m.lane)))
    }

    /// Whether a plan hold has lapsed (`object_pins` of `OBJECT_PLAN` past their lease).
    fn lapsed_plan_hold(&self, now: i64) -> StoreResult<bool> {
        let connection = self.connection.lock().expect("content store connection unavailable");
        lapsed_plan_hold(&connection, now)
    }

    /// `corpus_state.due`: 0 nothing, 1 a new manifest (a decision is due), 2 a decision's rows being collected.
    fn corpus_due(&self) -> StoreResult<i64> {
        self.corpus_kept(|c| c.state().due)
    }

    /// Decides which manifests to keep under `policy` at unix second `now`, and applies it: evicted manifests go,
    /// `corpus_live` becomes what the kept ones name, and plans no kept manifest names are marked dead.
    /// Defers with an empty report while a reader or another collector holds the collection lease.
    pub fn keep_corpus_runs(&self, policy: &CorpusPolicy, now: i64) -> StoreResult<CorpusKept> {
        let Some(_lease) = self.try_collect()? else { return Ok(CorpusKept::default()) };
        self.keep_corpus_runs_held(policy, now)
    }

    fn keep_corpus_runs_held(&self, policy: &CorpusPolicy, now: i64) -> StoreResult<CorpusKept> {
        // Read, decided and applied in one transaction (O95): a manifest or plan another run records meanwhile waits
        // for the write lock, so a decision never drops what it did not see.
        let (kept, written) = self.immediate_transaction(|tx| {
            Self::discard_abandoned_analysis(tx)?;
            self.corpus_write(tx, |c, w| {
            let manifests: Vec<Manifest> = c
                .manifests()
                .into_iter()
                .map(|m| Manifest {
                    id: m.id,
                    root: m.root,
                    last_time: m.last_time,
                    grammar_version: m.grammar_version,
                    tape_format: m.tape_format,
                    plan: m.plan.to_vec(),
                    lane: m.lane.map(|l| l.to_vec()),
                    policy: m.policy.map(|p| p.to_vec()),
                    sources: c.sources(m.id),
                })
                .collect();
            let scoped: HashSet<i64> = tx.prepare(
                "SELECT DISTINCT a.manifest_id FROM analysis_scope_manifest a JOIN corpus_manifests m USING(manifest_id) \
                 WHERE a.scope GLOB 'named:*' OR (a.scope GLOB 'cwd:*' AND NOT EXISTS (\
                    SELECT 1 FROM analysis_scope_manifest b JOIN corpus_manifests newer USING(manifest_id) \
                    WHERE b.scope=a.scope AND newer.root_id=m.root_id AND newer.policy IS m.policy \
                      AND (m.policy IS NOT NULL OR newer.lane IS m.lane)
                      AND (b.acquired_at,b.acquisition,b.recorded_run)>(a.acquired_at,a.acquisition,a.recorded_run)))",
            )?
                .query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
            // Bytes of each live source, and of its tape of the live grammar version and format.
            let mut sizes = Sizes::default();
            let mut live = HashSet::new();
            {
                let mut rows = tx.prepare(
                    "SELECT l.source_id, l.grammar_version, l.tape_format, coalesce(s.seg_len, 0), \
                     coalesce((SELECT t.seg_len FROM tape_objects t WHERE t.source_id=l.source_id \
                               AND t.grammar_version=l.grammar_version AND t.tape_format_version=l.tape_format), 0) \
                     FROM corpus_live l LEFT JOIN source_objects s ON s.source_id=l.source_id",
                )?;
                let mut q = rows.query([])?;
                while let Some(r) = q.next()? {
                    let key: (i64, i64, i64) = (r.get(0)?, r.get(1)?, r.get(2)?);
                    sizes.source.insert(key.0, r.get::<_, i64>(3)? as u64);
                    sizes.tape.insert(key, r.get::<_, i64>(4)? as u64);
                    live.insert(key);
                }
            }
            // A root whose directory is gone keeps nothing once it has been idle `missing_secs`: its manifests are
            // left out of the decision (so evicted), and its stat index and row go with them.
            let gone_roots: HashSet<i64> = {
                let mut last: HashMap<i64, i64> = HashMap::new();
                for m in &manifests {
                    let t = last.entry(m.root).or_insert(m.last_time);
                    *t = (*t).max(m.last_time);
                }
                let owned_roots: HashSet<_> = manifests.iter().filter(|m| scoped.contains(&m.id)).map(|m| m.root).collect();
                let roots = c.roots();
                roots
                    .into_iter()
                    .filter(|(id, path)| !owned_roots.contains(id) && last.get(id).is_none_or(|&t| now - t >= policy.missing_secs) && !std::path::Path::new(path).exists())
                    .map(|(id, _)| id)
                    .collect()
            };
            let present: Vec<Manifest>;
            let considered: &[Manifest] = if gone_roots.is_empty() {
                &manifests
            } else {
                present = manifests.iter().filter(|m| !gone_roots.contains(&m.root)).map(Manifest::clone).collect();
                &present
            };
            let mut aged = Aged {
                bytes: tx.query_row(
                    "SELECT coalesce(sum(aged_bytes),0) FROM store_sessions WHERE kind IN (?1,?2) AND ended IS NOT NULL",
                    [super::reclamation::SESSION_WORKSPACE, super::reclamation::SESSION_PINS],
                    |r| r.get::<_, i64>(0),
                )? as u64,
                ..Aged::default()
            };
            {
                let mut rows = tx.prepare(
                    "SELECT f.source_id, t.source_id, t.grammar_version, t.tape_format_version FROM workspaces w \
                     JOIN snapshot_files f ON f.snapshot_id=w.snapshot_id LEFT JOIN tape_objects t ON t.tape_id=f.tape_id \
                     WHERE w.state_id=2 \
                     UNION SELECT v.source_id, t.source_id, t.grammar_version, t.tape_format_version FROM workspaces w \
                     JOIN workspace_overlay v ON v.workspace_id=w.workspace_id LEFT JOIN tape_objects t ON t.tape_id=v.tape_id \
                     WHERE w.state_id=2 \
                     UNION SELECT CASE p.object_kind_id WHEN ?2 THEN p.object_id END, t.source_id, t.grammar_version, t.tape_format_version \
                     FROM store_sessions e JOIN object_pins p ON p.owner_string_id=e.key \
                     LEFT JOIN tape_objects t ON p.object_kind_id=?3 AND t.tape_id=p.object_id \
                     WHERE e.kind=?1 AND e.ended IS NOT NULL",
                )?;
                let mut q = rows.query([super::reclamation::SESSION_PINS, super::reclamation::OBJECT_SOURCE, super::reclamation::OBJECT_TAPE])?;
                while let Some(r) = q.next()? {
                    if let Some(source) = r.get::<_, Option<i64>>(0)? {
                        aged.sources.insert(source);
                    }
                    if let (Some(s), Some(g), Some(f)) = (r.get(1)?, r.get(2)?, r.get(3)?) {
                        aged.tapes.insert((s, g, f));
                    }
                }
            }
            let decided = decide(considered, &sizes, policy, now, &aged);
            let kept_ids: HashSet<i64> = decided.kept.iter().copied().chain(scoped.iter().copied()).collect();
            let mut keep_live: HashSet<(i64, i64, i64)> = HashSet::new();
            let mut keep_plans: BTreeSet<Vec<u8>> = BTreeSet::new();
            for m in manifests.iter().filter(|m| kept_ids.contains(&m.id)) {
                keep_live.extend(m.sources.iter().map(|&s| (s, m.grammar_version, m.tape_format)));
            }
            // A plan is live while a root's protected runs name it (or a run holds it). A kept manifest past them
            // keeps its sources and tapes (any build's warm run reads them), not its plan: a plan names the engine
            // build and the program, and its memo (decision 0146: a space of entries, the size of every file's
            // results) serves only runs of both. Manifests are kept by the bytes of their sources and tapes, which
            // a small corpus never fills, so every build's memo stayed live (1.75 GB of 1.86 GB of frames).
            let mut protection = Protection::over(&manifests);
            let protected_ids: HashSet<i64> = manifests
                .iter()
                .filter(|m| kept_ids.contains(&m.id))
                .filter(|m| scoped.contains(&m.id) || protection.protects(m, policy.protected_runs.max(1), policy.protected_lanes.max(1)))
                .map(|m| m.id)
                .collect();
            for m in manifests.iter().filter(|m| protected_ids.contains(&m.id)) {
                keep_plans.insert(m.plan.clone());
            }
            // The other plans those manifests name (`record_corpus_run_naming`).
            for m in manifests.iter().filter(|m| protected_ids.contains(&m.id)) {
                keep_plans.extend(c.named(m.id).iter().map(|p| p.to_vec()));
            }
            // Each lane is a store session (decision 0450 §2, kind 2): its protected runs are its latest data, and a lane
            // none of whose runs is protected ended at its last run. Rewritten by each decision from the kept manifests.
            {
                // A lane (its root and name): its latest manifest, that one's last use, and whether a run of it is protected.
                type Lane<'a> = (i64, Option<&'a [u8]>);
                let mut lanes: HashMap<Lane<'_>, (i64, i64, bool)> = HashMap::new();
                for m in manifests.iter().filter(|m| kept_ids.contains(&m.id)) {
                    // Newest first: a lane's first manifest is its latest.
                    let lane = lanes.entry((m.root, m.lane.as_deref())).or_insert((m.id, m.last_time, false));
                    lane.2 |= protected_ids.contains(&m.id);
                }
                let rows: Vec<[Option<i64>; 4]> = lanes
                    .iter()
                    .map(|(&(root, lane), &(latest, last, protected))| [Some(lane_session(root, lane)), Some(latest), Some(last), (!protected).then_some(last)])
                    .collect();
                tx.execute_cached("DELETE FROM store_sessions WHERE kind=?1", [super::reclamation::SESSION_LANE])?;
                tx.prepare_cached(
                    "INSERT INTO store_sessions(kind,key,latest,last_used,ended,aged_bytes) SELECT ?1, json_extract(value,'$[0]'), \
                     json_extract(value,'$[1]'), json_extract(value,'$[2]'), json_extract(value,'$[3]'), 0 FROM json_each(?2)",
                )?
                .execute(params![super::reclamation::SESSION_LANE, serde_json::to_string(&rows).expect("integers")])?;
            }
            let unlive: Vec<&(i64, i64, i64)> = live.iter().filter(|k| !keep_live.contains(k)).collect();
            let evicted: Vec<i64> = manifests.iter().map(|m| m.id).filter(|id| !kept_ids.contains(id)).collect();
            let evicted_json = serde_json::to_string(&evicted).expect("integers");
            // Their memberships go with them (as `ON DELETE CASCADE` took them with the table's rows).
            tx.prepare_cached("DELETE FROM analysis_scope_manifest WHERE manifest_id IN (SELECT value FROM json_each(?1))")?.execute([&evicted_json])?;
            for &id in &evicted {
                w.manifest(id, None);
            }
            let mut unlive_of: BTreeMap<i64, Vec<(i64, i64)>> = BTreeMap::new();
            for &&(s, g, f) in &unlive {
                unlive_of.entry(s).or_default().push((g, f));
            }
            for (source, gone) in unlive_of {
                let left: Vec<(i64, i64)> = c.live_of(source).into_iter().filter(|t| !gone.contains(t)).collect();
                w.live(source, &left);
            }
            // A plan a kept manifest names, or a run holds (`hold_corpus_plan`), is live; any other recorded plan is dead.
            let held: HashSet<i64> = tx
                .prepare("SELECT object_id FROM object_pins WHERE object_kind_id=?1 AND lease_until>=?2")?
                .query_map(params![OBJECT_PLAN, now], |r| r.get(0))?
                .collect::<Result<_, _>>()?;
            for (plan, _) in c.plans() {
                w.plan(plan, Some(keep_plans.contains(&plan[..]) || held.contains(&plan_object(&plan))));
            }
            for &root in &gone_roots {
                w.root(root, None);
            }
            let mut state = c.state();
            (state.decided_run, state.decided_time) = (state.run, now);
            // Collection stays due until a pass finds nothing (`collect_corpus_pass`), unless a new manifest came first.
            if state.due <= 1 {
                (state.due, state.history_bytes) = (2, decided.history as i64);
            }
            // What it left aged may be archived (decision 0450 step 4).
            state.archive_due = 1;
            w.state(state);
            Ok(CorpusKept { manifests: kept_ids.len(), evicted: evicted.len(), protected_bytes: decided.protected, history_bytes: decided.history, unlived: unlive.len() })
            })
        })?;
        self.note_kept(&kept::space(), written);
        Ok(kept)
    }

    /// Whether a keep decision was made since the memo entries it left unreachable were collected (`sweep_memo`).
    pub fn memo_sweep_wanted(&self) -> StoreResult<bool> {
        let connection = self.connection.lock().expect("content store connection unavailable");
        Ok(memo_sweep_wanted(&connection)?)
    }

    /// Collects the memo entries the last keep decision left unreachable (decision 0227), and records that decision as
    /// swept: one write transaction. What it collected, and the seconds it took; `None` when `stop` interrupted it
    /// (a run began: a progress handler's `SQLITE_INTERRUPT`, the transaction rolled back, the sweep still due).
    pub fn sweep_memo(&self, stop: impl Fn() -> bool + Send + Sync + 'static) -> StoreResult<Option<(usize, f64)>> {
        let stop: std::sync::Arc<dyn Fn() -> bool + Send + Sync> = std::sync::Arc::new(stop);
        // Its writes delete rows and set a mark, naming no atom: a forked process sweeps too (decision 0170).
        super::atoms::mapped(|| {
            let started = std::time::Instant::now();
            let out = self.immediate_transaction_until(std::sync::Arc::clone(&stop), |tx| {
                let out = super::held::collect_unreachable_memo(tx)?;
                let ((), written) = self.corpus_write(tx, |c, w| {
                    let mut state = c.state();
                    state.swept = state.decided_run;
                    w.state(state);
                    Ok(())
                })?;
                Ok((out, written))
            });
            match out {
                Ok((out, written)) => {
                    self.note_kept(&kept::space(), written);
                    Ok(Some((out, started.elapsed().as_secs_f64())))
                }
                Err(StoreError::Database(e)) if super::upkeep::interrupted(&e) => Ok(None),
                Err(e) => Err(e),
            }
        })
    }

    /// One bounded pass: the keep decision when one is due, up to `PLAN_ROWS_PER_PASS` memo rows of dead plans, and
    /// the store's collection (`collect`).
    /// Active readers defer the pass without clearing the pending collection flag.
    pub fn collect_corpus_pass(&self, policy: &CorpusPolicy, now: i64) -> StoreResult<CorpusPass> {
        // Its writes delete rows and set flags, naming no atom: a forked process collects too (decision 0170).
        super::atoms::mapped(|| self.collect_corpus_pass_unmapped(policy, now))
    }

    fn collect_corpus_pass_unmapped(&self, policy: &CorpusPolicy, now: i64) -> StoreResult<CorpusPass> {
        let Some(_lease) = self.try_collect()? else { return Ok(CorpusPass::default()) };
        let mut pass = CorpusPass::default();
        // Holds of runs whose process is gone are released: their plans may be dead now, so a decision is due.
        let orphans = {
            let connection = self.connection.lock().expect("content store connection unavailable");
            orphaned_holds(&connection)?
        };
        for owner in &orphans {
            self.release_pins(owner)?;
        }
        let orphaned = !orphans.is_empty();
        let due = match self.corpus_due()? {
            _ if orphaned => 1,
            0 if self.lapsed_plan_hold(now)? => 1,
            due => due,
        };
        if due == 1 {
            if !orphaned && !self.corpus_decision_worth(policy, now)? {
                // Nothing a decision could change: no root went past its protected runs, and the last decision is
                // recent. The sweep waits for the next decision too; a new manifest makes collection due again.
                self.corpus_state_with(|s| if s.due == 1 { s.due = 0 })?;
                return Ok(pass);
            }
            pass.kept = Some(self.keep_corpus_runs_held(policy, now)?);
            // What the decision left live reaches the memo entries it keeps; the rest go, by the memo sweep (a
            // utility job: soon, behind the run, and finished before the process exits; decision 0227).
            rerail_accel::upkeep::kick();
        }
        // The kept spaces' entries only dead programs read (decision 0401 F4), while their plans are still known dead.
        pass.plan_rows += self.collect_kept_facts()?;
        // Dead plans, one transaction each, until the pass's budget is spent: a dead build's per-file memo is its
        // entries' space (decision 0146), and its kept entries went with it above (`collect_kept_facts`).
        let mut budget = PLAN_ROWS_PER_PASS as usize;
        while budget > 0 {
            let (n, written) = self.immediate_transaction(|tx| {
                self.corpus_write(tx, |c, w| {
                    let Some((plan, _)) = c.plans().into_iter().find(|p| !p.1) else { return Ok(0) };
                    let memo_gone = super::held::drop_memo_space(tx, &super::held::local_space(&plan))?;
                    w.plan(plan, None);
                    // A dead plan collected is work done, though its memo is entries in segments (decision 0240 §4), not rows.
                    Ok(memo_gone + 1)
                })
            })?;
            self.note_kept(&kept::space(), written);
            pass.plan_rows += n;
            if n == 0 {
                break;
            }
            budget = budget.saturating_sub(n);
        }
        pass.collection = self.collect_held(now, MAX_COLLECTION_BATCH)?;
        // The memo's pack, once the entries collected left most of it unnamed (decision 0240 §4).
        pass.plan_rows += usize::from(self.compact_memo_pack(MEMO_PACK_FLOOR)? > 0);
        // The tapes' and sources' segments, once files stored again or collected left most of them unnamed (0295 step 5).
        pass.plan_rows += usize::from(self.compact_object_segments(MEMO_PACK_FLOOR)? > 0);
        if !pass.did_work() && self.corpus_due()? == 2 {
            // The same lease protects the live-plan snapshot and the observer sweep. Leave the
            // pass due until both stores have completed it, including after an interrupted pass.
            let live = self.live_plans()?;
            pass.observer = Some(super::observe::collect(&super::observe::store_beside(self.database_path()), |plan| live.contains(plan))?);
            self.corpus_state_with(|s| if s.due == 2 { s.due = 0 })?;
        }
        Ok(pass)
    }

    /// Converts a store made before decision 0053 (`auto_vacuum` never took) to incremental vacuum: a full `VACUUM`,
    /// which rewrites the file. True if it converted. For maintenance that holds the store alone (`rerail-engine gc`).
    pub fn convert_to_incremental_vacuum(&self) -> StoreResult<bool> {
        let connection = self.connection.lock().expect("content store connection unavailable");
        let mode: i64 = connection.query_row_cached("PRAGMA auto_vacuum", [], |r| r.get(0))?;
        if mode == 2 {
            return Ok(false);
        }
        connection.execute_batch("PRAGMA auto_vacuum = INCREMENTAL; VACUUM;")?;
        Ok(true)
    }
}

#[derive(Default)]
struct Sizes {
    source: HashMap<i64, u64>,
    tape: HashMap<(i64, i64, i64), u64>,
}

struct Decided {
    kept: Vec<i64>,
    protected: u64,
    /// The aged bytes kept: the history's and the ended workspaces' (`Aged`).
    history: u64,
}

/// What ended workspaces and pin owners hold while they are aged (decision 0450 step 3): one budget holds every store session's aged
/// data, and theirs is counted first. Its bytes count against the history's marks, and its sources and tapes count for
/// no manifest again.
#[derive(Default)]
struct Aged {
    bytes: u64,
    sources: HashSet<i64>,
    tapes: HashSet<(i64, i64, i64)>,
}

/// The manifests to keep, `manifests` newest first (the module's rules).
fn decide(manifests: &[Manifest], sizes: &Sizes, policy: &CorpusPolicy, now: i64, aged: &Aged) -> Decided {
    let walk = |cap: Option<u64>| -> Decided {
        let mut protection = Protection::over(manifests);
        let (mut seen_sources, mut seen_tapes) = (aged.sources.clone(), aged.tapes.clone());
        let mut out = Decided { kept: Vec::new(), protected: 0, history: aged.bytes };
        for m in manifests {
            let protected = protection.protects(m, policy.protected_runs, policy.protected_lanes);
            if now - m.last_time > policy.backstop_secs {
                continue;
            }
            // The bytes this manifest holds that no newer kept manifest does.
            let mut own = 0;
            let mut fresh = Vec::new();
            for &s in &m.sources {
                let tape = (s, m.grammar_version, m.tape_format);
                if !seen_tapes.contains(&tape) {
                    own += sizes.tape.get(&tape).copied().unwrap_or(0);
                    fresh.push(tape);
                }
                if !seen_sources.contains(&s) {
                    own += sizes.source.get(&s).copied().unwrap_or(0);
                }
            }
            let keep = if protected {
                out.protected += own;
                true
            } else if fresh.is_empty() {
                false
            } else if cap.is_none_or(|cap| out.history + own <= cap) {
                out.history += own;
                true
            } else {
                false
            };
            if keep {
                out.kept.push(m.id);
                seen_sources.extend(m.sources.iter().copied());
                seen_tapes.extend(fresh);
            }
        }
        out
    };
    let all = walk(None);
    if all.history > policy.high_water { walk(Some(policy.low_water)) } else { all }
}

/// A corpus lane's store session key (decision 0450 §2, kind 2): the first eight bytes of a digest of its root and
/// lane, so the key is the lane's durable name.
fn lane_session(root: i64, lane: Option<&[u8]>) -> i64 {
    let mut h = Sha256::new();
    h.update(root.to_le_bytes());
    h.update(lane.unwrap_or(&[]));
    i64::from_le_bytes(h.finalize()[..8].try_into().expect("eight bytes"))
}

/// Sorted ids as delta varints.
pub(in crate::content_store) fn encode_ids(ids: &[i64]) -> Vec<u8> {
    let mut out = Vec::with_capacity(ids.len() * 2);
    let mut last = 0i64;
    for &id in ids {
        let mut d = (id - last) as u64;
        last = id;
        loop {
            let byte = (d & 0x7f) as u8;
            d >>= 7;
            if d == 0 {
                out.push(byte);
                break;
            }
            out.push(byte | 0x80);
        }
    }
    out
}

pub(super) fn decode_ids(bytes: &[u8]) -> StoreResult<Vec<i64>> {
    let mut out = Vec::new();
    let (mut last, mut d, mut shift) = (0i64, 0u64, 0u32);
    for &b in bytes {
        if shift > 63 {
            return Err(StoreError::Corrupt("a corpus manifest's source ids overflow".into()));
        }
        d |= u64::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            last += d as i64;
            out.push(last);
            (d, shift) = (0, 0);
        } else {
            shift += 7;
        }
    }
    if shift != 0 {
        return Err(StoreError::Corrupt("a corpus manifest's source ids are truncated".into()));
    }
    Ok(out)
}

pub(super) mod kept;
use kept::KeptManifest;

#[cfg(test)]
mod tests;


/// Whether `corpus_decided.run` is past its `swept` on connection `c` (a read: the memo sweep's `due`).
pub fn memo_sweep_wanted(c: &rusqlite::Connection) -> StoreResult<bool> {
    let store = std::path::PathBuf::from(c.path().unwrap_or_default());
    kept::with_corpus(&store, c, |k| {
        let state = k.state();
        state.decided_run > state.swept
    })
}
