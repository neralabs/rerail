//! The stream program's interpreter (`09-compile-and-lower.md` §4.1): one operation at a time over
//! every file of a batch, the files one after another in a byte space (`space::NodeSpace`, a byte
//! per node), as `prefilter/sets.rs` `evaluate` runs the set program over a wave's node space.
//! Shifts and scans restart at each file's first byte, so the batch is one stream (06 §5.1).
//!
//! Word operations are the reference evaluator's (`mask.rs`). The scans that have no word form
//! yet (machines, reflections, counts, frames, hypothesis lanes) walk their events or bytes here;
//! their branch-free lowerings are `03-bitlex.md` §3.2 and §3.5.

use super::mask::Mask;
use super::program::{HypOp, Id, Op, Program, Seg};
use super::reference::Token;
use crate::code_rule_dsl::space::NodeSpace;

#[derive(Clone)]
enum Val {
    M(Mask),
    C(Vec<i32>),
    Ms(Vec<Mask>),
}

type R<T> = Result<T, String>;

/// Each file's tokens and errors, in the reference evaluator's terms.
pub(in crate::code_rule_dsl) type FileOut = (Vec<Token>, Vec<String>);

struct Run<'a> {
    space: NodeSpace,
    src: Vec<u8>,
    vals: Vec<Option<Val>>,
    errors: Vec<Vec<String>>,
    prog: &'a Program,
    /// Each file's first byte, outside the program's values.
    file_first: Mask,
    /// Per `k`, the bytes at each residue mod `k` of their position in the space.
    residues: std::collections::HashMap<u32, Vec<Mask>>,
}

/// Runs `prog` over `files` as one batch.
pub(in crate::code_rule_dsl) fn run(prog: &Program, files: &[&[u8]]) -> R<Vec<FileOut>> {
    let space = NodeSpace::new(files.iter().map(|f| f.len() as u32).collect());
    let src: Vec<u8> = files.concat();
    let mut file_first = Mask::zeros(space.len() as usize);
    (0..space.files()).filter(|&f| space.count(f) > 0).for_each(|f| file_first.set(space.base(f) as usize));
    let mut r = Run { space, src, vals: vec![None; prog.ops.len()], errors: vec![vec![]; files.len()], prog, file_first, residues: Default::default() };
    r.exec()?;
    let mut out = vec![];
    for f in 0..files.len() {
        let (base, count) = (r.space.base(f) as usize, r.space.count(f) as usize);
        let mut errors = std::mem::take(&mut r.errors[f]);
        let get = |id: Id| slice(r.m(id), base, count);
        let toks: Vec<(String, Mask, Option<Mask>)> = prog.tokens.iter().map(|t| (t.kind.clone(), get(t.start), t.through.map(get))).collect();
        let munch: Vec<_> = prog.munch.iter().map(|(k, over, lits)| (k.clone(), get(*over), lits.clone())).collect();
        let tokens = super::finish::finish(files[f], &toks, &munch, &prog.keywords, &get(prog.skip), &mut errors);
        out.push((tokens, errors));
    }
    Ok(out)
}

/// CPU seconds by operation kind, when `BITLEX_OP_TIMES` is set.
pub(in crate::code_rule_dsl) static OP_TIMES: std::sync::Mutex<std::collections::BTreeMap<String, f64>> =
    std::sync::Mutex::new(std::collections::BTreeMap::new());

fn timing() -> bool {
    static ON: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var_os("BITLEX_OP_TIMES").is_some());
    *ON
}

/// This thread's CPU time in seconds.
fn thread_cpu() -> f64 {
    let mut t = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) };
    t.tv_sec as f64 + t.tv_nsec as f64 * 1e-9
}

