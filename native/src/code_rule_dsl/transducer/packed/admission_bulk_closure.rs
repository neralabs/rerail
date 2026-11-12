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
#[path = "admission_bulk_closure/tests.rs"]
mod tests;
