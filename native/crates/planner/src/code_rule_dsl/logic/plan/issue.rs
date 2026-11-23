//! Strata issued ahead of the strata loop (decision 0113): a scoreboard of what each stratum still waits for, ready
//! strata packed into components by their estimates, and components run as jobs on the executor's host stream,
//! each on its own strand (`Strand`: its own model, the run's scope, no SQL). The strata loop stays the program
//! order: it retires each stratum at its position, and a component's at its first member's.
//!
//! - **The scoreboard** (§1): per stratum, how many of the strata it reads (`deps`) have not retired, and its
//!   readers. A stratum the loop has passed retires, unless it waits on its device wave (then once its wave ran).
//! - **Components** (§2): ready strata in program order, with the strata that become ready through the
//!   component's own earlier members (a chain), until the estimate is `k` fixed costs (`RERAIL_STRATA_K`, 8; the
//!   fixed cost `RERAIL_STRATA_FIXED`, 136 µs: 2.8 s over 20,589 strata). One strand per component: one fork of the
//!   statistics, one prime of the scope. `RERAIL_STRATA_MERGE=off`: one stratum per component.
//! - **Issue** (§3): the components of one look-ahead are submitted to the host stream with their estimates; the
//!   executor decides how many run at once (none with one thread). Decision 0282: each with its rank, its members'
//!   longest chain of estimates to the program's end (`levels`), so the stream claims the critical path first. Their count tables and seed joins go out first
//!   as one device batch (§3, step 6). `RERAIL_STRATA_PARALLEL=off`: components run on the loop's thread, at
//!   their retire.
//! - **Retire** (§4): in program order, on the loop's thread. A member that declined runs as it would have (its
//!   SQL), one after it on the loop's thread.

use rerail_blocks::vector::Vector;
use super::*;
use rerail_accel::stream::{Handle, Stream};
use std::time::Instant;

/// What a component's job gives back: its strand, how many members ran natively, and what stopped the next.
pub(super) type Out<'a> = (Strand<'a>, usize, Option<Result<(Option<String>, Option<String>)>>, Vec<(Instant, Instant)>, crate::content_store::atoms::Reserved, Parked);

/// Whether the components issued together share one device wave (decision 0123 §6). `RERAIL_LEVEL_WAVE=off` flushes
/// one a component, as before (a measurement aid).
fn level_wave() -> bool {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_LEVEL_WAVE").map_or(true, |v| v != "off"))
}

/// A component's members.
type Members = Vec<(crate::code_rule_dsl::logic::front::CompAt, Vec<RuleKey>)>;

/// What a component's job leaves waiting on the run's wave (decision 0123 §6): its members still waiting on their
/// device wave, the launches they queued on the job's thread, and its members' rules (to resume them).
pub(super) struct Parked {
    waiting: Vec<(usize, Box<Driving>)>,
    queued: super::super::lanes::wave::Queued,
    data: Members,
}

// SAFETY: as its strand (`Strand`): the drivers are the strand's state, made on the job's thread and resumed on one
// thread at a time, only after the job that made them ended; the kernels hold shared, immutable buffers.
unsafe impl Send for Parked {}

/// A component whose members wait on the run's wave: its job ended, it is not absorbed yet.
struct Waiting<'a> {
    /// Its waiting members' launches (their outputs go to the job that resumes them).
    tickets: Vector<u64>,
    strand: Strand<'a>,
    n: usize,
    stop: Option<Result<(Option<String>, Option<String>)>>,
    times: Vec<(Instant, Instant)>,
    reserved: crate::content_store::atoms::Reserved,
    inline: bool,
    waiting: Vec<(usize, Box<Driving>)>,
    data: Members,
}

// SAFETY: as `Parked`'s: a waiting component is resumed by one job at a time, after the one that parked it ended.
unsafe impl Send for Waiting<'_> {}