/// `count` bits of `m` from bit `from`, as a mask of their own (`set_steps::extract`).
fn slice(m: &Mask, from: usize, count: usize) -> Mask {
    let mut out = Mask::zeros(count);
    for (at, slot) in out.w.iter_mut().enumerate() {
        let bit = from + at * 64;
        let (word, shift) = (bit / 64, bit % 64);
        let low = m.w.get(word).copied().unwrap_or(0) >> shift;
        let high = if shift == 0 { 0 } else { m.w.get(word + 1).copied().unwrap_or(0) << (64 - shift) };
        let left = count - at * 64;
        *slot = (low | high) & if left >= 64 { u64::MAX } else { (1 << left) - 1 };
    }
    out
}

/// The set bits of `m` in `range`, ascending.
fn ones_in(m: &Mask, range: std::ops::Range<usize>) -> impl Iterator<Item = usize> + '_ {
    let words = if range.is_empty() { 0..0 } else { range.start / 64..(range.end - 1) / 64 + 1 };
    words
        .flat_map(move |j| {
            let mut x = m.w[j];
            std::iter::from_fn(move || {
                if x == 0 {
                    return None;
                }
                let b = x.trailing_zeros() as usize;
                x &= x - 1;
                Some(j * 64 + b)
            })
        })
        .filter(move |i| range.contains(i))
}

