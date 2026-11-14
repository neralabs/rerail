//! A complete run with one final tail needs scalar counts, not exit records.
//! Logical admission and work remain identical to the range operator.
use super::*;

struct Cost {
    visits: usize,
    maximum: usize,
    success: Option<usize>,
    labels: Vec<u16>,
}
fn cost(
    program: &Program,
    tape: &TapeDoc,
    body: &Body,
    header: usize,
    node: u32,
) -> Result<Cost, Error> {
    let mut stages = vec![Vec::new(); body.end - body.first];
    stages[0].push(0usize);
    let mut result = Cost {
        visits: 0,
        maximum: 0,
        success: None,
        labels: vec![],
    };
    for pc in body.first..body.end {
        let (op, _, arg) = split(program.words[pc]);
        for before in std::mem::take(&mut stages[pc - body.first]) {
            let steps = before + 1;
            result.visits += 1;
            result.maximum = result.maximum.max(steps);
            let mut target = pc + 1;
            match op {
                FORK => stages[usize::from(arg) - body.first].push(steps),
                GOTO => target = usize::from(arg),
                CAPTURE => result.labels.push(arg),
                REPEAT_BUMP => {}
                _ if guard(op) => {
                    if !matches_guard(program, tape, pc, node) {
                        continue;
                    }
                }
                _ => return Err(Error::BackendFailure),
            }
            if target == header {
                if result.success.replace(steps).is_some() {
                    return Err(Error::BackendFailure);
                }
            } else {
                stages[target - body.first].push(steps);
            }
        }
    }
    Ok(result)
}
fn path(steps: usize, work: &Work<'_>) -> Result<(), Error> {
    if steps > work.limits.path {
        Err(Error::WorkLimit)
    } else {
        Ok(())
    }
}
fn product(a: usize, b: usize) -> Result<usize, Error> {
    a.checked_mul(b).ok_or(Error::WorkLimit)
}