#[allow(clippy::large_enum_variant)]
enum State<'a> {
    Job(Handle<Out<'a>>),
    Inline(Strand<'a>, Members),
    /// Waiting on the run's wave (`flush_group`).
    Parked(Box<Waiting<'a>>),
    Done(usize, Option<Result<(Option<String>, Option<String>)>>),
}

struct Component<'a> {
    members: Vector<usize>,
    own: crate::code_rule_dsl::logic::front::Components,
    digests: Vector<u64>,
    estimate: f64,
    /// Decision 0282: the longest chain of estimates from its members to the program's end (`Board::level`).
    rank: f64,
    state: State<'a>,
    /// The `issue` that made it: the components issued together share one device wave (decision 0123 §6).
    group: usize,
}

/// What a stratum the loop reaches gets from its component.
pub(super) enum Got {
    /// Not issued: the loop runs it.
    Not,
    Ran,
    Declined(Option<String>, Option<String>),
    /// The loop runs it after all (its component stopped before it, or it read SQL).
    Here,
}

/// The counts the explain output shows (§5).
#[derive(Default, Debug)]
pub(super) struct Counts {
    pub components: usize,
    pub members: usize,
    pub largest: usize,
    pub ran: usize,
    pub here: usize,
    /// Atoms the components made, published at their retire.
    pub atoms: usize,
}

pub(super) struct Board<'a> {
    pending: Vector<u32>,
    readers: Vec<Vector<usize>>,
    retired: Vec<bool>,
    ready: BTreeSet<usize>,
    /// Strata below it were looked at for retiring; those then waiting on their wave are `held`.
    to: usize,
    held: Vector<usize>,
    member_of: Vec<Option<usize>>,
    comps: Vec<Component<'a>>,
    /// The components submitted whose jobs are not collected yet.
    flying: Vector<usize>,
    /// The strata whose components' jobs ended and ran them, since the loop last took them (`take_finished`): their
    /// reads are done, so a relation whose every reader is among them is released now, not when the loop reaches its
    /// last reader.
    finished: Vector<usize>,
    in_flight: usize,
    k: f64,
    fixed: f64,
    pub parallel: bool,
    pub counts: Counts,
    /// How many `issue`s made components (`Component::group`).
    groups: usize,
    /// Each stratum's producers (`deps`), for the trace.
    from: Vec<Vector<usize>>,
    /// Decision 0282: each stratum's bottom level, its estimate and the longest chain of its readers' to the
    /// program's end; made at the first `issue` (`levels`).
    level: Vector<f64>,
    /// `RERAIL_STRATA_TRACE`: the run's start, the stratum the loop is at (since when, and whether its component
    /// ran it), and each stratum's start, end and where it ran (0 the loop's thread, 1 a job, 2 a component run at
    /// its retire).
    trace: Option<Trace>,
}

struct Trace {
    t0: Instant,
    at: Option<(usize, Instant)>,
    by_component: bool,
    rows: Vec<Option<(f64, f64, u8)>>,
    /// When each stratum's component was submitted to the stream (`RERAIL_STRATA_TRACE`'s last column), or -1.
    submitted: Vector<f64>,
    /// Why a stratum ran on the loop's thread, if it was looked at.
    why: std::cell::RefCell<Vec<&'static str>>,
    /// The loop thread's seconds by what it did (`SPENT`), for the trace's `loop` lines.
    spent: std::cell::RefCell<[f64; 8]>,
}

const SPENT: [&str; 8] = ["issue: pack", "issue: statistics", "issue: device waves", "issue: submit", "collect", "help", "pause", "retire: wait or run"];

fn env_f(name: &str, or: f64) -> f64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(or)
}

/// Whether strata are issued ahead at all (`RERAIL_STRANDS=off`: the serial loop).
pub(super) fn on() -> bool {
    !matches!(std::env::var("RERAIL_STRANDS").as_deref(), Ok("off"))
}

impl<'a> Board<'a> {
    pub(super) fn new(sccs: &crate::code_rule_dsl::logic::front::Components, deps: &super::super::front::Edges) -> Self {
        let of: HashMap<&str, usize> = sccs.iter().enumerate().flat_map(|(j, scc)| scc.iter().map(move |r| (r, j))).collect();
        let mut pending = rerail_blocks::vector![0u32; sccs.len()];
        let mut readers = vec![Vector::new(); sccs.len()];
        let mut producers: Vec<Vector<usize>> = Vec::with_capacity(sccs.len());
        for (j, scc) in sccs.iter().enumerate() {
            let from: BTreeSet<usize> = scc.iter().flat_map(|r| deps.get(r).into_iter().flatten()).filter_map(|(d, _)| of.get(d.as_str()).copied()).filter(|&p| p != j).collect();
            pending[j] = from.len() as u32;
            for &p in &from {
                readers[p].push(j);
            }
            producers.push(from.into_iter().collect());
        }
        let ready = (0..sccs.len()).filter(|&j| pending[j] == 0).collect();
        let merge = !matches!(std::env::var("RERAIL_STRATA_MERGE").as_deref(), Ok("off"));
        Board {
            pending,
            readers,
            retired: vec![false; sccs.len()],
            ready,
            to: 0,
            held: rerail_blocks::vector::Vector::new(),
            member_of: vec![None; sccs.len()],
            comps: Vec::new(),
            flying: rerail_blocks::vector::Vector::new(),
            finished: rerail_blocks::vector::Vector::new(),
            in_flight: 0,
            k: if merge { env_f("RERAIL_STRATA_K", 8.0) } else { 0.0 },
            fixed: env_f("RERAIL_STRATA_FIXED", 136e-6),
            parallel: !matches!(std::env::var("RERAIL_STRATA_PARALLEL").as_deref(), Ok("off")),
            counts: Counts::default(),
            groups: 0,
            from: producers,
            level: rerail_blocks::vector::Vector::new(),
            // Always kept (a few words a stratum): the run's measured critical path reads it (decision 0208), and
            // `RERAIL_STRATA_TRACE` writes it.
            trace: Some(Trace { t0: Instant::now(), at: None, by_component: false, rows: vec![None; sccs.len()], submitted: rerail_blocks::vector![-1.0; sccs.len()], why: std::cell::RefCell::new(vec![""; sccs.len()]), spent: Default::default() }),
        }
    }

