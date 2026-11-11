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
//!   executor decides how many run at once (none with one thread). Their count tables and seed joins go out first
//!   as one device batch (§3, step 6). `RERAIL_STRATA_PARALLEL=off`: components run on the loop's thread, at
//!   their retire.
//! - **Retire** (§4): in program order, on the loop's thread. A member that declined runs as it would have (its
//!   SQL), one after it on the loop's thread.

use super::*;
use rerail_accel::stream::{Handle, Stream};

/// What a component's job gives back: its strand, how many members ran natively, and what stopped the next.
pub(super) type Out<'a> = (Strand<'a>, usize, Option<Result<(Option<String>, Option<String>)>>, Vec<(Instant, Instant)>, crate::content_store::atoms::Reserved);
use std::time::Instant;

enum State<'a> {
    Job(Handle<Out<'a>>),
    Inline(Strand<'a>, Vec<(BTreeSet<String>, Vec<&'a Rule>)>),
    Done(usize, Option<Result<(Option<String>, Option<String>)>>),
}

struct Component<'a> {
    members: Vec<usize>,
    own: BTreeSet<String>,
    digests: Vec<u64>,
    estimate: f64,
    state: State<'a>,
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
    pending: Vec<u32>,
    readers: Vec<Vec<usize>>,
    retired: Vec<bool>,
    ready: BTreeSet<usize>,
    /// Strata below it were looked at for retiring; those then waiting on their wave are `held`.
    to: usize,
    held: Vec<usize>,
    member_of: Vec<Option<usize>>,
    comps: Vec<Component<'a>>,
    /// The components submitted whose jobs are not collected yet.
    flying: Vec<usize>,
    in_flight: usize,
    k: f64,
    fixed: f64,
    pub parallel: bool,
    pub counts: Counts,
    /// Each stratum's producers (`deps`), for the trace.
    from: Vec<Vec<usize>>,
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
    /// Why a stratum ran on the loop's thread, if it was looked at.
    why: std::cell::RefCell<Vec<&'static str>>,
}

fn env_f(name: &str, or: f64) -> f64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(or)
}

/// Whether strata are issued ahead at all (`RERAIL_STRANDS=off`: the serial loop).
pub(super) fn on() -> bool {
    !matches!(std::env::var("RERAIL_STRANDS").as_deref(), Ok("off"))
}