pub(super) fn execute_region(
    program: &Program,
    tape: &TapeDoc,
    root: u32,
    frame: entry::Frame,
    proof: &Proof,
    work: &mut Work<'_>,
) -> Result<Region, Error> {
    let all_nodes =
        std::iter::successors(tape.transducer_next_sibling(frame.node, false), |&node| {
            tape.transducer_next_sibling(node, false)
        })
        .collect::<Vec<_>>();
    if (proof.initial_named != proof.repeating_named || proof.tail_named != proof.repeating_named)
        && all_nodes
            .iter()
            .any(|&node| tape.transducer_flags(node) & NAMED == 0)
    {
        return Err(Error::UnsupportedBackend);
    }
    let nodes = all_nodes
        .into_iter()
        .filter(|&node| !proof.repeating_named || tape.transducer_flags(node) & NAMED != 0)
        .collect::<Vec<_>>();
    let base = frame.steps as usize + 2;
    work.charge(4)?; // BEGIN and initial FOLLOWING
    path(base, work)?;
    let Some(&tail) = nodes.last() else {
        return Ok(Region {
            outputs: vec![],
            candidate_count: 0,
        });
    };
    if tape.transducer_kind(tail) != split(program.words[proof.tail_kind]).2 {
        return Err(Error::UnsupportedBackend);
    }
    let mut classes = Vec::new();
    let mut events = Vec::new();
    let mut positive = Vec::new();
    for (index, &node) in nodes.iter().enumerate() {
        let initial = cost(program, tape, &proof.initial, proof.header, node)?;
        let repeating = cost(program, tape, &proof.repeating, proof.header, node)?;
        let last = index + 1 == nodes.len();
        if last == initial.success.is_some()
            || last == repeating.success.is_some()
            || initial.labels != repeating.labels
        {
            return Err(Error::UnsupportedBackend);
        }
        if !last {
            if !initial.labels.is_empty() {
                positive.push(index);
            }
            events.extend(
                initial
                    .labels
                    .iter()
                    .map(|&label| capture(tape, label, node)),
            );
        }
        classes.push((initial, repeating));
    }
    u32::try_from(events.len()).map_err(|_| Error::BackendCapacity)?;
    capture_geometry(&frame.captures, &events)?;
    let body_count = nodes.len() - 1;
    let mut previous_count = 0usize;
    let mut previous_max = None;
    let mut nonempty_max = None;
    let mut empty_max = None;
    let mut first_empty_winner = None;
    let mut first_empty_closed = false;
    let mut captured_nodes = 0usize;
    let mut exits = 0usize;
    for (index, (initial, repeating)) in classes.iter().enumerate() {
        work.charge(product(
            2,
            initial.visits + product(previous_count, repeating.visits)?,
        )?)?;
        path(base + initial.maximum, work)?;
        if let Some(previous) = previous_max {
            path(previous + 1 + repeating.maximum, work)?;
        }
        if index == body_count {
            break;
        }
        let initial_steps = initial.success.ok_or(Error::BackendFailure)?;
        let repeat_steps = repeating.success.ok_or(Error::BackendFailure)?;
        let captured = !initial.labels.is_empty();
        captured_nodes += usize::from(captured);
        let count = captured_nodes + usize::from(!captured);
        let attempts = previous_count + 1;
        let duplicates = attempts.checked_sub(count).ok_or(Error::BackendFailure)?;
        if duplicates > 1 {
            return Err(Error::BackendFailure);
        }
        work.charge(product(2, attempts + count)?)?; // header attempts and admitted NEXTs
        work.report.duplicate_headers += duplicates;
        work.report.admitted_headers += count;
        exits += count;
        let mut header_max = previous_max.map(|previous| previous + 2 + repeat_steps);
        if duplicates == 0 {
            header_max = Some(header_max.unwrap_or(0).max(base + initial_steps + 1));
        }
        let header_max = header_max.ok_or(Error::BackendFailure)?;
        path(header_max + 1, work)?; // accepted header and NEXT
        if captured {
            nonempty_max = Some(header_max);
            empty_max = None;
            if first_empty_winner.is_some() {
                first_empty_closed = true;
            }
        } else {
            nonempty_max = nonempty_max.map(|previous| previous + 2 + repeat_steps);
            let empty = empty_max.map_or(base + initial_steps + 1, |previous| {
                previous + 2 + repeat_steps
            });
            empty_max = Some(empty);
            if !first_empty_closed {
                first_empty_winner = Some(empty);
            }
        }
        previous_count = count;
        previous_max = Some(header_max);
    }
    work.report.capture_events += events.len();
    work.report.exit_records += exits;
    work.charge(product(4, exits)?)?; // every END and tail FOLLOWING
    if let Some(previous) = previous_max {
        path(previous + 2, work)?;
    }
    // Each nonempty interval ending at a captured node has the same stop range.
    for (index, &at) in positive.iter().enumerate() {
        let next = positive.get(index + 1).copied().unwrap_or(body_count);
        let duplicates = next - at;
        work.charge(product(index + 1, 2 * (body_count - at) + duplicates - 1)?)?;
    }
    // The empty interval is shared across zero-capture runs. Later runs can
    // stop at the first duplicate lazy target, before any path work.
    let mut covered = nodes.len();
    let mut at = 0;
    let mut any_empty = false;
    while at < body_count {
        if !classes[at].0.labels.is_empty() {
            at += 1;
            continue;
        }
        any_empty = true;
        let first = at;
        while at < body_count && classes[at].0.labels.is_empty() {
            at += 1;
        }
        for stop in (first..at).rev() {
            let target = stop + 1;
            if target >= covered {
                work.charge(2 + usize::from(target + 1 < nodes.len()))?;
            } else {
                work.charge(2 * (covered - target) + usize::from(covered < nodes.len()))?;
                covered = target;
            }
        }
    }
    let raw = product(captured_nodes, captured_nodes + 1)? / 2 + usize::from(any_empty);
    work.charge(product(product(2, raw)?, proof.post_kind.len())?)?;
    if let Some(winner) = nonempty_max.into_iter().chain(first_empty_winner).max() {
        path(winner + 3 + proof.post_kind.len(), work)?;
    }
    work.report.terminal_candidates += raw;
    if body_count == 0 {
        return Ok(Region {
            outputs: vec![],
            candidate_count: 0,
        });
    }
    let mut captures = frame.captures;
    captures.extend(events);
    work.report.materialized_capture_records += captures.len();
    captures.sort_by_key(|c| (c.span.start, std::cmp::Reverse(c.span.end)));
    Ok(Region {
        candidate_count: raw,
        outputs: vec![Output {
            task_ordinal: 0,
            local_ordinal: 0,
            start: root,
            label: split(program.words[proof.emit]).2,
            captures,
            properties: program.properties(),
            dynamic_properties: vec![],
        }],
    })
}