    /// Each stratum's start and end, in seconds since the run's, and where it ran (0 the loop's thread, 1 a job, 2 a
    /// component run at its retire); `None` where it did not run.
    pub(super) fn timeline(&self) -> Vec<Option<(f64, f64, u8)>> {
        self.trace.as_ref().map(|t| t.rows.clone()).unwrap_or_default()
    }

    /// The loop is done with the stratum it was at, before it waits for anything (answers, a device wave): the
    /// stratum ends now, and the wait is the gap before the next one.
    pub(super) fn close(&mut self) {
        let Some(t) = self.trace.as_mut() else { return };
        if let Some((p, since)) = t.at
            && !t.by_component
            && t.rows[p].is_none()
        {
            t.rows[p] = Some(((since - t.t0).as_secs_f64(), (Instant::now() - t.t0).as_secs_f64(), 0));
        }
    }

    /// The loop reaches `at` (`len`: it ended): the stratum before, if the loop's thread ran it, is traced.
    pub(super) fn mark(&mut self, at: usize) {
        let Some(t) = self.trace.as_mut() else { return };
        let now = Instant::now();
        if let Some((p, since)) = t.at.take()
            && !std::mem::take(&mut t.by_component)
            && t.rows[p].is_none()
        {
            t.rows[p] = Some(((since - t.t0).as_secs_f64(), (now - t.t0).as_secs_f64(), 0));
        }
        if at < t.rows.len() {
            t.at = Some((at, now));
        }
    }

