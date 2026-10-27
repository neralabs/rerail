//! Acyclic prefix stages carry complete frames into capture-run regions.
//! Membership is checked for the current stage and retires at its boundary.
use super::*;

#[derive(Clone)]
pub(super) struct Frame {
    pub node: u32,
    pub steps: u32,
    pub captures: Vec<Capture>,
    cursors: BTreeMap<u16, u32>,
}

fn captures(frame: &Frame) -> Vec<(u16, u32)> {
    let mut captures = frame.captures.iter().collect::<Vec<_>>();
    captures.sort_by_key(|c| (c.span.start, std::cmp::Reverse(c.span.end)));
    captures.into_iter().map(|c| (c.label, c.node)).collect()
}

pub(super) fn execute(
    program: &Program,
    tape: &TapeDoc,
    root: u32,
    end: usize,
    work: &mut Work<'_>,
) -> Result<Vec<Frame>, Error> {
    let mut current = vec![Frame {
        node: root,
        steps: 0,
        captures: vec![],
        cursors: BTreeMap::new(),
    }];
    for pc in 0..end {
        let (op, mode, arg) = split(program.words[pc]);
        let mut membership = BTreeSet::new();
        let mut next = Vec::new();
        for mut frame in current {
            let cursors = frame
                .cursors
                .iter()
                .map(|(&slot, &node)| (slot, node))
                .collect::<Vec<_>>();
            if !membership.insert((frame.node, cursors, captures(&frame))) {
                // This class proves independent prefix paths. A reconverging
                // prefix needs the general complete-state admission operator.
                return Err(Error::UnsupportedBackend);
            }
            let mut cost = Record {
                captures: CaptureRange::default(),
                entry: 0,
                stop: 0,
                steps: frame.steps,
            };
            work.step(&mut cost)?;
            frame.steps = cost.steps;
            match op {
                CHILD | NEXT | FOLLOWING => {
                    let named = mode == NAMED;
                    let nodes = if op == CHILD {
                        tape.transducer_children(frame.node)
                            .filter(|&n| !named || tape.transducer_flags(n) & NAMED != 0)
                            .collect::<Vec<_>>()
                    } else {
                        let mut node = tape.transducer_next_sibling(frame.node, named);
                        let mut nodes = Vec::new();
                        while let Some(found) = node {
                            nodes.push(found);
                            if op == NEXT {
                                break;
                            }
                            node = tape.transducer_next_sibling(found, named);
                        }
                        nodes
                    };
                    next.extend(nodes.into_iter().map(|node| Frame {
                        node,
                        ..frame.clone()
                    }));
                    continue;
                }
                SAVE_CURSOR => {
                    frame.cursors.retain(|&slot, _| slot <= arg);
                    frame.cursors.insert(arg, frame.node);
                }
                RESTORE_CURSOR => {
                    frame.node = *frame.cursors.get(&arg).ok_or(Error::BackendFailure)?;
                }
                CAPTURE => frame.captures.push(capture(tape, arg, frame.node)),
                _ if guard(op) => {
                    if !matches_guard(program, tape, pc, frame.node) {
                        continue;
                    }
                }
                _ => return Err(Error::BackendFailure),
            }
            next.push(frame);
        }
        current = next;
    }
    // Distinct saved cursor frames remain distinct throughout each region,
    // including after restoration. No cross-region query key can collide.
    let mut frames = BTreeSet::new();
    for frame in &current {
        if !frames.insert(
            frame
                .cursors
                .iter()
                .map(|(&s, &n)| (s, n))
                .collect::<Vec<_>>(),
        ) {
            return Err(Error::UnsupportedBackend);
        }
    }
    Ok(current)
}