impl<'a> Board<'a> {
    pub(super) fn new(sccs: &[BTreeSet<String>], deps: &super::super::front::Edges) -> Self {
        let of: HashMap<&str, usize> = sccs.iter().enumerate().flat_map(|(j, scc)| scc.iter().map(move |r| (r.as_str(), j))).collect();
        let mut pending = vec![0u32; sccs.len()];
        let mut readers = vec![Vec::new(); sccs.len()];
        let mut producers: Vec<Vec<usize>> = Vec::with_capacity(sccs.len());
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
            held: Vec::new(),
            member_of: vec![None; sccs.len()],
            comps: Vec::new(),
            flying: Vec::new(),
            in_flight: 0,
            k: if merge { env_f("RERAIL_STRATA_K", 8.0) } else { 0.0 },
            fixed: env_f("RERAIL_STRATA_FIXED", 136e-6),
            parallel: !matches!(std::env::var("RERAIL_STRATA_PARALLEL").as_deref(), Ok("off")),
            counts: Counts::default(),
            from: producers,
            trace: std::env::var_os("RERAIL_STRATA_TRACE").map(|_| Trace { t0: Instant::now(), at: None, by_component: false, rows: vec![None; sccs.len()], why: std::cell::RefCell::new(vec![""; sccs.len()]) }),
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
    /// component and its producers.
    pub(super) fn dump(&self, level_of: &[usize], sccs: &[BTreeSet<String>]) {
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
            out.push_str(&format!("s\t{j}\t{}\t{a:.7}\t{b:.7}\t{w}\t{comp}\t{}\t{}\t{why}\n", level_of[j], from.join(","), sccs[j].iter().next().map_or("", String::as_str)));
        }
        let _ = f.write_all(out.as_bytes());
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

    /// A stratum's estimate: the fixed cost and its rules' reads' rows.
    fn estimate(&self, run: &Run<'a, '_>, rules: &[&Rule]) -> f64 {
        let rows: f64 = rules.iter().flat_map(|r| r.body.iter().filter_map(Literal::atom)).map(|a| run.model.stats.card.get(a.rel.as_str()).copied().unwrap_or(1.0)).sum();
        self.fixed + rows * 20e-9
    }

    /// Issues the components of the strata ready from `at` on (§2, §3): `at` too, which then retires at once. `rules_of` gives a stratum's rules as the loop
    /// orders them; `load` binds a stratum's inputs early (before its first reader, as the loop would).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn issue<'s>(
        &mut self,
        at: usize,
        run: &mut Run<'a, '_>,
        sccs: &[BTreeSet<String>],
        rules_of: &dyn Fn(usize) -> Vec<&'a Rule>,
        wants_of: &[BTreeSet<(String, Vec<usize>)>],
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
        let mut issued: Vec<usize> = Vec::new();
        while self.in_flight < IN_FLIGHT {
            let Some(j) = self.ready.range(at..).next().copied() else { break };
            self.ready.remove(&j);
            let none = BTreeSet::new();
            let Some(first) = self.eligible(j, run, sccs, rules_of, load, &none)? else { continue };
            let mut members = vec![j];
            let mut held: BTreeSet<String> = sccs[j].clone();
            let mut reads = first;
            let mut est = self.estimate(run, &rules_of(j));
            let mut local: HashMap<usize, u32> = HashMap::new();
            let mut chain: BTreeSet<usize> = BTreeSet::new();
            let mut grow = |m: usize, local: &mut HashMap<usize, u32>, chain: &mut BTreeSet<usize>, this: &Self| {
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
                held.extend(sccs[next].iter().cloned());
                reads.extend(more);
                grow(next, &mut local, &mut chain, self);
            }
            members.sort_unstable();
            let rules: Vec<Vec<&'a Rule>> = members.iter().map(|&m| rules_of(m)).collect();
            let digests: Vec<u64> = rules.iter().flatten().map(|r| obs_rule(&r.head, &r.body)).collect();
            let all: Vec<&Rule> = rules.iter().flatten().copied().collect();
            // A component whose statistics cannot be made for its strand runs on the loop's thread, as it would.
            let Ok(strand) = run.strand(&reads, &digests, &Model::read_by(all.iter().copied())) else {
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
            let data: Vec<(BTreeSet<String>, Vec<&'a Rule>)> = members.iter().map(|&m| sccs[m].clone()).zip(rules).collect();
            issued.push(c);
            self.comps.push(Component { members, own: held, digests, estimate: est, state: State::Inline(strand, data) });
        }
        if issued.is_empty() {
            return Ok(());
        }
        // The issued components' count tables and seed joins, as one device batch (§3, step 6).
        let mut wants: BTreeSet<(String, Vec<usize>)> = BTreeSet::new();
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
        if !self.parallel {
            return Ok(());
        }
        for c in issued {
            let est = self.comps[c].estimate;
            let State::Inline(strand, data) = std::mem::replace(&mut self.comps[c].state, State::Done(0, None)) else { unreachable!() };
            self.flying.push(c);
            self.comps[c].state = State::Job(stream.submit(est, move || {
                let _in = ambient.enter();
                let mut strand = strand;
                let mut times = Vec::new();
                let mut reserved = Default::default();
                let (n, stop) = strand.run_members(&data, &mut times, &mut reserved);
                (strand, n, stop, times, reserved)
            }));
        }
        Ok(())
    }

    /// Whether `j` may be issued with the relations `held` (its component's earlier members') taken as held: the
    /// relations its strand holds (`Run::strand_reads`).
    #[allow(clippy::too_many_arguments)]
    fn eligible(&self, j: usize, run: &Run<'a, '_>, sccs: &[BTreeSet<String>], rules_of: &dyn Fn(usize) -> Vec<&'a Rule>, load: &mut dyn FnMut(usize) -> Result<()>, held: &BTreeSet<String>) -> Result<Option<BTreeSet<String>>> {
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
        Ok(())
    }

    /// Waits for component `c`'s job (or runs it here) and absorbs it; its members that ran retire.
    fn finish<'s>(&mut self, c: usize, run: &mut Run<'a, '_>, stream: &Stream<'s, '_>) -> Result<()> {
        let comp = &mut self.comps[c];
        let (strand, n, stop, times, reserved, inline) = match std::mem::replace(&mut comp.state, State::Done(0, None)) {
            State::Job(h) => {
                let (s, n, stop, times, reserved) = stream.wait(h).map_err(Error::Plan)?;
                (s, n, stop, times, reserved, false)
            }
            State::Inline(mut strand, data) => {
                let mut times = Vec::new();
                let mut reserved = Default::default();
                let (n, stop) = strand.run_members(&data, &mut times, &mut reserved);
                (strand, n, stop, times, reserved, true)
            }
            State::Done(..) => return Ok(()),
        };
        // Its strand's relations are those its members that ran made: the others' are not held.
        run.absorb(strand, &comp.own, &comp.digests);
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
        let ran: Vec<usize> = self.comps[c].members[..n].to_vec();
        ran.into_iter().for_each(|m| self.retire(m));
        Ok(())
    }

    /// The stratum at `at`, as its component left it: waited for (or run here) when it is the component's first.
    /// While it waits, the components that end are collected and what they make ready is issued (`more`).
    pub(super) fn retire_at<'s, 'r>(&mut self, at: usize, run: &mut Run<'a, 'r>, stream: &Stream<'s, '_>, more: &mut dyn FnMut(&mut Self, &mut Run<'a, 'r>) -> Result<()>) -> Result<Got> {
        let Some(c) = self.member_of[at] else { return Ok(Got::Not) };
        let mut nap = 20e-6f64;
        loop {
            match &self.comps[c].state {
                State::Done(..) => break,
                State::Job(h) if !stream.done(h) && !stream.queued(h) => {}
                _ => {
                    self.flying.retain(|&x| x != c);
                    self.finish(c, run, stream)?;
                    break;
                }
            }
            let before = self.counts.components;
            self.collect(run, stream)?;
            more(self, run)?;
            if self.counts.components != before || stream.help() {
                nap = 20e-6;
                continue;
            }
            std::thread::sleep(std::time::Duration::from_secs_f64(nap));
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
