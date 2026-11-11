//! Immutable value code: ordered NFA traces and checked Lua byte masks.
use super::Error;
use super::metal_batch_abi::{NONE, ValueRecord};
use super::packed_batch::PackedBatch;
use regex_automata::util::look::{Look, LookMatcher};
use std::collections::BTreeMap;

#[path = "metal_lua_code.rs"]
mod lua;

fn size(n: usize) -> Result<u32, Error> {
    u32::try_from(n).map_err(|_| Error::BackendCapacity)
}

/// Boundary categories include every distinction made by the default LookMatcher.
/// The final context denotes an interior UTF-8 byte, rather than a scalar boundary.
fn contexts(kinds: &[Look]) -> Vec<u32> {
    let representatives = ["", "\n", "\r", "A", "!", "é", "💥"];
    let matcher = LookMatcher::new();
    let mut masks = Vec::with_capacity(50);
    for previous in representatives {
        for current in representatives {
            let text = format!("{previous}{current}");
            masks.push(kinds.iter().fold(0, |mask, &kind| {
                mask | if matcher.matches(kind, text.as_bytes(), previous.len()) {
                    kind as u32
                } else {
                    0
                }
            }));
        }
    }
    masks.push(kinds.iter().fold(0, |mask, &kind| {
        mask | if matcher.matches(kind, "é".as_bytes(), 1) {
            kind as u32
        } else {
            0
        }
    }));
    masks
}

/// Unit: owner state, event kind, byte range, visited state, parent unit,
/// inherited whole-match capture overrides, and NFA state kind.
/// A range event charges work even when its byte does not match.
struct Traces<'a> {
    batch: &'a PackedBatch,
    value: ValueRecord,
    seen: Vec<u32>,
    epoch: u32,
    mask: u32,
    units: Vec<[u32; 8]>,
}
impl Traces<'_> {
    fn walk(&mut self, first: u32, owner: u32, range: [u32; 2]) -> Result<(), Error> {
        self.epoch = self.epoch.checked_add(1).ok_or(Error::BackendCapacity)?;
        let mut pending = vec![(first, NONE, 0u32)];
        while let Some((state, parent, captures)) = pending.pop() {
            if state >= self.value.state_count {
                return Err(Error::InvalidAnchor);
            }
            let record = self.batch.nfa_states[self.value.state_start as usize + state as usize];
            let unit = size(self.units.len())?;
            self.units.push([
                owner,
                0,
                range[0],
                range[1],
                state,
                parent,
                captures,
                record.kind,
            ]);
            // Duplicate attempts still cost work; only their outgoing edges disappear.
            if self.seen[state as usize] == self.epoch {
                continue;
            }
            self.seen[state as usize] = self.epoch;
            match record.kind {
                0 | 4 | 5 => {}
                1 if self.mask & record.data_start != 0 => {
                    pending.push((record.next, unit, captures));
                }
                1 => {}
                2 => {
                    let first = record.data_start as usize;
                    let end = first
                        .checked_add(record.data_count as usize)
                        .ok_or(Error::InvalidAnchor)?;
                    let targets = self
                        .batch
                        .nfa_data
                        .get(first..end)
                        .ok_or(Error::InvalidAnchor)?;
                    pending.extend(targets.iter().rev().map(|&target| (target, unit, captures)));
                }
                3 => {
                    let captures = captures
                        | if record.data_start < 2 {
                            1 << record.data_start
                        } else {
                            0
                        };
                    pending.push((record.next, unit, captures));
                }
                _ => return Err(Error::InvalidAnchor),
            }
        }
        Ok(())
    }
    fn compile(mut self) -> Result<Vec<[u32; 8]>, Error> {
        self.walk(self.value.operand, NONE, [0, 255])?;
        for owner in 0..self.value.state_count {
            let state = self.batch.nfa_states[self.value.state_start as usize + owner as usize];
            if state.kind != 0 {
                continue;
            }
            let first = state.data_start as usize;
            let end = first
                .checked_add(state.data_count as usize)
                .ok_or(Error::InvalidAnchor)?;
            let ranges = self
                .batch
                .nfa_ranges
                .get(first..end)
                .ok_or(Error::InvalidAnchor)?;
            for range in ranges {
                self.units
                    .push([owner, 1, range.start, range.end, NONE, NONE, 0, NONE]);
                self.walk(range.target, owner, [range.start, range.end])?;
            }
        }
        Ok(self.units)
    }
}

/// Blob header: value count, descriptor offset, Unicode word ranges, range count.
/// Each descriptor contains item count, scratch width, table offset, and engine tag.
/// Tag zero names ordered NFA traces. Tag one names checked Lua frames and masks.
/// Each of its 50 context records names a unit word offset and exact unit count.
pub(super) fn compile(batch: &PackedBatch) -> Result<Vec<u32>, Error> {
    let descriptors = batch
        .values
        .len()
        .checked_mul(4)
        .and_then(|n| n.checked_add(4))
        .ok_or(Error::BackendCapacity)?;
    let mut words = vec![0u32; descriptors];
    let range_count = *batch.nfa_data.first().ok_or(Error::InvalidAnchor)?;
    words[..4].copy_from_slice(&[
        size(batch.values.len())?,
        4,
        size(descriptors)?,
        range_count,
    ]);
    let range_words = (range_count as usize)
        .checked_mul(2)
        .ok_or(Error::BackendCapacity)?;
    words.extend_from_slice(
        batch
            .nfa_data
            .get(1..1 + range_words)
            .ok_or(Error::InvalidAnchor)?,
    );
    for (index, &value) in batch.values.iter().enumerate() {
        if value.kind == 3 {
            let descriptor = lua::append(batch, value, &mut words)?;
            words[4 + index * 4..8 + index * 4].copy_from_slice(&descriptor);
            continue;
        }
        if value.kind != 2 {
            continue;
        }
        if value.state_count == 0 || value.operand >= value.state_count {
            return Err(Error::InvalidAnchor);
        }
        let end = value
            .state_start
            .checked_add(value.state_count)
            .ok_or(Error::InvalidAnchor)? as usize;
        let states = batch
            .nfa_states
            .get(value.state_start as usize..end)
            .ok_or(Error::InvalidAnchor)?;
        let mut kinds = Vec::new();
        for state in states.iter().filter(|state| state.kind == 1) {
            let kind = Look::from_repr(state.data_start).ok_or(Error::InvalidAnchor)?;
            if !kinds.contains(&kind) {
                kinds.push(kind);
            }
        }
        let width = value
            .state_count
            .checked_next_power_of_two()
            .ok_or(Error::BackendCapacity)?;
        let table = words.len();
        words.resize(table.checked_add(100).ok_or(Error::BackendCapacity)?, 0);
        words[4 + index * 4..8 + index * 4].copy_from_slice(&[
            value.state_count,
            width,
            size(table)?,
            0,
        ]);
        let mut compiled = BTreeMap::<u32, [u32; 2]>::new();
        for (context, mask) in contexts(&kinds).into_iter().enumerate() {
            let span = if let Some(&span) = compiled.get(&mask) {
                span
            } else {
                let units = Traces {
                    batch,
                    value,
                    seen: vec![0; value.state_count as usize],
                    epoch: 0,
                    mask,
                    units: Vec::new(),
                }
                .compile()?;
                let span = [size(words.len())?, size(units.len())?];
                words.extend(units.into_iter().flatten());
                compiled.insert(mask, span);
                span
            };
            words[table + context * 2..table + context * 2 + 2].copy_from_slice(&span);
        }
    }
    size(words.len())?;
    Ok(words)
}
