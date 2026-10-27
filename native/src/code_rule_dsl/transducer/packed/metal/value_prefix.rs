//! Necessary first-byte conditions with bounded rejection work.
//! A zero work bound requires ordinary value execution.
use super::metal_batch_abi::ValueRecord;
use super::{Error, packed_batch::PackedBatch};

fn slice<T>(data: &[T], start: u32, count: u32) -> Result<&[T], Error> {
    let end = start.checked_add(count).ok_or(Error::BackendFailure)?;
    data.get(start as usize..end as usize)
        .ok_or(Error::BackendFailure)
}

fn regex(batch: &PackedBatch, value: ValueRecord) -> Result<[u32; 9], Error> {
    let states = slice(&batch.nfa_states, value.state_start, value.state_count)?;
    if states.is_empty()
        || value.operand >= value.state_count
        || value.auxiliary >= value.state_count
    {
        return Err(Error::BackendFailure);
    }
    // Validate the producer even when this value cannot authorize a prefix.
    for state in states {
        match state.kind {
            0 => {
                for range in slice(&batch.nfa_ranges, state.data_start, state.data_count)? {
                    if range.start > range.end
                        || range.end > 255
                        || range.target >= value.state_count
                    {
                        return Err(Error::BackendFailure);
                    }
                }
            }
            1 if state.next < value.state_count
                && regex_automata::util::look::Look::from_repr(state.data_start).is_some() => {}
            2 => {
                if slice(&batch.nfa_data, state.data_start, state.data_count)?
                    .iter()
                    .any(|&target| target >= value.state_count)
                {
                    return Err(Error::BackendFailure);
                }
            }
            3 if state.next < value.state_count => {}
            4 | 5 => {}
            _ => return Err(Error::BackendFailure),
        }
    }
    // An unanchored start can consume an arbitrary prefix before the pattern.
    if value.operand != value.auxiliary {
        return Ok([0; 9]);
    }
    let mut proof = [0; 9];
    let mut seen = vec![false; states.len()];
    let mut pending = vec![value.operand];
    let mut work = 0usize;
    while let Some(index) = pending.pop() {
        work = work.checked_add(1).ok_or(Error::BackendCapacity)?;
        if std::mem::replace(&mut seen[index as usize], true) {
            continue;
        }
        let state = states[index as usize];
        match state.kind {
            0 => {
                for range in slice(&batch.nfa_ranges, state.data_start, state.data_count)? {
                    work = work.checked_add(1).ok_or(Error::BackendCapacity)?;
                    for byte in range.start..=range.end {
                        proof[byte as usize / 32] |= 1 << (byte % 32);
                    }
                }
            }
            // Ignoring look conditions gives a superset of possible starts.
            1 | 3 => pending.push(state.next),
            2 => pending.extend_from_slice(slice(
                &batch.nfa_data,
                state.data_start,
                state.data_count,
            )?),
            4 => {}
            // A nullable prefix can accept before reading any byte.
            5 => return Ok([0; 9]),
            _ => return Err(Error::BackendFailure),
        }
    }
    if work > super::MAX_STEPS_PER_PATH {
        return Ok([0; 9]);
    }
    proof[8] = u32::try_from(work).map_err(|_| Error::BackendCapacity)?;
    Ok(proof)
}

fn lua(batch: &PackedBatch, value: ValueRecord) -> Result<[u32; 9], Error> {
    let instructions = slice(
        &batch.lua_instructions,
        value.state_start,
        value.state_count,
    )?;
    if value.auxiliary != 0 || value.operand >> 2 > 32 {
        return Err(Error::BackendFailure);
    }
    if value.operand & 1 == 0 {
        return Ok([0; 9]);
    }
    for (prefix, &instruction) in instructions.iter().enumerate() {
        match instruction.kind {
            3..=5 => continue,
            0 if instruction.first == 0 || instruction.first == 4 => {
                let matcher = super::value_packing::lua_matcher(batch, instruction)
                    .map_err(|_| Error::BackendFailure)?;
                let mut proof = [0; 9];
                for byte in 0..=255u8 {
                    if matcher.matches(byte) {
                        proof[byte as usize / 32] |= 1 << (byte % 32);
                    }
                }
                proof[8] = u32::try_from(prefix.checked_add(4).ok_or(Error::BackendCapacity)?)
                    .map_err(|_| Error::BackendCapacity)?;
                return Ok(proof);
            }
            // Optional atoms and other zero-width conditions need the frontier.
            _ => return Ok([0; 9]),
        }
    }
    Ok([0; 9])
}

