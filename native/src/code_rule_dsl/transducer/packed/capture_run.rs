//! CPU execution of proved normal sibling repetitions over capture ranges.
//! Finite bodies form DAGs. Current-node admission retires before the next node.
use super::*;
#[path = "capture_run_entry.rs"]
mod entry;
#[path = "capture_run_stream.rs"]
mod stream;
#[cfg(test)]
#[path = "capture_run_stream_tests.rs"]
mod stream_tests;

#[derive(Clone, Copy, Debug)]
pub(super) struct Limits {
    pub path: usize,
    pub task: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            path: MAX_STEPS_PER_PATH,
            task: MAX_WORK_PER_TASK,
        }
    }
}
#[derive(Default, Debug, Eq, PartialEq)]
pub(super) struct Report {
    pub task_work: usize,
    pub capture_events: usize,
    pub exit_records: usize,
    pub terminal_candidates: usize,
    pub admitted_headers: usize,
    pub duplicate_headers: usize,
    pub materialized_capture_records: usize,
}
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
#[repr(C)]
struct CaptureRange {
    /// Half-open offsets in the immutable capture-event arena.
    first: u32,
    last: u32,
}
#[derive(Clone, Copy, Debug)]
#[repr(C)]
struct Record {
    captures: CaptureRange,
    /// Inclusive sibling-domain positions. These are not tape node IDs.
    entry: u32,
    stop: u32,
    steps: u32,
}
#[derive(Clone, Debug)]
struct Body {
    first: usize,
    end: usize,
    paths: Vec<Vec<usize>>,
}
struct Proof {
    begin: usize,
    header: usize,
    tail_kind: usize,
    emit: usize,
    initial_named: bool,
    repeating_named: bool,
    tail_named: bool,
    post_kind: Vec<usize>,
    initial: Body,
    repeating: Body,
}
fn guard(op: u8) -> bool {
    matches!(
        op,
        KIND | FIELD | FLAGS | SIZE | FIRST | LAST | ABSENT_FIELD | SUPERTYPE
    )
}
fn body(program: &Program, first: usize, end: usize, header: usize, repeat: u16) -> Option<Body> {
    let mut pending = vec![(first, Vec::new(), false, false)];
    let mut paths = Vec::new();
    let mut incoming = BTreeMap::<usize, usize>::new();
    let mut edges = BTreeSet::new();
    while let Some((mut pc, mut path, mut captured, mut bumped)) = pending.pop() {
        loop {
            if pc == header {
                if !bumped {
                    return None;
                }
                paths.push(path);
                break;
            }
            if pc < first || pc >= end || path.len() >= MAX_FINITE_JOB_INSTRUCTIONS {
                return None;
            }
            let (op, mode, arg) = split(program.words[pc]);
            if bumped && op != GOTO {
                return None;
            }
            path.push(pc);
            let target = usize::from(arg);
            let mut next = pc + 1;
            match op {
                FORK if mode == 0 && target > pc && target < end => {
                    if edges.insert((pc, target)) {
                        *incoming.entry(target).or_default() += 1;
                    }
                    pending.push((target, path.clone(), captured, bumped));
                }
                GOTO if (target > pc && target < end) || (target == header && bumped) => {
                    next = target
                }
                REPEAT_BUMP if arg == repeat => bumped = true,
                CAPTURE if !bumped => captured = true,
                _ if guard(op) && !captured => {}
                _ => return None,
            }
            if edges.insert((pc, next)) {
                *incoming.entry(next).or_default() += 1;
            }
            pc = next;
        }
        if paths.len() + pending.len() > 16 {
            return None;
        }
    }
    // A branch can join only at the bump. Distinct KIND guards make its
    // successful predecessor unique for each input node.
    if incoming
        .iter()
        .any(|(&pc, &n)| n > 1 && pc != header && split(program.words[pc]).0 != REPEAT_BUMP)
    {
        return None;
    }
    let mut kinds = BTreeSet::new();
    for path in &paths {
        let found = path
            .iter()
            .filter_map(|&pc| {
                let (op, _, arg) = split(program.words[pc]);
                (op == KIND).then_some(arg)
            })
            .collect::<BTreeSet<_>>();
        if found.len() != 1 || !kinds.insert(*found.first()?) {
            return None;
        }
    }
    (!paths.is_empty()).then_some(Body { first, end, paths })
}
fn prove(program: &Program) -> Result<Proof, Error> {
    program.validate()?;
    let unsupported = || Error::UnsupportedBackend;
    if !program.tree_sitter_query {
        return Err(unsupported());
    }
    let begins = program
        .words
        .iter()
        .enumerate()
        .filter(|(_, w)| split(**w).0 == REPEAT_BEGIN)
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    let [begin] = begins.as_slice() else {
        return Err(unsupported());
    };
    let begin = *begin;
    let repeat = split(program.words[begin]).2;
    if begin > MAX_FINITE_JOB_INSTRUCTIONS {
        return Err(unsupported());
    }
    let mut saved = BTreeSet::new();
    for &w in &program.words[..begin] {
        let (op, _, arg) = split(w);
        match op {
            SAVE_CURSOR => {
                saved.retain(|&slot| slot <= arg);
                saved.insert(arg);
            }
            RESTORE_CURSOR if saved.contains(&arg) => {}
            CAPTURE | CHILD | NEXT | FOLLOWING => {}
            _ if guard(op) => {}
            _ => return Err(unsupported()),
        }
    }
    let (FOLLOWING, mode, _) = split(*program.words.get(begin + 1).ok_or_else(unsupported)?) else {
        return Err(unsupported());
    };
    let headers = program
        .words
        .iter()
        .enumerate()
        .filter_map(|(pc, &w)| {
            let (op, m, exit) = split(w);
            (op == FORK
                && m == 0
                && program
                    .words
                    .get(pc + 1)
                    .is_some_and(|&next| split(next).0 == NEXT)
                && program.words.get(usize::from(exit)).copied()
                    == Some(word(REPEAT_END, 0, repeat)))
            .then_some((pc, usize::from(exit)))
        })
        .collect::<Vec<_>>();
    let [(header, exit)] = headers.as_slice() else {
        return Err(unsupported());
    };
    let (header, exit) = (*header, *exit);
    if begin + 2 >= header || header + 2 >= exit {
        return Err(unsupported());
    }
    let initial = body(program, begin + 2, header, header, repeat).ok_or_else(unsupported)?;
    let repeating = body(program, header + 2, exit, header, repeat).ok_or_else(unsupported)?;
    let normalize = |b: &Body| {
        b.paths
            .iter()
            .map(|p| {
                p.iter()
                    .filter_map(|&pc| {
                        let w = program.words[pc];
                        let op = split(w).0;
                        (guard(op) || op == CAPTURE).then_some(w)
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    };
    if normalize(&initial) != normalize(&repeating) {
        return Err(unsupported());
    }
    let follow = exit + 1;
    let tail_kind = exit + 2;
    let (FOLLOWING, tail_mode, _) = split(*program.words.get(follow).ok_or_else(unsupported)?)
    else {
        return Err(unsupported());
    };
    if program
        .words
        .get(tail_kind)
        .is_none_or(|&w| split(w).0 != KIND)
    {
        return Err(unsupported());
    }
    let mut pc = tail_kind + 1;
    let mut post_kind = Vec::new();
    loop {
        if post_kind.len() >= MAX_FINITE_JOB_INSTRUCTIONS {
            return Err(unsupported());
        }
        let (op, _, arg) = split(*program.words.get(pc).ok_or_else(unsupported)?);
        post_kind.push(pc);
        match op {
            GOTO if usize::from(arg) > pc => {
                pc = usize::from(arg);
                continue;
            }
            RESTORE_CURSOR if saved.contains(&arg) => {}
            ASSERT => {}
            EMIT if pc + 1 == program.words.len() => break,
            _ => return Err(unsupported()),
        }
        pc += 1;
    }
    Ok(Proof {
        begin,
        header,
        tail_kind,
        emit: pc,
        initial_named: mode == NAMED,
        repeating_named: split(program.words[header + 1]).1 == NAMED,
        tail_named: tail_mode == NAMED,
        post_kind,
        initial,
        repeating,
    })
}

fn matches_guard(program: &Program, tape: &TapeDoc, pc: usize, node: u32) -> bool {
    let (op, mode, arg) = split(program.words[pc]);
    match op {
        KIND => tape.transducer_kind(node) == arg,
        FIELD => tape.transducer_field(node) == arg,
        FLAGS => {
            tape.transducer_flags(node) & mode == mode
                && tape.transducer_flags(node) & arg as u8 == 0
        }
        SIZE => {
            let actual = tape.transducer_span(node).len() as u64;
            let expected = if mode & SIZE_LONG == 0 {
                u64::from(arg)
            } else {
                program.longs[usize::from(arg)]
            };
            match mode & !SIZE_LONG {
                0 => actual == expected,
                1 => actual >= expected,
                2 => actual <= expected,
                _ => unreachable!(),
            }
        }
        FIRST | LAST => {
            let mut children = tape.transducer_parent(node).map(|p| {
                tape.transducer_children(p)
                    .filter(|&n| mode != NAMED || tape.transducer_flags(n) & NAMED != 0)
            });
            let edge = children.as_mut().map_or(Some(node), |items| {
                if op == FIRST {
                    items.next()
                } else {
                    items.last()
                }
            });
            edge == Some(node)
        }
        ABSENT_FIELD => !tape
            .transducer_children(node)
            .any(|n| tape.transducer_field(n) == arg),
        SUPERTYPE => tape.transducer_has_supertype(node, arg),
        _ => unreachable!(),
    }
}
struct Work<'a> {
    limits: Limits,
    report: &'a mut Report,
}
impl Work<'_> {
    fn charge(&mut self, units: usize) -> Result<(), Error> {
        self.report.task_work = self
            .report
            .task_work
            .checked_add(units)
            .ok_or(Error::WorkLimit)?;
        if self.report.task_work > self.limits.task {
            return Err(Error::WorkLimit);
        }
        Ok(())
    }
    fn path(&self, row: &mut Record) -> Result<(), Error> {
        row.steps = row.steps.checked_add(1).ok_or(Error::WorkLimit)?;
        if row.steps as usize > self.limits.path {
            return Err(Error::WorkLimit);
        }
        Ok(())
    }
    fn step(&mut self, row: &mut Record) -> Result<(), Error> {
        self.charge(2)?;
        self.path(row)
    }
}
struct BodyInput<'a> {
    body: &'a Body,
    header: usize,
    node: u32,
    event_start: u32,
    events: &'a [Capture],
    records: Vec<Record>,
}
fn run_body(
    program: &Program,
    tape: &TapeDoc,
    input: BodyInput<'_>,
    work: &mut Work<'_>,
) -> Result<Vec<Record>, Error> {
    let BodyInput {
        body,
        header,
        node,
        event_start,
        events,
        records,
    } = input;
    let mut stages = (body.first..body.end)
        .map(|_| Vec::new())
        .collect::<Vec<Vec<Record>>>();
    if records.is_empty() {
        return Ok(Vec::new());
    }
    stages[0] = records;
    let mut output = Vec::new();
    for pc in body.first..body.end {
        let (op, _, arg) = split(program.words[pc]);
        for mut row in std::mem::take(&mut stages[pc - body.first]) {
            work.step(&mut row)?;
            let mut target = pc + 1;
            match op {
                FORK => stages[usize::from(arg) - body.first].push(row),
                GOTO => target = usize::from(arg),
                CAPTURE => {
                    if row.captures.first == row.captures.last {
                        row.captures = CaptureRange {
                            first: event_start,
                            last: event_start,
                        };
                    }
                    let value = events
                        .get(row.captures.last as usize)
                        .ok_or(Error::BackendFailure)?;
                    if value.label != arg || value.node != node {
                        return Err(Error::BackendFailure);
                    }
                    row.captures.last += 1;
                }
                REPEAT_BUMP => {}
                _ if guard(op) => {
                    if !matches_guard(program, tape, pc, node) {
                        continue;
                    }
                }
                _ => return Err(Error::BackendFailure),
            }
            if target == header {
                output.push(row);
            } else {
                stages[target - body.first].push(row);
            }
        }
    }
    Ok(output)
}
fn capture(tape: &TapeDoc, label: u16, node: u32) -> Capture {
    Capture {
        label,
        node,
        span: tape.transducer_span(node),
        text: None,
        properties: Vec::new(),
    }
}
fn capture_geometry(prefix: &[Capture], events: &[Capture]) -> Result<(), Error> {
    let geometry = |c: &Capture| (c.span.start, std::cmp::Reverse(c.span.end));
    if events
        .windows(2)
        .any(|p| p[0].node != p[1].node && geometry(&p[0]) >= geometry(&p[1]))
        || events
            .first()
            .is_some_and(|e| prefix.iter().any(|p| geometry(p) >= geometry(e)))
    {
        return Err(Error::UnsupportedBackend);
    }
    Ok(())
}
fn execute_region(
    program: &Program,
    tape: &TapeDoc,
    root: u32,
    frame: entry::Frame,
    proof: &Proof,
    work: &mut Work<'_>,
) -> Result<Region, Error> {
    let mut base = Record {
        captures: CaptureRange::default(),
        entry: 0,
        stop: 0,
        steps: frame.steps,
    };
    let prefix = frame.captures;
    work.step(&mut base)?; // BEGIN
    work.step(&mut base)?; // initial FOLLOWING
    let nodes = std::iter::successors(tape.transducer_next_sibling(frame.node, false), |&node| {
        tape.transducer_next_sibling(node, false)
    })
    .collect::<Vec<_>>();
    u32::try_from(nodes.len()).map_err(|_| Error::BackendCapacity)?;
    let mut events = Vec::new();
    let mut offsets = vec![0];
    for &node in &nodes {
        if proof.initial_named && proof.repeating_named && tape.transducer_flags(node) & NAMED == 0
        {
            offsets.push(u32::try_from(events.len()).map_err(|_| Error::BackendCapacity)?);
            continue;
        }
        let before = events.len();
        for path in &proof.initial.paths {
            if path
                .iter()
                .filter(|&&pc| guard(split(program.words[pc]).0))
                .all(|&pc| matches_guard(program, tape, pc, node))
            {
                events.extend(path.iter().filter_map(|&pc| {
                    let (op, _, label) = split(program.words[pc]);
                    (op == CAPTURE).then(|| capture(tape, label, node))
                }));
                break;
            }
        }
        if proof.repeating_named
            && tape.transducer_flags(node) & NAMED == 0
            && events.len() != before
        {
            return Err(Error::UnsupportedBackend);
        }
        offsets.push(u32::try_from(events.len()).map_err(|_| Error::BackendCapacity)?);
    }
    // The interval reducer requires strict geometry between distinct nodes.
    // Valid tapes outside this class retain their ordinary execution class.
    capture_geometry(&prefix, &events)?;
    work.report.capture_events += events.len();
    let mut next_nodes = vec![None; nodes.len()];
    let mut next = None;
    for index in (0..nodes.len()).rev() {
        next_nodes[index] = next;
        if !proof.repeating_named || tape.transducer_flags(nodes[index]) & NAMED != 0 {
            next = Some(index);
        }
    }
    let mut incoming = Vec::new();
    let mut incoming_at = None;
    let mut exits = Vec::new();
    for (index, &node) in nodes.iter().enumerate() {
        let initial = Record {
            entry: index as u32,
            stop: index as u32,
            ..base
        };
        let mut headers = run_body(
            program,
            tape,
            BodyInput {
                body: &proof.initial,
                header: proof.header,
                node,
                event_start: offsets[index],
                events: &events,
                records: if !proof.initial_named || tape.transducer_flags(node) & NAMED != 0 {
                    vec![initial]
                } else {
                    Vec::new()
                },
            },
            work,
        )?;
        headers.extend(run_body(
            program,
            tape,
            BodyInput {
                body: &proof.repeating,
                header: proof.header,
                node,
                event_start: offsets[index],
                events: &events,
                records: if incoming_at == Some(index) {
                    incoming_at = None;
                    std::mem::take(&mut incoming)
                } else {
                    Vec::new()
                },
            },
            work,
        )?);
        for row in &mut headers {
            row.stop = index as u32;
            if row.captures.first == row.captures.last {
                row.captures = CaptureRange::default();
            }
        }
        headers.sort_by_key(|row| (row.captures, row.entry));
        let mut previous = None;
        for mut row in headers {
            work.charge(2)?; // every attempted header admission, including duplicates
            if previous == Some(row.captures) {
                work.report.duplicate_headers += 1;
                continue;
            }
            previous = Some(row.captures);
            work.path(&mut row)?; // accepted FORK
            work.report.admitted_headers += 1;
            exits.push(row);
            work.step(&mut row)?; // NEXT, including an unsuccessful final attempt
            if let Some(target) = next_nodes[index] {
                if incoming_at.is_some_and(|old| old != target) {
                    return Err(Error::BackendFailure);
                }
                incoming_at = Some(target);
                incoming.push(row);
            }
        }
    }
    work.report.exit_records += exits.len();
    exits.sort_by_key(|row| (row.captures, row.entry, std::cmp::Reverse(row.stop)));
    let tail_kind = split(program.words[proof.tail_kind]).2;
    let tail_nodes = (0..nodes.len())
        .filter(|&i| !proof.tail_named || tape.transducer_flags(nodes[i]) & NAMED != 0)
        .collect::<Vec<_>>();
    let mut terminals = Vec::new();
    let mut offset = 0;
    while offset < exits.len() {
        let end =
            offset + exits[offset..].partition_point(|r| r.captures == exits[offset].captures);
        let mut covered = tail_nodes.len();
        let mut passed = Vec::new();
        for &exit in &exits[offset..end] {
            let mut row = exit;
            work.step(&mut row)?; // END
            work.step(&mut row)?; // FOLLOWING
            let first = tail_nodes.partition_point(|&i| i <= row.stop as usize);
            if first >= tail_nodes.len() {
                continue;
            }
            if first >= covered {
                work.charge(2 + usize::from(first + 1 < tail_nodes.len()))?;
                continue;
            }
            for &target in &tail_nodes[first..covered] {
                let mut candidate = row;
                work.step(&mut candidate)?; // first path or admitted lazy FOLLOWING
                if tape.transducer_kind(nodes[target]) == tail_kind {
                    passed.push((candidate, target));
                }
            }
            if covered < tail_nodes.len() {
                work.charge(1)?;
            } // a lazy duplicate ends its chain
            covered = first;
        }
        let mut shared_suffix = false;
        for (mut row, target) in passed {
            let mut after_restore = false;
            let mut joined = false;
            for &pc in &proof.post_kind {
                if after_restore && !joined {
                    work.charge(2)?;
                    if shared_suffix {
                        break;
                    }
                    work.path(&mut row)?;
                    shared_suffix = true;
                    joined = true;
                    after_restore = false;
                } else {
                    work.step(&mut row)?;
                }
                match split(program.words[pc]).0 {
                    RESTORE_CURSOR => after_restore = !joined,
                    EMIT => terminals.push((row, target)),
                    GOTO | ASSERT => {}
                    _ => return Err(Error::BackendFailure),
                }
            }
        }
        offset = end;
    }
    work.report.terminal_candidates += terminals.len();
    let candidate_count = terminals.len();
    terminals.sort_by_key(|(r, target)| (r.entry, std::cmp::Reverse(r.stop), *target));
    let mut ranked = terminals
        .into_iter()
        .enumerate()
        .map(|(ordinal, (row, _))| (ordinal, row.captures))
        .collect::<Vec<_>>();
    ranked.sort_by_key(|&(ordinal, slice)| (slice.first, std::cmp::Reverse(slice.last), ordinal));
    let any_nonempty = ranked.iter().any(|(_, s)| s.first != s.last);
    let mut maximum = 0;
    let mut selected = Vec::new();
    for (ordinal, slice) in ranked {
        if slice.first == slice.last {
            if !any_nonempty && selected.is_empty() {
                selected.push((ordinal, slice));
            }
        } else if slice.last > maximum {
            maximum = slice.last;
            selected.push((ordinal, slice));
        }
    }
    selected.sort_by_key(|&(ordinal, _)| ordinal);
    let mut output = Vec::new();
    for (ordinal, slice) in selected {
        let mut captures = prefix.clone();
        captures.extend_from_slice(&events[slice.first as usize..slice.last as usize]);
        work.report.materialized_capture_records += captures.len();
        captures.sort_by_key(|capture| (capture.span.start, std::cmp::Reverse(capture.span.end)));
        output.push(Output {
            task_ordinal: 0,
            local_ordinal: u32::try_from(ordinal).map_err(|_| Error::BackendCapacity)?,
            start: root,
            label: split(program.words[proof.emit]).2,
            captures,
            properties: program.properties(),
            dynamic_properties: Vec::new(),
        });
    }
    Ok(Region {
        outputs: output,
        candidate_count,
    })
}

struct Region {
    outputs: Vec<Output>,
    candidate_count: usize,
}

pub(super) struct Prepared<'p> {
    program: &'p Program,
    proof: Proof,
}
pub(super) struct Run {
    pub outputs: Vec<Output>,
    pub report: Report,
}
pub(super) fn prepare(program: &Program) -> Result<Prepared<'_>, Error> {
    Ok(Prepared {
        program,
        proof: prove(program)?,
    })
}
impl Prepared<'_> {
    pub(super) fn execute_streaming(
        &self,
        tape: &TapeDoc,
        root: u32,
        limits: Limits,
    ) -> Result<Run, Error> {
        self.execute_mode(tape, root, limits, true)
    }
    pub(super) fn execute(&self, tape: &TapeDoc, root: u32, limits: Limits) -> Result<Run, Error> {
        self.execute_mode(tape, root, limits, false)
    }
    fn execute_mode(
        &self,
        tape: &TapeDoc,
        root: u32,
        limits: Limits,
        streaming: bool,
    ) -> Result<Run, Error> {
        if limits.path > MAX_STEPS_PER_PATH
            || limits.task > MAX_WORK_PER_TASK
            || root as usize >= tape.transducer_len()
        {
            return Err(Error::InvalidAnchor);
        }
        let mut report = Report::default();
        let mut work = Work {
            limits,
            report: &mut report,
        };
        let frames = entry::execute(self.program, tape, root, self.proof.begin, &mut work)?;
        let mut outputs = Vec::new();
        let mut ordinal = 0usize;
        for frame in frames {
            let mut region = if streaming {
                stream::execute_region(self.program, tape, root, frame, &self.proof, &mut work)?
            } else {
                execute_region(self.program, tape, root, frame, &self.proof, &mut work)?
            };
            for output in &mut region.outputs {
                output.local_ordinal = u32::try_from(
                    ordinal
                        .checked_add(output.local_ordinal as usize)
                        .ok_or(Error::BackendCapacity)?,
                )
                .map_err(|_| Error::BackendCapacity)?;
            }
            ordinal = ordinal
                .checked_add(region.candidate_count)
                .ok_or(Error::BackendCapacity)?;
            outputs.extend(region.outputs);
        }
        let outputs = self.program.finalize_outputs(tape, outputs, true)?;
        Ok(Run { outputs, report })
    }
}
