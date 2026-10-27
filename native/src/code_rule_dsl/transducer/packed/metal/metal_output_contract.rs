//! Source points and output instructions describe one checked resident meaning.
use super::Error;
use super::metal_batch_abi::NONE;
use super::packed_batch::PackedBatch;

fn range(start: u32, count: u32, length: usize) -> Result<std::ops::Range<usize>, Error> {
    let end = start.checked_add(count).ok_or(Error::InvalidAnchor)?;
    if end as usize > length {
        return Err(Error::InvalidAnchor);
    }
    Ok(start as usize..end as usize)
}

pub(super) fn validate(batch: &PackedBatch) -> Result<(), Error> {
    let mut line_end = 0;
    for file in &batch.files {
        let source =
            &batch.source[range(file.source_start, file.source_length, batch.source.len())?];
        std::str::from_utf8(source).map_err(|_| Error::InvalidAnchor)?;
        let lines = range(file.line_start, file.line_count, batch.line_starts.len())?;
        if lines.start != line_end {
            return Err(Error::InvalidAnchor);
        }
        line_end = lines.end;
        let expected =
            std::iter::once(file.source_start).chain(source.iter().enumerate().filter_map(
                |(index, &byte)| (byte == b'\n').then_some(file.source_start + index as u32 + 1),
            ));
        if !expected.eq(batch.line_starts[lines].iter().copied()) {
            return Err(Error::InvalidAnchor);
        }
    }
    if line_end != batch.line_starts.len() {
        return Err(Error::InvalidAnchor);
    }
    for program in &batch.programs {
        let words = program
            .output_count
            .checked_mul(6)
            .ok_or(Error::InvalidAnchor)?;
        let instructions =
            &batch.assertion_data[range(program.output_start, words, batch.assertion_data.len())?];
        for instruction in instructions.chunks_exact(6) {
            let [kind, first, second, third, fourth, fifth] = *instruction else {
                return Err(Error::InvalidAnchor);
            };
            let capture = |label| label < program.label_count;
            let string = |index| index < program.string_count;
            let optional_string = |index| index == NONE || string(index);
            let valid = match kind {
                0 => string(first) && optional_string(second) && [third, fourth, fifth] == [0; 3],
                1 => {
                    capture(first)
                        && string(second)
                        && optional_string(third)
                        && [fourth, fifth] == [0; 2]
                }
                2 => capture(first),
                3 => {
                    let values =
                        range(program.value_start, program.value_count, batch.values.len())?;
                    capture(first)
                        && values.contains(&(second as usize))
                        && batch.values[second as usize].kind == 2
                        && [third, fourth, fifth] == [0; 3]
                }
                4 => capture(first) && capture(second) && [third, fourth, fifth] == [0; 3],
                5 | 6 => capture(first) && [second, third, fourth, fifth] == [0; 4],
                _ => false,
            };
            if !valid {
                return Err(Error::InvalidAnchor);
            }
        }
    }
    Ok(())
}
