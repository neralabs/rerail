//! CPU reference for a GPU-shaped closure over checked, complete-value domains.
//! Each domain is one sibling chain and one complete control-state value group.
//! Domain packing belongs to the research adapter, not to these scan operators.
use super::*;

const WIDTH: usize = 256;
const MAX_CELLS: usize = LIMITS.candidates;
const NONE: u32 = u32::MAX;

#[derive(Clone, Copy, Debug)]
struct Domain {
    first: u32,
    count: u32,
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    domain: u32,
    offset: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Stage {
    ValidateDomains,
    ValidateEntries,
    InitializeCells,
    SeedEntries,
    ClassifyRuns,
    ScanBlocks,
    ScanTotals,
    AddCarries,
    ClassifyEnds,
    CountExits,
    ScatterExits,
}

#[derive(Clone, Copy, Debug)]
struct Dispatch {
    stage: Stage,
    lanes: usize,
    // Scan kernels have eight fixed workgroup-local synchronization rounds.
    local_rounds: usize,
}

#[derive(Default, Debug)]
struct Report {
    dispatches: Vec<Dispatch>,
    reserved_bytes: usize,
}
impl Report {
    fn record(&mut self, stage: Stage, lanes: usize, local_rounds: usize) {
        self.dispatches.push(Dispatch {
            stage,
            lanes,
            local_rounds,
        });
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Exit {
    entry: u32,
    // Absolute cell address in the input domain array, not a tape node ID.
    last: u32,
}

#[derive(Clone, Copy)]
struct Cell {
    head: bool,
    value: u32,
}
impl Default for Cell {
    fn default() -> Self {
        Self {
            head: false,
            value: NONE,
        }
    }
}
fn minimum(left: Cell, right: Cell) -> Cell {
    Cell {
        head: left.head || right.head,
        value: if right.head {
            right.value
        } else {
            left.value.min(right.value)
        },
    }
}

// A CPU emulation of one 256-lane workgroup. Every round reads the previous
// round's immutable registers. There is no scalar, data-dependent prefix loop.
fn block_scan<T: Copy + Default>(values: &[T], combine: fn(T, T) -> T) -> [T; WIDTH] {
    let mut cells = [T::default(); WIDTH];
    cells[..values.len()].copy_from_slice(values);
    for distance in [1, 2, 4, 8, 16, 32, 64, 128] {
        let previous = cells;
        for lane in 0..WIDTH {
            if lane >= distance {
                cells[lane] = combine(previous[lane - distance], previous[lane]);
            }
        }
    }
    cells
}

// At most 128 workgroups under MAX_CELLS. Block totals fit one workgroup.
// All three broad stages execute, including for an empty or one-block input.
fn scan<T: Copy + Default>(input: &[T], combine: fn(T, T) -> T, report: &mut Report) -> Vec<T> {
    let mut output = vec![T::default(); input.len()];
    let mut totals = Vec::with_capacity(input.len().div_ceil(WIDTH));
    for (block, chunk) in input.chunks(WIDTH).enumerate() {
        let cells = block_scan(chunk, combine);
        output[block * WIDTH..block * WIDTH + chunk.len()].copy_from_slice(&cells[..chunk.len()]);
        totals.push(cells[WIDTH - 1]);
    }
    report.record(Stage::ScanBlocks, input.len().div_ceil(WIDTH) * WIDTH, 8);
    let carries = block_scan(&totals, combine);
    report.record(Stage::ScanTotals, WIDTH, 8);
    for (index, value) in output.iter_mut().enumerate() {
        let block = index / WIDTH;
        if block != 0 {
            *value = combine(carries[block - 1], *value);
        }
    }
    report.record(Stage::AddCarries, input.len(), 0);
    output
}

fn reservation(cells: usize, entries: usize, domains: usize) -> Result<usize, Error> {
    if cells > MAX_CELLS || entries > MAX_CELLS || domains > MAX_CELLS {
        return Err(Error::BackendCapacity);
    }
    // Conservative simultaneous reservation, including input, output, both scan
    // arrays, fixed workgroup registers, totals, and all temporary descriptors.
    let bytes = cells
        .checked_mul(64)
        .and_then(|n| entries.checked_mul(32).and_then(|e| n.checked_add(e)))
        .and_then(|n| domains.checked_mul(16).and_then(|d| n.checked_add(d)))
        .and_then(|n| n.checked_add(16 * 1024))
        .ok_or(Error::BackendCapacity)?;
    if bytes > LIMITS.payload_bytes {
        return Err(Error::BackendCapacity);
    }
    Ok(bytes)
}

fn resolve(
    domains: &[Domain],
    entries: &[Entry],
    allowed: &[bool],
) -> Result<(Vec<Exit>, Report), Error> {
    let mut report = Report {
        reserved_bytes: reservation(allowed.len(), entries.len(), domains.len())?,
        ..Report::default()
    };
    // Each domain lane checks its own bounds and its immediate predecessor.
    // Errors reject the entire batch before any output can be returned.
    for (index, domain) in domains.iter().enumerate() {
        let expected = if index == 0 {
            Some(0)
        } else {
            domains[index - 1]
                .first
                .checked_add(domains[index - 1].count)
        };
        if domain.count == 0
            || Some(domain.first) != expected
            || domain
                .first
                .checked_add(domain.count)
                .is_none_or(|end| end as usize > allowed.len())
        {
            return Err(Error::InvalidAnchor);
        }
    }
    let end = domains.last().map_or(0, |d| d.first + d.count);
    if end as usize != allowed.len() {
        return Err(Error::InvalidAnchor);
    }
    report.record(Stage::ValidateDomains, domains.len(), 0);
    for entry in entries {
        if domains
            .get(entry.domain as usize)
            .is_none_or(|d| entry.offset >= d.count)
        {
            return Err(Error::InvalidAnchor);
        }
    }
    report.record(Stage::ValidateEntries, entries.len(), 0);

    let mut seeds = vec![NONE; allowed.len()];
    let mut heads = vec![false; allowed.len()];
    report.record(Stage::InitializeCells, allowed.len(), 0);
    for (priority, entry) in entries.iter().enumerate() {
        let at = (domains[entry.domain as usize].first + entry.offset) as usize;
        // Exactly one atomic-min equivalent per input entry. Equal starts keep
        // the earliest logical entry; no output allocation uses an atomic cursor.
        seeds[at] = seeds[at].min(priority as u32);
    }
    for domain in domains {
        heads[domain.first as usize] = true;
    }
    report.record(Stage::SeedEntries, entries.len().max(domains.len()), 0);
    let cells = allowed
        .iter()
        .enumerate()
        .map(|(index, &pass)| Cell {
            head: heads[index] || !pass,
            value: seeds[index],
        })
        .collect::<Vec<_>>();
    report.record(Stage::ClassifyRuns, allowed.len(), 0);
    let owners = scan(&cells, minimum, &mut report);

    // Reverse segmented minimum finds the end of each ownership interval.
    // A node can change owner only at a later entry of higher logical priority
    // or at an iteration guard failure. Domains never share an ownership scan.
    let ends = (0..allowed.len())
        .rev()
        .map(|index| {
            let last = index + 1 == allowed.len() || heads[index + 1];
            Cell {
                head: last,
                value: if last || owners[index].value != owners[index + 1].value {
                    index as u32
                } else {
                    NONE
                },
            }
        })
        .collect::<Vec<_>>();
    report.record(Stage::ClassifyEnds, allowed.len(), 0);
    let ends = scan(&ends, minimum, &mut report);
    let keep = entries
        .iter()
        .enumerate()
        .map(|(priority, entry)| {
            let at = (domains[entry.domain as usize].first + entry.offset) as usize;
            u32::from(owners[at].value == priority as u32)
        })
        .collect::<Vec<_>>();
    report.record(Stage::CountExits, entries.len(), 0);
    let offsets = scan(&keep, |a, b| a + b, &mut report);
    let count = offsets.last().copied().unwrap_or(0) as usize;
    let mut output = vec![
        Exit {
            entry: NONE,
            last: NONE
        };
        entries.len()
    ];
    for (priority, entry) in entries.iter().enumerate() {
        if keep[priority] != 0 {
            let at = (domains[entry.domain as usize].first + entry.offset) as usize;
            output[offsets[priority] as usize - 1] = Exit {
                entry: priority as u32,
                last: ends[allowed.len() - 1 - at].value,
            };
        }
    }
    report.record(Stage::ScatterExits, entries.len(), 0);
    // The GPU-shaped scatter uses a preallocated capacity and a separate count.
    // Only this CPU publication boundary exposes the compact initialized prefix.
    output.truncate(count);
    Ok((output, report))
}

use closure_proof::Region as Proof;

fn prove(program: &Program, header: usize) -> Result<Option<Proof<'_>>, Error> {
    Ok(closure_proof::regions(program)?
        .into_iter()
        .find(|region| region.header == header))
}

fn permits(proof: &Proof<'_>, program: &Program, tape: &TapeDoc, node: u32) -> bool {
    proof.guards.iter().all(|&instruction| {
        let (op, mode, arg) = split(instruction);
        match op {
            KIND => tape.transducer_kind(node) == arg,
            FIELD => tape.transducer_field(node) == arg,
            FLAGS => {
                tape.transducer_flags(node) & mode == mode
                    && tape.transducer_flags(node) & arg as u8 == 0
            }
            SIZE => {
                let size = tape.transducer_span(node).len() as u64;
                let expected = if mode & SIZE_LONG == 0 {
                    u64::from(arg)
                } else {
                    program.longs[usize::from(arg)]
                };
                match mode & !SIZE_LONG {
                    0 => size == expected,
                    1 => size >= expected,
                    2 => size <= expected,
                    _ => unreachable!(),
                }
            }
            _ => unreachable!(),
        }
    })
}

// CPU research adapter: group complete values and pack resident sibling ranges.
// This adapter is NOT a GPU grouping implementation or included in stage counts.
// It borrows frame values; it never substitutes hashes or storage IDs for equality.
#[derive(Eq, PartialEq, Ord, PartialOrd)]
struct Key<'a> {
    header: usize,
    parent: Option<u32>,
    cursors: &'a [Option<u32>],
    repeats: &'a [u16],
    captures: &'a [(u16, u32, usize, usize)],
}

pub(super) fn close(
    program: &Program,
    tape: &TapeDoc,
    frontier: &[Item],
    header: usize,
) -> Result<Option<(Vec<Item>, usize, usize)>, Error> {
    let Some(proof) = prove(program, header)? else {
        return Ok(None);
    };
    if frontier
        .windows(2)
        .any(|pair| pair[0].position >= pair[1].position)
    {
        return Err(Error::InvalidAnchor);
    }
    let live_bytes = frontier.iter().try_fold(0usize, |bytes, item| {
        bytes
            .checked_add(item_payload(item))
            .ok_or(Error::BackendCapacity)
    })?;
    let checked_reservation = |cells, domains| -> Result<usize, Error> {
        let bound = reservation(cells, frontier.len(), domains)?
            .checked_add(live_bytes.checked_mul(4).ok_or(Error::BackendCapacity)?)
            .and_then(|bytes| {
                frontier
                    .len()
                    .checked_mul(512)
                    .and_then(|planning| bytes.checked_add(planning))
            })
            .ok_or(Error::BackendCapacity)?;
        if bound > LIMITS.payload_bytes {
            return Err(Error::BackendCapacity);
        }
        Ok(bound)
    };
    checked_reservation(0, 0)?;
    let mut groups = BTreeMap::new();
    let mut domains = Vec::new();
    let mut nodes = Vec::new();
    let mut entries = Vec::new();
    let mut source = Vec::new();
    for (index, item) in frontier.iter().enumerate() {
        if !matches!(item.kind, Kind::Active) || item.state.pc != header {
            continue;
        }
        if item.state.node as usize >= tape.transducer_len() {
            return Err(Error::InvalidAnchor);
        }
        if item.state.repeats.last() != Some(&proof.repeat) {
            return Err(Error::InvalidOperand(header));
        }
        let key = Key {
            header,
            parent: tape.transducer_parent(item.state.node),
            cursors: &item.state.cursors,
            repeats: &item.state.repeats,
            captures: &item.state.captures,
        };
        let domain = if let Some(&id) = groups.get(&key) {
            id
        } else {
            let first = nodes.len();
            let siblings: Box<dyn Iterator<Item = u32> + '_> = match key.parent {
                Some(parent) => Box::new(tape.transducer_children(parent)),
                None => Box::new(std::iter::once(item.state.node)),
            };
            for sibling in siblings {
                checked_reservation(nodes.len() + 1, domains.len() + 1)?;
                nodes.push(sibling);
            }
            let id = domains.len();
            domains.push(Domain {
                first: first as u32,
                count: (nodes.len() - first) as u32,
            });
            groups.insert(key, id);
            id
        };
        let d = domains[domain];
        let offset = nodes[d.first as usize..(d.first + d.count) as usize]
            .binary_search(&item.state.node)
            .map_err(|_| Error::InvalidAnchor)?;
        entries.push(Entry {
            domain: domain as u32,
            offset: offset as u32,
        });
        source.push(index);
    }
    if entries.is_empty() {
        return Err(Error::BackendFailure);
    }
    // A fresh eligible header cannot already have a pending offer. Closing a
    // partially expanded scope would need information beyond this operator input.
    if frontier
        .iter()
        .any(|i| matches!(i.kind, Kind::Greedy { .. }) && i.state.pc == proof.exit)
    {
        return Err(Error::BackendFailure);
    }
    let allowed = nodes
        .iter()
        .map(|&node| permits(&proof, program, tape, node))
        .collect::<Vec<_>>();
    let (exits, report) = resolve(&domains, &entries, &allowed)?;
    let mut replacements = vec![None; frontier.len()];
    for exit in exits {
        let index = source[exit.entry as usize];
        let mut item = frontier[index].clone();
        item.state.node = nodes[exit.last as usize];
        item.state.pc = proof.exit;
        item.position.push(1);
        replacements[index] = Some(item);
    }
    let mut output = Vec::new();
    let mut bytes = 0;
    for (index, item) in frontier.iter().enumerate() {
        if matches!(item.kind, Kind::Active) && item.state.pc == proof.header {
            if let Some(replacement) = replacements[index].take() {
                retain(&mut output, &mut bytes, replacement)?;
            }
        } else {
            retain(&mut output, &mut bytes, item.clone())?;
        }
    }
    // One additional broad guard-classification stage. Input packing remains
    // separate and explicit; these counts never claim to be Metal dispatches.
    Ok(Some((
        output,
        report.dispatches.len() + 1,
        checked_reservation(nodes.len(), domains.len())?,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Deliberately serial first-visit oracle. No prefix scan or interval formula
    // is shared with the operator. It records each visited state exactly once.
    fn oracle(domains: &[Domain], entries: &[Entry], allowed: &[bool]) -> Vec<Exit> {
        let mut visited = vec![false; allowed.len()];
        let mut exits = Vec::new();
        for (priority, entry) in entries.iter().enumerate() {
            let domain = domains[entry.domain as usize];
            let mut at = (domain.first + entry.offset) as usize;
            if visited[at] {
                continue;
            }
            loop {
                visited[at] = true;
                let next = at + 1;
                if next == (domain.first + domain.count) as usize || !allowed[next] || visited[next]
                {
                    break;
                }
                at = next;
            }
            exits.push(Exit {
                entry: priority as u32,
                last: at as u32,
            });
        }
        exits
    }

    #[test]
    fn bulk_closure_matches_every_short_ordered_entry_list_and_guard_partition() {
        let mut checked = 0;
        for len in 1..=5u32 {
            let domains = [Domain {
                first: 0,
                count: len,
            }];
            for mask in 0..(1 << len) {
                let allowed = (0..len).map(|i| mask & (1 << i) != 0).collect::<Vec<_>>();
                // Repetition permits duplicate starts; order is semantically significant.
                for count in 0..=3u32 {
                    for mut encoded in 0..len.pow(count) {
                        let entries = (0..count)
                            .map(|_| {
                                let offset = encoded % len;
                                encoded /= len;
                                Entry { domain: 0, offset }
                            })
                            .collect::<Vec<_>>();
                        let expected = oracle(&domains, &entries, &allowed);
                        let (actual, _) = resolve(&domains, &entries, &allowed).unwrap();
                        assert_eq!(
                            actual, expected,
                            "len={len} mask={mask} entries={entries:?}"
                        );
                        checked += 1;
                    }
                }
            }
        }
        assert_eq!(checked, 6740);
    }

    #[test]
    fn bulk_closure_scales_independent_domains_without_serial_entry_rounds() {
        let mut expected_stages = None;
        for segments in [1, 128, 4096] {
            let domains = (0..segments)
                .map(|i| Domain {
                    first: i * 8,
                    count: 8,
                })
                .collect::<Vec<_>>();
            let mut entries = Vec::new();
            for group in (0..segments).rev() {
                // Earlier entry owns 5..7; later entry must stop at 4. Duplicate
                // entry 5 is suppressed. The third surviving entry owns 0..1.
                entries.extend([5, 2, 5, 0].map(|offset| Entry {
                    domain: group,
                    offset,
                }));
            }
            let allowed = vec![true; segments as usize * 8];
            let expected = oracle(&domains, &entries, &allowed);
            let start = std::time::Instant::now();
            let (actual, report) = resolve(&domains, &entries, &allowed).unwrap();
            let elapsed = start.elapsed();
            assert_eq!(actual, expected);
            assert_eq!(actual.len(), segments as usize * 3);
            assert_eq!(
                report.dispatches.len(),
                *expected_stages.get_or_insert(report.dispatches.len())
            );
            assert_eq!(
                report
                    .dispatches
                    .iter()
                    .filter(|d| d.stage == Stage::ScanBlocks)
                    .count(),
                3
            );
            assert!(
                report
                    .dispatches
                    .iter()
                    .all(|d| d.lanes <= MAX_CELLS && matches!(d.local_rounds, 0 | 8))
            );
            assert!(report.reserved_bytes <= LIMITS.payload_bytes);
            eprintln!(
                "BULK_SEGMENTS {}",
                serde_json::json!({"segments":segments,"exits":actual.len(),"stages":report.dispatches.len(),"reservedBytes":report.reserved_bytes,"cpuSeconds":elapsed.as_secs_f64()})
            );
        }
    }

    #[test]
    fn bulk_closure_crosses_workgroup_boundaries_and_preserves_all_run_breaks() {
        for count in [255, 256, 257, 511, 1024, 32768] {
            let domains = [Domain { first: 0, count }];
            let entries = (0..count)
                .rev()
                .step_by(29)
                .map(|offset| Entry { domain: 0, offset })
                .collect::<Vec<_>>();
            let allowed = (0..count)
                .map(|i| i % 127 != 0 && i % 257 != 0)
                .collect::<Vec<_>>();
            assert_eq!(
                resolve(&domains, &entries, &allowed).unwrap().0,
                oracle(&domains, &entries, &allowed)
            );
        }
    }

    #[test]
    fn bulk_closure_rejects_inconsistent_domains_and_capacity_before_output() {
        for domains in [
            vec![Domain { first: 1, count: 2 }],
            vec![Domain { first: 0, count: 0 }],
            vec![Domain { first: 0, count: 4 }],
            vec![Domain { first: 0, count: 2 }, Domain { first: 1, count: 2 }],
            vec![Domain { first: 0, count: 2 }, Domain { first: 3, count: 1 }],
            vec![Domain {
                first: 0,
                count: u32::MAX,
            }],
        ] {
            assert!(matches!(
                resolve(&domains, &[], &[true; 3]),
                Err(Error::InvalidAnchor)
            ));
        }
        let domains = [Domain { first: 0, count: 3 }];
        for entry in [
            Entry {
                domain: 1,
                offset: 0,
            },
            Entry {
                domain: 0,
                offset: 3,
            },
        ] {
            assert!(matches!(
                resolve(&domains, &[entry], &[true; 3]),
                Err(Error::InvalidAnchor)
            ));
        }
        assert_eq!(
            reservation(MAX_CELLS + 1, 0, 0),
            Err(Error::BackendCapacity)
        );
        assert_eq!(
            reservation(0, MAX_CELLS + 1, 0),
            Err(Error::BackendCapacity)
        );
        assert_eq!(resolve(&[], &[], &[]).unwrap().0, vec![]);
    }
    fn linear_program(body: &[u32]) -> Program {
        let exit = (7 + body.len()) as u16;
        let mut words = vec![
            word(CAPTURE, 0, 0),
            word(REPEAT_BEGIN, 0, 0),
            word(REPEAT_BUMP, 0, 0),
            word(FORK, GREEDY, exit),
            word(NEXT, 0, 0),
        ];
        words.extend_from_slice(body);
        words.extend([
            word(REPEAT_BUMP, 0, 0),
            word(GOTO, 0, 3),
            word(REPEAT_END, 0, 0),
            word(EMIT, 0, 0),
        ]);
        Program::new(
            words,
            vec![b"a".to_vec(), b"b".to_vec()],
            vec![],
            vec![0, 1],
            vec![],
        )
        .unwrap()
        .with_tree_sitter_query_semantics()
        .unwrap()
    }

    #[test]
    fn bulk_closure_keeps_complete_frame_capture_values_and_exit_positions() {
        let program = linear_program(&[]);
        let tape = document("x\n# a\n# bb\n# ccc\n");
        let siblings = tape.transducer_children(0).collect::<Vec<_>>();
        assert_eq!(siblings.len(), 4);
        let expression = siblings[0];
        let identifier = tape.transducer_children(expression).next().unwrap();
        assert_ne!(expression, identifier);
        assert_eq!(
            tape.transducer_span(expression),
            tape.transducer_span(identifier)
        );
        let base = State {
            pc: 3,
            node: siblings[0],
            cursors: vec![Some(0)],
            repeats: vec![0],
            captures: vec![(0, expression, 0, 1), (1, expression, 0, 1)],
        };
        let mut frontier = (0..8)
            .map(|index| Item {
                position: vec![index],
                state: base.clone(),
                kind: Kind::Active,
            })
            .collect::<Vec<_>>();
        frontier[0].state.node = siblings[2];
        frontier[2].state.node = siblings[2]; // exact duplicate of the first entry
        frontier[3].state.cursors[0] = Some(expression);
        frontier[4].state.repeats.insert(0, 1);
        frontier[5].state.captures.reverse();
        frontier[6].state.captures.push((1, expression, 0, 1));
        frontier[7].state.captures[0].1 = identifier; // equal geometry, different identity
        let (actual, _, reserved) = close(&program, &tape, &frontier, 3).unwrap().unwrap();
        assert_eq!(actual.len(), 7);
        assert!(reserved <= LIMITS.payload_bytes);
        for (actual, index) in actual.iter().zip([0, 1, 3, 4, 5, 6, 7]) {
            let mut expected = frontier[index].state.clone();
            expected.node = if index == 1 { siblings[1] } else { siblings[3] };
            expected.pc = 7;
            assert_eq!(actual.state, expected, "entry={index}");
            assert_eq!(actual.position, vec![index, 1]);
            assert!(matches!(actual.kind, Kind::Active));
        }
        let mut invalid = frontier.clone();
        invalid[0].state.node = u32::MAX;
        assert!(matches!(
            close(&program, &tape, &invalid, 3),
            Err(Error::InvalidAnchor)
        ));
        invalid = frontier.clone();
        invalid[0].position = invalid[1].position.clone();
        assert!(matches!(
            close(&program, &tape, &invalid, 3),
            Err(Error::InvalidAnchor)
        ));
        invalid = frontier.clone();
        invalid[0].state.repeats.clear();
        assert!(matches!(
            close(&program, &tape, &invalid, 3),
            Err(Error::InvalidOperand(3))
        ));
        let mut pending = frontier;
        pending.push(Item {
            position: vec![9],
            state: State { pc: 7, ..base },
            kind: Kind::Greedy {
                body: vec![9, 0],
                last_offer: vec![9],
            },
        });
        assert!(matches!(
            close(&program, &tape, &pending, 3),
            Err(Error::BackendFailure)
        ));
    }

    #[test]
    fn bulk_closure_preserves_cpu_results_when_earlier_entries_start_later() {
        for body_guard in [word(FLAGS, 0, 0), word(SIZE, 2, 4)] {
            for exit_size in 2..=5 {
                let program = Program::new(
                    vec![
                        word(CAPTURE, 0, 0),
                        word(REPEAT_BEGIN, 0, 0),
                        word(FORK, 0, 6),
                        word(NEXT, 0, 0),
                        word(NEXT, 0, 0),
                        word(GOTO, 0, 6),
                        word(REPEAT_BUMP, 0, 0),
                        word(FORK, GREEDY, 12),
                        word(NEXT, 0, 0),
                        body_guard,
                        word(REPEAT_BUMP, 0, 0),
                        word(GOTO, 0, 7),
                        word(REPEAT_END, 0, 0),
                        word(SIZE, 0, exit_size),
                        word(EMIT, 0, 0),
                    ],
                    vec![b"prefix".to_vec()],
                    vec![],
                    vec![0],
                    vec![],
                )
                .unwrap()
                .with_tree_sitter_query_semantics()
                .unwrap();
                let tape = document("#a\n#aa\n#aaa\n#aaaa\n");
                let expected = program.execute_roots(&tape, [1]).unwrap();
                let selected_sizes = if split(body_guard).0 == SIZE {
                    [3, 4]
                } else {
                    [3, 5]
                };
                assert_eq!(
                    expected.len(),
                    usize::from(selected_sizes.contains(&exit_size))
                );
                let mut measured = ScheduleObservation::default();
                let actual = execute_bulk(&program, &tape, 1, &mut measured).unwrap();
                assert_eq!(actual, expected, "body={body_guard} exit={exit_size}");
                assert_eq!(measured.closures, 1);
                assert!(measured.waves < 64);
            }
        }
    }

    #[test]
    fn bulk_closure_proof_selects_only_bounded_scalar_bodies_without_changing_acceptance() {
        let tape = document("#a\n#aa\n#aaa\n#aaaa\n");
        for body in [
            vec![],
            vec![word(SIZE, 2, 4)],
            vec![word(FLAGS, 0, 0); 64],
            vec![word(SAVE_CURSOR, 0, 0)],
            vec![word(FLAGS, 0, 0); 65],
        ] {
            let program = linear_program(&body);
            let supported = body.len() <= 64 && !body.iter().any(|&w| split(w).0 == SAVE_CURSOR);
            assert_eq!(prove(&program, 3).unwrap().is_some(), supported);
            let expected = program.execute_roots(&tape, [1]).unwrap();
            let mut measured = ScheduleObservation::default();
            assert_eq!(
                execute_bulk(&program, &tape, 1, &mut measured).unwrap(),
                expected
            );
            assert_eq!(measured.closures, usize::from(supported));
        }
        assert!(prove(&greedy_diamond(), 3).unwrap().is_none());
        let mut ordinary = linear_program(&[]);
        ordinary.tree_sitter_query = false;
        assert!(close(&ordinary, &tape, &[], 3).unwrap().is_none());
        assert_eq!(
            execute(&ordinary, &tape, 1),
            ordinary.execute_roots(&tape, [1])
        );
    }

    #[test]
    fn bulk_closure_refuses_oversized_input_before_partial_publication() {
        let program = linear_program(&[]);
        let tape = document(&"# a\n".repeat(MAX_CELLS + 1));
        let frontier = [Item {
            position: vec![],
            state: State {
                pc: 3,
                node: 1,
                cursors: vec![],
                repeats: vec![0],
                captures: vec![],
            },
            kind: Kind::Active,
        }];
        assert!(matches!(
            close(&program, &tape, &frontier, 3),
            Err(Error::BackendCapacity)
        ));
    }
    #[test]
    fn production_closure_proof_preserves_observable_guarded_exits_and_query_acceptance() {
        let tape = document("#a\n#aa\n#aaa\n#aaaa\n");
        for body in [
            vec![],
            vec![word(SIZE, 2, 4)],
            vec![word(FLAGS, 0, 0); 64],
            vec![word(FLAGS, 0, 0); 65],
            vec![word(SAVE_CURSOR, 0, 0)],
        ] {
            let program = linear_program(&body);
            let regions = closure_proof::regions(&program).unwrap();
            let eligible = body.len() <= 64 && body.iter().all(|&w| split(w).0 != SAVE_CURSOR);
            assert_eq!(regions.len(), usize::from(eligible));
            for size in 2..=5 {
                let mut guarded = program.clone();
                guarded
                    .words
                    .insert(guarded.words.len() - 1, word(SIZE, 0, size));
                guarded.validate().unwrap();
                let expected = guarded.execute_roots(&tape, [1]).unwrap();
                // SAVE_CURSOR changes complete state identity on each advance.
                // Its four exit positions remain observable through general execution.
                let exit_count = usize::from(match body.first().copied().map(split) {
                    Some((SAVE_CURSOR, _, _)) => true,
                    Some((SIZE, _, _)) => size == 4,
                    _ => size == 5,
                });
                assert_eq!(expected.len(), exit_count, "body={body:?} size={size}");
                let mut measured = ScheduleObservation::default();
                assert_eq!(
                    execute_bulk(&guarded, &tape, 1, &mut measured).unwrap(),
                    expected
                );
                assert_eq!(measured.closures, usize::from(eligible));
            }
        }
        assert!(
            closure_proof::regions(&greedy_diamond())
                .unwrap()
                .is_empty()
        );
        let mut observable_body = linear_program(&[word(FLAGS, 0, 0)]);
        observable_body.words[5] = word(CAPTURE, 0, 0);
        assert_eq!(
            observable_body.validate(),
            Err(Error::InvalidGreedyRegion(3))
        );
        assert_eq!(
            closure_proof::regions(&observable_body),
            Err(Error::InvalidGreedyRegion(3))
        );
        let mut ordinary = linear_program(&[]);
        ordinary.tree_sitter_query = false;
        assert!(closure_proof::regions(&ordinary).unwrap().is_empty());
        assert_eq!(
            execute(&ordinary, &tape, 1),
            ordinary.execute_roots(&tape, [1])
        );
        let invalid = Program {
            words: vec![word(FORK, GREEDY, u16::MAX)],
            ..ordinary
        };
        assert_eq!(
            closure_proof::regions(&invalid),
            invalid.validate().map(|()| vec![])
        );
        assert_eq!(
            close(&invalid, &tape, &[], 0).err(),
            invalid.validate().err()
        );
    }

    #[test]
    fn bulk_closure_scalar_guards_retain_observable_cpu_endpoints() {
        let tape = document("#a\nx\n#aa\n");
        let comment = tape.transducer_kind(1);
        let guards = [
            (word(KIND, 0, comment), vec![]),
            (word(FIELD, 0, 0), vec![]),
            (word(FIELD, 0, 1), vec![]),
            (word(FLAGS, NAMED, 0), vec![]),
            (word(FLAGS, 0, u16::from(NAMED)), vec![]),
            (word(SIZE, 0, 3), vec![]),
            (word(SIZE, 1, 3), vec![]),
            (word(SIZE, 2, 3), vec![]),
            (word(SIZE, SIZE_LONG, 0), vec![3]),
            (word(SIZE, SIZE_LONG | 1, 0), vec![u64::MAX]),
            (word(SIZE, SIZE_LONG | 2, 0), vec![u64::MAX]),
        ];
        let mut comparisons = 0;
        for (guard, longs) in guards {
            let mut positives = 0;
            for endpoint_size in 1..=4 {
                let mut program = linear_program(&[word(FLAGS, 0, 0)]);
                program.words[5] = guard;
                program.longs = longs.clone();
                program.words.insert(9, word(SIZE, 0, endpoint_size));
                program.validate().unwrap();
                let expected = program.execute_roots(&tape, [1]).unwrap();
                positives += expected.len();
                let mut measured = ScheduleObservation::default();
                assert_eq!(
                    execute_bulk(&program, &tape, 1, &mut measured).unwrap(),
                    expected,
                    "guard={guard} endpoint={endpoint_size}"
                );
                assert_eq!(measured.closures, 1);
                comparisons += 1;
            }
            assert_eq!(
                positives, 1,
                "exactly one endpoint must be observable for guard={guard}"
            );
        }
        assert_eq!(comparisons, 44);
    }
}