impl Run<'_> {
    fn m(&self, id: Id) -> &Mask {
        match &self.vals[id] {
            Some(Val::M(m)) => m,
            Some(_) => panic!("value {id} is not a mask"),
            None => panic!("value {id} ({:?}) is not computed", self.prog.ops[id]),
        }
    }
    fn c(&self, id: Id) -> &Vec<i32> {
        match &self.vals[id] {
            Some(Val::C(c)) => c,
            _ => panic!("value {id} is not a count"),
        }
    }
    fn n(&self) -> usize {
        self.space.len() as usize
    }
    fn range(&self, f: usize) -> std::ops::Range<usize> {
        let r = self.space.range(f);
        r.start as usize..r.end as usize
    }
    fn file_of(&self, i: usize) -> usize {
        let mut lo = 0;
        let mut hi = self.space.files();
        while lo + 1 < hi {
            let mid = (lo + hi) / 2;
            if self.space.base(mid) as usize <= i { lo = mid } else { hi = mid }
        }
        lo
    }

    fn exec(&mut self) -> R<()> {
        let last = self.prog.last_uses();
        let mut frees: Vec<Vec<Id>> = vec![vec![]; self.prog.ops.len() + 1];
        for (id, &l) in last.iter().enumerate() {
            if self.prog.live[id] && l < self.prog.ops.len() {
                frees[l].push(id);
            }
        }
        // Per loop (by its `RoundsBegin`): the round, and each file's errors on entry.
        let mut loops: std::collections::HashMap<usize, (usize, Vec<usize>)> = Default::default();
        let mut pc = 0;
        while pc < self.prog.ops.len() {
            if !self.prog.live[pc] {
                pc += 1;
                continue;
            }
            let op = &self.prog.ops[pc];
            let mut jump = None;
            let v = match op {
                Op::RoundsBegin => {
                    // Entered from before the loop: a jump back goes past it.
                    loops.insert(pc, (1, self.errors.iter().map(Vec::len).collect()));
                    None
                }
                Op::RoundsEnd { begin, n, carries, label, in_hyp } => {
                    let mut changed = Mask::zeros(self.n());
                    for &(c, nx) in carries {
                        changed = changed.or(&self.m(c).xor(self.m(nx)));
                    }
                    let (r, marks) = loops.get(begin).cloned().ok_or("a loop's end before its start")?;
                    if !changed.any() {
                        loops.remove(begin);
                        Some(Val::M(Mask::zeros(self.n())))
                    } else if r == *n {
                        loops.remove(begin);
                        if *in_hyp {
                            Some(Val::M(changed))
                        } else {
                            for f in 0..self.space.files() {
                                if ones_in(&changed, self.range(f)).next().is_some() {
                                    self.errors[f].push(format!("{label} did not settle in {n}"));
                                }
                            }
                            Some(Val::M(Mask::zeros(self.n())))
                        }
                    } else {
                        for &(c, nx) in carries {
                            let v = self.m(nx).clone();
                            self.vals[c] = Some(Val::M(v));
                        }
                        for (e, &m) in self.errors.iter_mut().zip(&marks) {
                            e.truncate(m);
                        }
                        loops.insert(*begin, (r + 1, marks));
                        jump = Some(*begin + 1);
                        None
                    }
                }
                _ => {
                    let t0 = timing().then(thread_cpu);
                    let v = self.op(op)?;
                    if let Some(t0) = t0 {
                        let kind = format!("{op:?}");
                        let kind = kind.split(['(', ' ', '{']).next().unwrap_or("").to_string();
                        *OP_TIMES.lock().unwrap().entry(kind).or_default() += thread_cpu() - t0;
                    }
                    Some(v)
                }
            };
            if let Some(v) = v {
                self.vals[pc] = Some(v);
            }
            match jump {
                Some(to) => pc = to,
                None => {
                    for &id in &frees[pc] {
                        self.vals[id] = None;
                    }
                    pc += 1;
                }
            }
        }
        Ok(())
    }

    /// `prev(a, k)` cut at segment starts: no bit moves past a segment's first byte.
    fn prev(&self, a: &Mask, k: usize, seg: Option<Seg>) -> Mask {
        let m = a.prev(k);
        match seg {
            Some(s) if k > 0 => m.andnot(&self.m(s.first).span(k)),
            _ => m,
        }
    }
    fn next(&self, a: &Mask, k: usize, seg: Option<Seg>) -> Mask {
        let m = a.next(k);
        match seg {
            Some(s) if k > 0 => {
                let last = self.m(s.last);
                let mut cut = last.clone();
                for j in 1..k {
                    cut = cut.or(&last.next(j));
                }
                m.andnot(&cut)
            }
            _ => m,
        }
    }

    fn op(&mut self, op: &Op) -> R<Val> {
        let n = self.n();
        Ok(Val::M(match op {
            Op::Empty => Mask::zeros(n),
            Op::All => Mask::ones(n),
            Op::Or(a, b) => self.m(*a).or(self.m(*b)),
            Op::And(a, b) => self.m(*a).and(self.m(*b)),
            Op::AndNot(a, b) => self.m(*a).andnot(self.m(*b)),
            Op::Xor(a, b) => self.m(*a).xor(self.m(*b)),
            Op::Not(a) => self.m(*a).not(),
            Op::Within(scope, test) => {
                let mut out = self.m(*scope).clone();
                for f in 0..self.space.files() {
                    let r = self.range(f);
                    if ones_in(self.m(*test), r.clone()).next().is_none() {
                        out.set_range(r, false);
                    }
                }
                out
            }
            Op::FileFirst => self.file_first.clone(),
            Op::FileLast => {
                let mut m = Mask::zeros(n);
                (0..self.space.files()).filter(|&f| self.space.count(f) > 0).for_each(|f| m.set(self.range(f).end - 1));
                m
            }
            Op::Class(items) => {
                // A table of the 256 byte values, then 64 lookups per word.
                let mut lut = [0u64; 256];
                for (b, slot) in lut.iter_mut().enumerate() {
                    *slot = items.iter().any(|&(lo, hi)| lo <= b as u32 && b as u32 <= hi) as u64;
                }
                let mut m = Mask::zeros(n);
                for (w, chunk) in m.w.iter_mut().zip(self.src.chunks(64)) {
                    *w = chunk.iter().enumerate().fold(0, |acc, (k, &c)| acc | lut[c as usize] << k);
                }
                m
            }
            Op::Ucode { over, classes } => return Ok(Val::Ms(self.ucode(*over, classes))),
            Op::Pick(id, k) => match &self.vals[*id] {
                Some(Val::Ms(v)) => v[*k].clone(),
                _ => return Err(format!("pick {k} of {id}: not a multi-mask")),
            },
            Op::Prev { a, k, seg } => self.prev(self.m(*a), *k, *seg),
            Op::Next { a, k, seg } => self.next(self.m(*a), *k, *seg),
            Op::Toggle { a, seg } => {
                // The prefix XOR over the space, then each segment's incoming parity undone: a
                // fill from the segment starts that inherit a 1.
                let t = self.m(*a).toggle();
                let first = self.m(seg.first);
                let x = first.and(&t.prev(1));
                t.xor(&Mask::fill(&x, first))
            }
            Op::Fill { s, e, seg } => Mask::fill(self.m(*s), &self.m(*e).or(self.m(seg.first))),
            Op::BackFill { s, e, seg } => Mask::back_fill(self.m(*s), &self.m(*e).or(self.m(seg.last))),
            Op::Aligned { a, k, seg } => {
                if !self.residues.contains_key(k) {
                    let r = (0..*k).map(|c| Mask::from_fn(n, |i| i as u32 % k == c)).collect();
                    self.residues.insert(*k, r);
                }
                // Runs whose start is at residue c mod k, by a fill from their starts; a byte is
                // aligned when it is at its run's residue.
                let a = self.m(*a);
                if *k <= 1 {
                    return Ok(Val::M(a.clone()));
                }
                let rs = a.andnot(&self.prev(a, 1, Some(*seg)));
                let na = a.not();
                let mut out = Mask::zeros(n);
                for p in &self.residues[k] {
                    let r = Mask::fill(&rs.and(&p), &na.or(&rs.andnot(&p)));
                    out = out.or(&r.and(p));
                }
                out.and(a)
            }
            Op::LineHas { a, nl, seg } => {
                let (a, nl) = (self.m(*a), self.m(*nl));
                let f = Mask::fill(&a.andnot(nl), &nl.or(self.m(seg.first)));
                nl.and(&a.or(&self.prev(&f, 1, Some(*seg))))
            }
            Op::Machine { events, table, start, seg } => return Ok(Val::Ms(self.machine(events, table, *start, *seg))),
            Op::Reflect { a, b, reset, start, seg } => return Ok(Val::Ms(self.reflect(*a, *b, *reset, *start, *seg))),
            Op::Count { up, down, reset, seg } => {
                let (u, d, first) = (self.m(*up), self.m(*down), self.m(seg.first));
                let reset = reset.map(|r| self.m(r));
                let mut v = vec![0i32; n];
                let mut x = 0i32;
                for (i, slot) in v.iter_mut().enumerate() {
                    if first.get(i) {
                        x = 0
                    }
                    if reset.is_some_and(|r| r.get(i)) {
                        x = 0
                    } else if u.get(i) {
                        x += 1
                    } else if d.get(i) && x > 0 {
                        x -= 1
                    }
                    *slot = x;
                }
                return Ok(Val::C(v));
            }
            Op::CountTest { c, k, other, ge } => {
                let c = self.c(*c);
                let o = other.map(|o| self.c(o));
                Mask::from_fn(n, |i| {
                    let r = o.map(|o| o[i]).unwrap_or(*k);
                    if *ge { c[i] >= r } else { c[i] == r }
                })
            }
            Op::Frame { c, m, since, seg } => {
                let (c, m, first) = (self.c(*c), self.m(*m), self.m(seg.first));
                let mut out = Mask::zeros(n);
                let mut stack: Vec<bool> = vec![];
                let mut before = 0i32;
                for i in 0..n {
                    if first.get(i) {
                        stack.clear();
                        before = 0
                    }
                    if stack.last() == Some(&true) {
                        out.set(i)
                    }
                    let after = c[i];
                    if after > before {
                        stack.push(!since && m.get(i));
                    } else if after < before {
                        stack.truncate(after.max(0) as usize);
                    } else if *since && m.get(i) {
                        if let Some(t) = stack.last_mut() {
                            *t = true
                        }
                    }
                    before = after;
                }
                out
            }
            Op::HeadCount { a, c, seg } => {
                let (a, c, first) = (self.m(*a), self.c(*c), self.m(seg.first));
                // The count before a run's first byte; 0 at a file's first byte.
                let files = &self.file_first;
                let mut out = Mask::zeros(n);
                let (mut off, mut lim) = (0usize, 0usize);
                for i in 0..n {
                    if !a.get(i) {
                        continue;
                    }
                    if files.get(i) || !a.get(i - 1) || first.get(i) {
                        off = 0;
                        lim = if files.get(i) { 0 } else { c[i - 1].max(0) as usize };
                    } else {
                        off += 1
                    }
                    if off < lim {
                        out.set(i)
                    }
                }
                out
            }
            Op::CountSelect(pairs) => {
                let mut v = vec![0i32; n];
                for &(c, sel) in pairs {
                    let (c, sel) = (self.c(c), self.m(sel));
                    for i in sel.ones_iter() {
                        v[i] = c[i];
                    }
                }
                return Ok(Val::C(v));
            }
            Op::Carry(init) => self.m(*init).clone(),
            Op::Hyp(h) => return Ok(Val::Ms(self.hyp(h)?)),
            Op::Error(msg, a) => {
                let a = self.m(*a).clone();
                let mut seen = usize::MAX;
                for i in a.ones_iter() {
                    let f = self.file_of(i);
                    if f != seen {
                        seen = f;
                        let at = i - self.space.base(f) as usize;
                        self.errors[f].push(format!("{msg} (byte {at})"));
                    }
                }
                Mask::zeros(0)
            }
            Op::RoundsBegin | Op::RoundsEnd { .. } => unreachable!("control is run by exec"),
        }))
    }

    fn ucode(&self, over: Id, classes: &[Vec<(u32, u32)>]) -> Vec<Mask> {
        let n = self.n();
        let over = self.m(over);
        let mut ms: Vec<Mask> = classes.iter().map(|_| Mask::zeros(n)).collect();
        for f in 0..self.space.files() {
            let r = self.range(f);
            let mut cursor = r.start;
            for i in ones_in(over, r.clone()) {
                if i < cursor {
                    continue;
                }
                let (cp, len) = decode(&self.src[i..r.end]);
                for (k, c) in classes.iter().enumerate() {
                    if c.iter().any(|&(lo, hi)| lo <= cp && cp <= hi) {
                        for j in i..(i + len).min(r.end) {
                            ms[k].set(j)
                        }
                    }
                }
                cursor = i + len;
            }
        }
        ms
    }

    /// A machine's walk over its event bytes, file by file; each state's bytes by runs.
    fn machine(&self, events: &[Id], table: &[Vec<usize>], start: usize, seg: Seg) -> Vec<Mask> {
        let n = self.n();
        let evs: Vec<&Mask> = events.iter().map(|&e| self.m(e)).collect();
        let mut any = Mask::zeros(n);
        for e in &evs {
            any = any.or(e);
        }
        let mut ins: Vec<Mask> = table.iter().map(|_| Mask::zeros(n)).collect();
        let _ = seg;
        for f in 0..self.space.files() {
            let r = self.range(f);
            let (mut x, mut from) = (start, r.start);
            for i in ones_in(&any, r.clone()) {
                ins[x].set_range(from..i, true);
                let k = evs.iter().position(|m| m.get(i)).unwrap();
                x = table[x][k];
                from = i;
            }
            ins[x].set_range(from..r.end, true);
        }
        ins.push(any);
        ins
    }

    /// A reflection's walk over the bytes where its state can change; each state's bytes by runs.
    fn reflect(&self, a: Id, b: Id, reset: Option<Id>, start: u8, seg: Seg) -> Vec<Mask> {
        let n = self.n();
        let (a, b, first) = (self.m(a), self.m(b), self.m(seg.first));
        let reset = reset.map(|r| self.m(r));
        let mut events = a.or(b).or(first);
        if let Some(r) = reset {
            events = events.or(r);
        }
        let mut ins = vec![Mask::zeros(n), Mask::zeros(n), Mask::zeros(n)];
        let mut ev = Mask::zeros(n);
        let (mut x, mut from) = (0u8, 0usize);
        for i in events.ones_iter() {
            ins[x as usize].set_range(from..i, true);
            if first.get(i) {
                x = start
            }
            if reset.is_some_and(|r| r.get(i)) {
                x = 0
            } else if a.get(i) {
                x = (4 - x) % 3;
                ev.set(i)
            } else if b.get(i) {
                x = (5 - x) % 3;
                ev.set(i)
            }
            from = i;
        }
        ins[x as usize].set_range(from..n, true);
        ins.push(ev);
        ins
    }

    /// Each segment's lane, by a walk over the segments of each file (a scan of lane maps when
    /// lowered, `08-without-automata.md` §7); errors where a lane rejects its segment.
    fn hyp(&mut self, h: &HypOp) -> R<Vec<Mask>> {
        let n = self.n();
        let first = self.m(h.first).clone();
        let mut sel: Vec<Mask> = h.lanes.iter().map(|_| Mask::zeros(n)).collect();
        for f in 0..self.space.files() {
            let r = self.range(f);
            let starts: Vec<usize> = ones_in(&first, r.clone()).collect();
            if starts.is_empty() {
                continue;
            }
            let end_of = |k: usize| if k + 1 < starts.len() { starts[k + 1] - 1 } else { r.end - 1 };
            let seg_of = |i: usize| starts.partition_point(|&s| s <= i) - 1;
            let mut rejects: Vec<Vec<Option<&String>>> = vec![vec![None; starts.len()]; h.lanes.len()];
            for (l, list) in h.rejects.iter().enumerate() {
                for (msg, m) in list {
                    for i in ones_in(self.m(*m), r.clone()) {
                        let k = seg_of(i);
                        if rejects[l][k].is_none() {
                            rejects[l][k] = Some(msg);
                        }
                    }
                }
            }
            let mut lane = 0usize;
            for k in 0..starts.len() {
                for i in starts[k]..=end_of(k) {
                    sel[lane].set(i)
                }
                if let Some(msg) = rejects[lane][k] {
                    self.errors[f].push(format!("{}: {msg} (byte {})", h.name, starts[k] - r.start));
                    lane = 0;
                } else {
                    let (ins, map) = &h.exits[lane];
                    let e = end_of(k);
                    let state = ins.iter().position(|&m| self.m(m).get(e)).ok_or("a scan in no state")?;
                    lane = map[state].ok_or(format!("hypotheses {}: exit state is no lane", h.name))?;
                }
            }
            if lane != 0 {
                self.errors[f].push(format!("{}: the file ends in lane {}", h.name, h.lanes[lane]));
            }
        }
        Ok(sel)
    }
}

/// One UTF-8 character: its code point and length. An invalid sequence is one byte, U+FFFD.
fn decode(b: &[u8]) -> (u32, usize) {
    let c = b[0];
    let (len, init) = match c {
        0x00..=0x7f => return (c as u32, 1),
        0xc0..=0xdf => (2, (c & 0x1f) as u32),
        0xe0..=0xef => (3, (c & 0x0f) as u32),
        0xf0..=0xf7 => (4, (c & 0x07) as u32),
        _ => return (0xfffd, 1),
    };
    if b.len() < len || !b[1..len].iter().all(|&x| x & 0xc0 == 0x80) {
        return (0xfffd, 1);
    }
    (b[1..len].iter().fold(init, |a, &x| (a << 6) | (x & 0x3f) as u32), len)
}