    /// Appends the run's trace to `RERAIL_STRATA_TRACE`: per stratum its level, start, end, where it ran, its
    /// component, its producers, its name, why it ran on the loop's thread, and when its component was submitted.
    pub(super) fn dump(&self, level_of: &[usize], sccs: &crate::code_rule_dsl::logic::front::Components) {
        use std::io::Write as _;
        let Some(t) = &self.trace else { return };
        let Some(path) = std::env::var_os("RERAIL_STRATA_TRACE") else { return };
        let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) else { return };
        let mut out = format!("run\t{}\n", sccs.len());
        for (j, row) in t.rows.iter().enumerate() {
            let (a, b, w) = row.unwrap_or((-1.0, -1.0, 9));
            let comp = self.member_of[j].map_or(-1, |c| c as i64);
            let from: Vec<String> = self.from[j].iter().map(usize::to_string).collect();
            let why = t.why.borrow()[j];
            let why = if why.is_empty() && comp < 0 { "not offered" } else { why };
            out.push_str(&format!("s\t{j}\t{}\t{a:.7}\t{b:.7}\t{w}\t{comp}\t{}\t{}\t{why}\t{:.7}\n", level_of[j], from.join(","), sccs[j].first().unwrap_or(""), t.submitted[j]));
        }
        for (k, s) in t.spent.borrow().iter().enumerate() {
            out.push_str(&format!("loop\t{}\t{s:.7}\n", SPENT[k]));
        }
        let _ = f.write_all(out.as_bytes());
    }

    /// Adds the time since `t` to the loop thread's `k`th count (`SPENT`), and returns now.
    fn spend(&self, k: usize, t: Instant) -> Instant {
        let now = Instant::now();
        if let Some(tr) = &self.trace {
            tr.spent.borrow_mut()[k] += (now - t).as_secs_f64();
        }
        now
    }

    fn retire(&mut self, p: usize) {
        if std::mem::replace(&mut self.retired[p], true) {
            return;
        }
        for k in 0..self.readers[p].len() {
            let r = self.readers[p][k];
            self.pending[r] -= 1;
            if self.pending[r] == 0 && self.member_of[r].is_none() {
                self.ready.insert(r);
            }
        }
    }

    /// The loop is at `at`: the strata before it retire, but those waiting on their device wave.
    pub(super) fn advance(&mut self, at: usize, waiting: &[usize]) {
        let held = std::mem::take(&mut self.held);
        for p in held.into_iter().chain(self.to..at) {
            if waiting.contains(&p) { self.held.push(p) } else { self.retire(p) }
        }
        self.to = self.to.max(at);
        self.mark(at);
    }

    /// Decision 0282: each stratum's bottom level, as list scheduling ranks a task: its estimate, and the most of its
    /// readers'. A stratum on the run's critical path has the highest of those ready with it, so it is issued, and
    /// claimed by the stream, before larger strata off the path (the stream claimed the largest first, and the issue
    /// took program order).
    /// `RERAIL_STRATA_RANK=off` (a measurement aid): every stratum ranks 0, and the stream claims the largest first.
    fn levels(&self, run: &Run<'a, '_>, rules_of: &dyn Fn(usize) -> Vec<RuleKey>) -> Vector<f64> {
        let mut level = rerail_blocks::vector![0.0f64; self.readers.len()];
        if matches!(std::env::var("RERAIL_STRATA_RANK").as_deref(), Ok("off")) {
            return level;
        }
        for j in (0..level.len()).rev() {
            // Readers come later in program order; one that does not is in a cycle the strata already broke.
            let below = self.readers[j].iter().filter(|&&r| r > j).map(|&r| level[r]).fold(0.0, f64::max);
            level[j] = self.estimate(run, &rules_of(j)) + below;
        }
        level
    }

    /// A stratum's estimate: the fixed cost and its rules' reads' rows.
    fn estimate(&self, run: &Run<'a, '_>, rules: &[RuleKey]) -> f64 {
        // Summed in the order the reads come, one sum over every rule's (as the tree's reading summed them).
        let rows: f64 = rules.iter().flat_map(|&r| {
            let (v, rule) = run.versions.rule_at(r);
            rule.body().filter_map(|l| l.atom()).map(move |a| run.model.stats.card.get(v.rel_name(a.rel())).copied().unwrap_or(1.0))
        }).sum();
        self.fixed + rows * 20e-9
    }

    /// Issues the components of the strata ready from `at` on (§2, §3): `at` too, which then retires at once. `rules_of` gives a stratum's rules as the loop
    /// orders them; `load` binds a stratum's inputs early (before its first reader, as the loop would).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn issue<'s>(
        &mut self,
        at: usize,
        run: &mut Run<'a, '_>,
        sccs: &std::sync::Arc<crate::code_rule_dsl::logic::front::Components>,
        rules_of: &dyn Fn(usize) -> Vec<RuleKey>,
        wants_of: &[BTreeSet<(String, Vector<usize>)>],
        waved: &mut [bool],
        load: &mut dyn FnMut(usize) -> Result<()>,
        stream: &Stream<'s, '_>,
        ambient: &'s Ambient,
    ) -> Result<()>
    where
        'a: 's,
    {
        const IN_FLIGHT: usize = 256;
        const MEMBERS: usize = 64;
        if self.level.is_empty() {
            self.level = self.levels(run, rules_of);
        }
        if self.in_flight >= IN_FLIGHT || self.ready.range(at..).next().is_none() {
            return Ok(());
        }
        let mut t = Instant::now();
        let mut issued: Vector<usize> = rerail_blocks::vector::Vector::new();
        // Decision 0282: the ready strata are issued by rank, the stratum at `at` first, so the members in flight are
        // the ones the run's end waits on longest; in program order, the cap filled with strata off the path.
        let mut order: Vector<usize> = self.ready.range(at..).copied().collect();
        order.sort_by(|&a, &b| (b == at).cmp(&(a == at)).then(self.level[b].total_cmp(&self.level[a])).then(a.cmp(&b)));
        let mut order = order.into_iter();
        while self.in_flight < IN_FLIGHT {
            let Some(j) = order.by_ref().find(|j| self.ready.contains(j)) else { break };
            self.ready.remove(&j);
            let none = BTreeSet::new();
            let Some(first) = self.eligible(j, run, sccs, rules_of, load, &none)? else { continue };
            let mut members = rerail_blocks::vector![j];
            let mut held: BTreeSet<String> = sccs[j].names();
            let mut reads = first;
            let mut est = self.estimate(run, &rules_of(j));
            let mut local: HashMap<usize, u32> = HashMap::new();
            let mut chain: BTreeSet<usize> = BTreeSet::new();
            let grow = |m: usize, local: &mut HashMap<usize, u32>, chain: &mut BTreeSet<usize>, this: &Self| {
                for &r in &this.readers[m] {
                    let n = local.entry(r).or_default();
                    *n += 1;
                    if *n == this.pending[r] && this.member_of[r].is_none() && r > at {
                        chain.insert(r);
                    }
                }
            };
            grow(j, &mut local, &mut chain, self);
            while est < self.k * self.fixed && members.len() < MEMBERS {
                let next_ready = self.ready.range(at..).next().copied();
                let next = match (chain.first().copied(), next_ready) {
                    (Some(c), Some(r)) => c.min(r),
                    (Some(c), None) => c,
                    (None, Some(r)) => r,
                    (None, None) => break,
                };
                chain.remove(&next);
                self.ready.remove(&next);
                let Some(more) = self.eligible(next, run, sccs, rules_of, load, &held)? else { continue };
                est += self.estimate(run, &rules_of(next));
                members.push(next);
                held.extend(sccs[next].iter().map(str::to_owned));
                reads.extend(more);
                grow(next, &mut local, &mut chain, self);
            }
            members.sort_unstable();
            let rules: Vec<Vec<RuleKey>> = members.iter().map(|&m| rules_of(m)).collect();
            let digests: Vector<u64> = rules.iter().flatten().map(|&r| run.versions.at(r).observed_key()).collect();
            // Its statistics are not made here: the components issued together request theirs, made in one batch
            // below (decision 0123). A component whose strand cannot be made runs on the loop's thread, as it would.
            let Ok(strand) = run.strand(&reads, &digests, &[]) else {
                if let Some(t) = &self.trace {
                    members.iter().for_each(|&m| t.why.borrow_mut()[m] = "its statistics not made");
                }
                continue;
            };
            let c = self.comps.len();
            for &m in &members {
                self.member_of[m] = Some(c);
            }
            self.in_flight += members.len();
            self.counts.components += 1;
            self.counts.members += members.len();
            self.counts.largest = self.counts.largest.max(members.len());
            let data: Members = members.iter().map(|&m| crate::code_rule_dsl::logic::front::CompAt::of(sccs, m)).zip(rules).collect();
            issued.push(c);
            let rank = members.iter().map(|&m| self.level[m]).fold(0.0, f64::max);
            self.comps.push(Component { members, own: crate::code_rule_dsl::logic::front::Components::of_names([held.iter().map(String::as_str)]), digests, estimate: est, rank, state: State::Inline(strand, data), group: self.groups });
        }
        t = self.spend(0, t);
        if issued.is_empty() {
            return Ok(());
        }
        self.groups += 1;
        // Decision 0123: the statistics the issued components' plans will read, requested by each line that will be
        // priced (its seeds; its probes), made in one batch. The strands bind the same lazy statistics, so each
        // takes them as made; what a stratum's own rounds find still to make is made there.
        if run.model.has_deferred() {
            let mut wanted = BTreeSet::new();
            let strata: Vector<usize> = issued.iter().flat_map(|&c| self.comps[c].members.clone()).collect();
            let strata: Vec<(&crate::code_rule_dsl::logic::front::Comp, Vec<RuleKey>)> = strata.iter().map(|&m| (&sccs[m], run.runnable(&sccs[m], &rules_of(m)))).collect();
            let vs = Arc::clone(&run.versions);
            run.model.wants_strata(&vs, strata, &mut wanted)?;
            run.model.make_statistics(vs.whole(run.version), &wanted)?;
        }
        t = self.spend(1, t);
        // The issued components' count tables and seed joins, as one device batch (§3, step 6).
        let mut wants: BTreeSet<(String, Vector<usize>)> = BTreeSet::new();
        let mut specs = Vec::new();
        for &c in &issued {
            for k in 0..self.comps[c].members.len() {
                let m = self.comps[c].members[k];
                if !std::mem::replace(&mut waved[m], true) {
                    wants.extend(wants_of[m].iter().cloned());
                    specs.extend(run.seed_specs(&sccs[m], &rules_of(m)));
                }
            }
        }
        let db: &Connection = &run.db;
        if !wants.is_empty() {
            let _wave = super::super::profile::span(|| "device wave (count tables, issued components)".into());
            super::super::native::component::prefetch_tables(db, &wants)?;
        }
        if !specs.is_empty() {
            let _wave = super::super::profile::span(|| "device wave (joins, issued components)".into());
            super::super::native::component::prefetch_joins(db, &specs)?;
        }
        t = self.spend(2, t);
        if !self.parallel {
            return Ok(());
        }
        for c in issued {
            let (est, rank) = (self.comps[c].estimate, self.comps[c].rank);
            if let Some(t) = self.trace.as_mut() {
                let now = (Instant::now() - t.t0).as_secs_f64();
                self.comps[c].members.iter().for_each(|&m| t.submitted[m] = now);
            }
            let State::Inline(strand, data) = std::mem::replace(&mut self.comps[c].state, State::Done(0, None)) else { unreachable!() };
            self.flying.push(c);
            self.comps[c].state = State::Job(stream.submit_ranked(est, rank, move || {
                let _in = ambient.enter();
                let mut strand = strand;
                let mut times = Vec::new();
                let mut reserved = Default::default();
                let before = super::super::lanes::wave::flushed_now();
                let mark = super::super::lanes::wave::mark();
                let mut waiting = Vec::new();
                let (mut n, mut stop) = strand.run_members(&data, &mut times, &mut reserved, level_wave().then_some(&mut waiting));
                // A device joins' submission opened here, or a wave flushed here meanwhile, is this thread's: its
                // members' wave is flushed here (`kept_here`).
                if !waiting.is_empty() && super::super::lanes::wave::kept_here(mark) && let Err(e) = strand.flush_parked(&data, &mut waiting, &mut times) {
                    waiting.clear();
                    (n, stop) = (0, Some(Err(e)));
                }
                strand.2 = super::super::lanes::wave::since(before);
                // Its members waiting on their wave go back with their launches, for the level's wave.
                let queued = super::super::lanes::wave::take_queued(mark);
                (strand, n, stop, times, reserved, Parked { waiting, queued, data })
            }));
        }
        self.spend(3, t);
        Ok(())
    }

    /// Whether `j` may be issued with the relations `held` (its component's earlier members') taken as held: the
    /// relations its strand holds (`Run::strand_reads_with`).
    #[allow(clippy::too_many_arguments)]
    fn eligible(&self, j: usize, run: &Run<'a, '_>, sccs: &crate::code_rule_dsl::logic::front::Components, rules_of: &dyn Fn(usize) -> Vec<RuleKey>, load: &mut dyn FnMut(usize) -> Result<()>, held: &BTreeSet<String>) -> Result<Option<BTreeSet<String>>> {
        if sccs[j].iter().all(|r| run.views.contains(r)) {
            if let Some(t) = &self.trace {
                t.why.borrow_mut()[j] = "views only";
            }
            return Ok(None);
        }
        load(j)?;
        match run.strand_reads_with(&sccs[j], &rules_of(j), held) {
            Ok(reads) => Ok(Some(reads)),
            Err(why) => {
                if let Some(t) = &self.trace {
                    t.why.borrow_mut()[j] = why;
                }
                Ok(None)
            }
        }
    }

    /// Collects the components whose jobs ended, out of order (decision 0114 §1): each is absorbed and its members
    /// retire on the scoreboard now, so their readers are ready before the loop reaches them. A component writes only
    /// its own relations, so the order of absorbing is not the program's.
    pub(super) fn collect<'s>(&mut self, run: &mut Run<'a, '_>, stream: &Stream<'s, '_>) -> Result<()> {
        let t = Instant::now();
        let mut k = 0;
        while k < self.flying.len() {
            let c = self.flying[k];
            if matches!(&self.comps[c].state, State::Job(h) if stream.done(h)) {
                self.flying.swap_remove(k);
                self.finish(c, run, stream)?;
            } else {
                k += 1;
            }
        }
        self.spend(4, t);
        Ok(())
    }

    /// Waits for component `c`'s job (or runs it here) and absorbs it; its members that ran retire.
    fn finish<'s>(&mut self, c: usize, run: &mut Run<'a, '_>, stream: &Stream<'s, '_>) -> Result<()> {
        let comp = &mut self.comps[c];
        let (strand, n, stop, times, reserved, inline) = match std::mem::replace(&mut comp.state, State::Done(0, None)) {
            State::Job(h) => {
                let (s, n, stop, times, reserved, parked) = stream.wait(h).map_err(Error::Plan)?;
                if !parked.waiting.is_empty() {
                    let tickets = parked.queued.tickets();
                    super::super::lanes::wave::requeue(parked.queued);
                    comp.state = State::Parked(Box::new(Waiting { tickets, strand: s, n, stop, times, reserved, inline: false, waiting: parked.waiting, data: parked.data }));
                    return Ok(());
                }
                (s, n, stop, times, reserved, false)
            }
            State::Inline(mut strand, data) => {
                let mut times = Vec::new();
                let mut reserved = Default::default();
                let mut waiting = Vec::new();
                let (n, stop) = strand.run_members(&data, &mut times, &mut reserved, level_wave().then_some(&mut waiting));
                if !waiting.is_empty() {
                    comp.state = State::Parked(Box::new(Waiting { tickets: rerail_blocks::vector::Vector::new(), strand, n, stop, times, reserved, inline: true, waiting, data }));
                    return Ok(());
                }
                (strand, n, stop, times, reserved, true)
            }
            State::Parked(..) | State::Done(..) => return Ok(()),
        };
        self.taken(c, run, strand, n, stop, times, reserved, inline);
        Ok(())
    }

    /// Absorbs component `c`'s strand once its members ran (`finish`, `flush_group`); those that ran retire.
    #[allow(clippy::too_many_arguments)]
    fn taken(&mut self, c: usize, run: &mut Run<'a, '_>, strand: Strand<'a>, n: usize, stop: Option<Result<(Option<String>, Option<String>)>>, times: Vec<(Instant, Instant)>, reserved: crate::content_store::atoms::Reserved, inline: bool) {
        let comp = &mut self.comps[c];
        // Its strand's relations are those its members that ran made: the others' are not held.
        run.absorb(strand, &comp.own[0], &comp.digests);
        // Its relations are published: so are its atoms (decision 0114 §4).
        self.counts.atoms += reserved.len();
        drop(reserved);
        self.in_flight -= comp.members.len();
        self.counts.ran += n;
        if let Some(t) = self.trace.as_mut() {
            for (k, &m) in comp.members.iter().enumerate().take(n) {
                let (a, b) = times[k];
                t.rows[m] = Some(((a - t.t0).as_secs_f64(), (b - t.t0).as_secs_f64(), if inline { 2 } else { 1 }));
            }
        }
        comp.state = State::Done(n, stop);
        let ran: Vector<usize> = self.comps[c].members[..n].iter().copied().collect::<rerail_blocks::vector::Vector<_>>();
        self.finished.extend(ran.iter().copied());
        ran.into_iter().for_each(|m| self.retire(m));
    }

    /// The strata that ran in their components' jobs since the last call, in the order their jobs ended.
    pub(super) fn take_finished(&mut self) -> Vector<usize> {
        std::mem::take(&mut self.finished)
    }

    /// The device wave of the components issued with `c` (decision 0123 §6): each of them is waited for, so the
    /// launches its members left waiting are queued on this thread, and they go out as one wave. A component that ran
    /// as a job is resumed as a job, with its launches' outputs; one that ran here is resumed here. As often as some
    /// wait again, their launches go out as the next wave. Each then retires as `finish` would retire it.
    fn flush_group<'s>(&mut self, c: usize, run: &mut Run<'a, '_>, stream: &Stream<'s, '_>, ambient: &'s Ambient) -> Result<()>
    where
        'a: 's,
    {
        let group = self.comps[c].group;
        let mates: Vector<usize> = (0..self.comps.len()).filter(|&x| self.comps[x].group == group).collect();
        for &x in &mates {
            if matches!(self.comps[x].state, State::Job(_) | State::Inline(..)) {
                self.flying.retain(|&y| y != x);
                self.finish(x, run, stream)?;
            }
        }
        let mut parked: Vec<(usize, Box<Waiting<'a>>)> = Vec::new();
        for &x in &mates {
            if matches!(self.comps[x].state, State::Parked(_)) {
                let State::Parked(w) = std::mem::replace(&mut self.comps[x].state, State::Done(0, None)) else { unreachable!() };
                parked.push((x, w));
            }
        }
        // As `run_members` leaves a component whose wave failed: none of its members ran natively.
        let failed = |w: &mut Waiting<'a>, e: Error| {
            w.waiting.clear();
            w.n = 0;
            w.stop = Some(Err(e));
        };
        while !parked.is_empty() {
            let strata = parked.iter().map(|(_, w)| w.waiting.len()).sum();
            super::super::lanes::wave::flush(strata).map_err(Error::Plan)?;
            let mut back = Vec::new();
            let mut jobs = Vec::new();
            for (x, mut w) in parked {
                if w.inline || !self.parallel {
                    let Waiting { strand, waiting, data, times, reserved, .. } = &mut *w;
                    if let Err(e) = strand.reserving(reserved, |s| s.resume_parked(data, waiting, times)) {
                        failed(&mut w, e);
                    }
                    back.push((x, w));
                    continue;
                }
                let outs = super::super::lanes::wave::take_outputs(&std::mem::take(&mut w.tickets));
                jobs.push((x, stream.submit_ranked(self.comps[x].estimate, self.comps[x].rank, move || {
                    let _in = ambient.enter();
                    let mut w = w;
                    super::super::lanes::wave::put_outputs(outs);
                    let before = super::super::lanes::wave::flushed_now();
                    let mark = super::super::lanes::wave::mark();
                    let Waiting { strand, waiting, data, times, reserved, .. } = &mut *w;
                    let mut r = strand.reserving(reserved, |s| s.resume_parked(data, waiting, times));
                    // A device joins' submission opened here, or a wave flushed here meanwhile, is this thread's: its
                    // members' wave is flushed here (`kept_here`).
                    if r.is_ok() && !waiting.is_empty() && super::super::lanes::wave::kept_here(mark) {
                        r = strand.reserving(reserved, |s| s.flush_parked(data, waiting, times));
                    }
                    let waves = super::super::lanes::wave::since(before);
                    strand.2 = (strand.2.0 + waves.0, strand.2.1 + waves.1);
                    if let Err(e) = r {
                        failed(&mut w, e);
                    }
                    let queued = super::super::lanes::wave::take_queued(mark);
                    w.tickets = queued.tickets();
                    (w, queued)
                })));
            }
            for (x, h) in jobs {
                let (w, queued) = stream.wait(h).map_err(Error::Plan)?;
                super::super::lanes::wave::requeue(queued);
                back.push((x, w));
            }
            parked = Vec::new();
            for (x, w) in back {
                if w.waiting.is_empty() {
                    let Waiting { strand, n, stop, times, reserved, inline, .. } = *w;
                    self.taken(x, run, strand, n, stop, times, reserved, inline);
                } else {
                    parked.push((x, w));
                }
            }
        }
        Ok(())
    }

    /// The stratum at `at`, as its component left it: waited for (or run here) when it is the component's first.
    /// While it waits, the components that end are collected and what they make ready is issued (`more`).
    pub(super) fn retire_at<'s, 'r>(&mut self, at: usize, run: &mut Run<'a, 'r>, stream: &Stream<'s, '_>, ambient: &'s Ambient, more: &mut dyn FnMut(&mut Self, &mut Run<'a, 'r>) -> Result<()>) -> Result<Got>
    where
        'a: 's,
    {
        let Some(c) = self.member_of[at] else { return Ok(Got::Not) };
        let mut nap = 20e-6f64;
        loop {
            let seen = stream.ended();
            match &self.comps[c].state {
                State::Done(..) => break,
                State::Parked(_) => {
                    self.flush_group(c, run, stream, ambient)?;
                    break;
                }
                State::Job(h) if !stream.done(h) && !stream.queued(h) => {}
                _ => {
                    let t = Instant::now();
                    self.flying.retain(|&x| x != c);
                    self.finish(c, run, stream)?;
                    if matches!(self.comps[c].state, State::Parked(_)) {
                        self.flush_group(c, run, stream, ambient)?;
                    }
                    self.spend(7, t);
                    break;
                }
            }
            let before = self.counts.components;
            self.collect(run, stream)?;
            more(self, run)?;
            if self.counts.components != before {
                nap = 20e-6;
                continue;
            }
            let t = Instant::now();
            let helped = stream.help();
            let t = self.spend(5, t);
            if helped {
                nap = 20e-6;
                continue;
            }
            // Woken when a job ends (one of a component's, which `collect` then retires), not after a nap.
            stream.pause(seen, nap);
            self.spend(6, t);
            nap = (nap * 2.0).min(1e-3);
        }
        let comp = &mut self.comps[c];
        let k = comp.members.iter().position(|&m| m == at).expect("a member");
        let State::Done(n, stop) = &mut comp.state else { unreachable!() };
        if k < *n {
            if let Some(t) = self.trace.as_mut() {
                t.by_component = true;
            }
            return Ok(Got::Ran);
        }
        self.counts.here += 1;
        if let Some(t) = &self.trace {
            t.why.borrow_mut()[at] = if k == *n { match stop { Some(Ok(_)) => "declined", Some(Err(_)) => "read SQL or made an atom", None => "after one that stopped" } } else { "after one that stopped" };
        }
        if k == *n {
            match stop.take() {
                Some(Ok((why, class))) => return Ok(Got::Declined(why, class)),
                Some(Err(e)) if !off_thread(&e) => return Err(e),
                _ => {}
            }
        }
        Ok(Got::Here)
    }
}