pub(super) fn build(batch: &PackedBatch) -> Result<Vec<[u32; 9]>, Error> {
    batch
        .values
        .iter()
        .map(|&value| match value.kind {
            0 | 1 => Ok([0; 9]),
            2 => regex(batch, value),
            3 => lua(batch, value),
            _ => Err(Error::BackendFailure),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy_dsl::transducer::{
        LuaPattern, MAX_STEPS_PER_PATH, TextPredicate,
        value::{ByteNfa, ValueProgram},
        value_packing::PackedValueTables,
    };

    fn packed(values: &[ValueProgram]) -> PackedBatch {
        let mut tables = PackedValueTables::with_unicode_word_data();
        tables.push_program(values).unwrap();
        PackedBatch {
            values: tables.values,
            nfa_states: tables.nfa_states,
            nfa_data: tables.nfa_data,
            nfa_ranges: tables.nfa_ranges,
            lua_instructions: tables.lua_instructions,
            lua_members: tables.lua_members,
            ..PackedBatch::default()
        }
    }
    fn possible(proof: [u32; 9], text: &[u8], captures: usize) -> bool {
        if proof[8] == 0 || proof[8] as usize * captures > MAX_STEPS_PER_PATH {
            return true;
        }
        text.first()
            .is_some_and(|&byte| proof[byte as usize / 32] & (1 << (byte % 32)) != 0)
    }
    fn evaluate(value: &ValueProgram, bytes: &[u8]) -> (Result<bool, Error>, usize) {
        match value {
            ValueProgram::Regex(nfa) => nfa.test_work_count(bytes),
            ValueProgram::Lua(lua) => lua.test_work_count(bytes),
            _ => unreachable!(),
        }
    }
    #[test]
    fn value_prefix_rejections_preserve_complete_match_truth_and_bound_every_rejected_byte_domain()
    {
        let mut values = [
            "^[A-Z]",
            "^_*[A-Z][A-Z\\d_]*$",
            "^[A-Z][A-Z_]*$",
            "^a+b$",
            "^\\p{Greek}+$",
            "^\\bA",
            "^\\x00A$",
        ]
        .into_iter()
        .map(|s| ValueProgram::Regex(ByteNfa::compile(s).unwrap()))
        .collect::<Vec<_>>();
        values.extend(
            [
                "^[A-Z].*[a-z]",
                "^[A-Z][A-Z_0-9]*$",
                "^__[a-zA-Z0-9_]*__$",
                "^%u+$",
                "^(A)%1$",
            ]
            .into_iter()
            .map(|s| {
                ValueProgram::lower(
                    &TextPredicate::LuaPattern {
                        source: 0,
                        compiled: LuaPattern::parse(s.as_bytes()).unwrap(),
                    },
                    &[s.as_bytes().to_vec()],
                )
                .unwrap()
            }),
        );
        let proofs = build(&packed(&values)).unwrap();
        assert_eq!(proofs.len(), values.len());
        for (index, (value, proof)) in values.iter().zip(proofs).enumerate() {
            assert!(
                !possible(proof, b"lower", 1),
                "the proved class must reject its lowercase domain, value={index}"
            );
            let mut accepted = 0;
            for first in 0..=255u8 {
                for suffix in [
                    b"".as_slice(),
                    b"aZ",
                    b"___",
                    b"\n",
                    "é".as_bytes(),
                    "Α".as_bytes(),
                ] {
                    let bytes = [vec![first], suffix.to_vec()].concat();
                    let (actual, work) = evaluate(value, &bytes);
                    if actual == Ok(true) {
                        accepted += 1;
                    }
                    if !possible(proof, &bytes, 1) {
                        assert_eq!(actual, Ok(false), "value={index} bytes={bytes:?}");
                        assert!(
                            work <= proof[8] as usize,
                            "value={index} work={work} bound={}",
                            proof[8]
                        );
                    }
                }
            }
            for bytes in [
                b"A".as_slice(),
                b"aab",
                b"__magic__",
                b"AA",
                "Α".as_bytes(),
                b"\0A",
            ] {
                let (actual, _) = evaluate(value, bytes);
                if actual == Ok(true) {
                    accepted += 1;
                    assert!(possible(proof, bytes, 1));
                }
            }
            assert!(
                accepted > 0,
                "each oracle domain must include successful matches, value={index}"
            );
            for bytes in [vec![], vec![b'x'; 65_537]] {
                assert!(!possible(proof, &bytes, 1));
                let (actual, work) = evaluate(value, &bytes);
                assert_eq!(actual, Ok(false));
                assert!(work <= proof[8] as usize);
            }
            let excessive = MAX_STEPS_PER_PATH / proof[8] as usize + 1;
            assert!(
                possible(proof, b"lower", excessive),
                "a rejection cannot hide a possible aggregate work failure"
            );
        }
    }
    #[test]
    fn value_prefix_keeps_unanchored_nullable_and_optional_prefixes_on_the_value_frontier() {
        let mut values = ["A", "(?m)^A", "^a?$", "^", "$"]
            .into_iter()
            .map(|s| ValueProgram::Regex(ByteNfa::compile(s).unwrap()))
            .collect::<Vec<_>>();
        values.extend(["A", "^A?b", "^A*b", "^%f[%w]A"].into_iter().map(|s| {
            ValueProgram::lower(
                &TextPredicate::LuaPattern {
                    source: 0,
                    compiled: LuaPattern::parse(s.as_bytes()).unwrap(),
                },
                &[s.as_bytes().to_vec()],
            )
            .unwrap()
        }));
        for proof in build(&packed(&values)).unwrap() {
            assert!(possible(proof, b"", 1));
            for byte in 0..=255u8 {
                assert!(possible(proof, &[byte], 1));
            }
        }
    }
    #[test]
    fn value_prefix_rejects_inconsistent_value_producers_before_masking_any_input() {
        let values = [ValueProgram::Regex(ByteNfa::compile("^[A-Z]").unwrap())];
        let original = packed(&values);
        let mut bad = original.clone();
        bad.values[0].state_start = u32::MAX;
        assert_eq!(build(&bad), Err(Error::BackendFailure));
        let mut bad = original.clone();
        bad.values[0].operand = bad.values[0].state_count;
        assert_eq!(build(&bad), Err(Error::BackendFailure));
        let mut bad = original;
        bad.nfa_ranges[0].target = u32::MAX;
        assert_eq!(build(&bad), Err(Error::BackendFailure));
    }
}
