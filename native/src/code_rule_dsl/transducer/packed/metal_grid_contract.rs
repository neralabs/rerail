//! Reject inconsistent resident addresses before a GPU command can read them.
use super::metal_grid::MetalExecutionGrid;
use super::metal_grid_plan::ResidentInput;
use super::*;

fn range(start: u32, count: u32, length: usize) -> Result<Range<usize>, Error> {
    let end = start.checked_add(count).ok_or(Error::InvalidAnchor)?;
    if end as usize > length {
        return Err(Error::InvalidAnchor);
    }
    Ok(start as usize..end as usize)
}

fn selector_interval(
    grid: &MetalExecutionGrid,
    file: u32,
    start: u32,
    count: u32,
) -> Result<(), Error> {
    if count == 0 {
        return Err(Error::InvalidAnchor);
    }
    let interval = range(start, count, grid.roots.len())?;
    if grid.roots[interval.start].file != file || grid.roots[interval.end - 1].file != file {
        return Err(Error::InvalidAnchor);
    }
    Ok(())
}

pub(super) fn validate(resident: &ResidentInput) -> Result<(), Error> {
    let batch = &resident.input.packed;
    super::metal_output_contract::validate(batch)?;
    let grid = &resident.input.grid;
    if batch.header.node_count as usize != batch.nodes.len()
        || batch.header.file_count as usize != batch.files.len()
        || batch.header.program_count as usize != batch.programs.len()
        || resident.witness_limits.len() != batch.programs.len()
    {
        return Err(Error::InvalidAnchor);
    }
    for (index, root) in grid.roots.iter().enumerate() {
        let file = batch
            .files
            .get(root.file as usize)
            .ok_or(Error::InvalidAnchor)?;
        if root.node >= file.node_count
            || root.semantic_root_ordinal as usize != index
            || (index > 0
                && (grid.roots[index - 1].file, grid.roots[index - 1].node)
                    >= (root.file, root.node))
        {
            return Err(Error::InvalidAnchor);
        }
    }
    // All navigation representations must describe the same preorder tree.
    // Range checks alone admit valid addresses with contradictory relationships.
    let mut node_end = 0;
    let mut source_end = 0;
    let mut child_end = 0;
    for file in &batch.files {
        let nodes = range(file.node_start, file.node_count, batch.nodes.len())?;
        let source = range(file.source_start, file.source_length, batch.source.len())?;
        if nodes.is_empty() || nodes.start != node_end || source.start != source_end {
            return Err(Error::InvalidAnchor);
        }
        node_end = nodes.end;
        source_end = source.end;
        for index in nodes.clone() {
            let node = batch.nodes[index];
            let parent = node.parent;
            if index == nodes.start {
                if parent != u32::MAX
                    || node.next_any != u32::MAX
                    || node.next_named != u32::MAX
                    || node.descendant_end as usize != nodes.end
                {
                    return Err(Error::InvalidAnchor);
                }
            } else {
                if !nodes.contains(&(parent as usize)) || parent as usize >= index {
                    return Err(Error::InvalidAnchor);
                }
                let parent = batch.nodes[parent as usize];
                if node.source_start < parent.source_start || node.source_end > parent.source_end {
                    return Err(Error::InvalidAnchor);
                }
            }
            if (node.source_start as usize) < source.start
                || node.source_end < node.source_start
                || node.source_end as usize > source.end
                || node.descendant_end as usize <= index
                || node.descendant_end as usize > nodes.end
            {
                return Err(Error::InvalidAnchor);
            }
            let children = range(node.child_start, node.child_count, batch.children.len())?;
            if children.start != child_end {
                return Err(Error::InvalidAnchor);
            }
            child_end = children.end;
            let children = &batch.children[children];
            let mut expected_child = index + 1;
            for &child in children {
                if !nodes.contains(&(child as usize))
                    || child as usize != expected_child
                    || batch.nodes[child as usize].parent as usize != index
                {
                    return Err(Error::InvalidAnchor);
                }
                expected_child = batch.nodes[child as usize].descendant_end as usize;
            }
            if expected_child != node.descendant_end as usize {
                return Err(Error::InvalidAnchor);
            }
            let mut next_any = u32::MAX;
            let mut next_named = u32::MAX;
            for &child in children.iter().rev() {
                let child_node = batch.nodes[child as usize];
                if child_node.next_any != next_any || child_node.next_named != next_named {
                    return Err(Error::InvalidAnchor);
                }
                next_any = child;
                if child_node.flags & 1 != 0 {
                    next_named = child;
                }
            }
        }
    }
    if node_end != batch.nodes.len()
        || source_end != batch.source.len()
        || child_end != batch.children.len()
    {
        return Err(Error::InvalidAnchor);
    }
    for program in &batch.programs {
        if program.structural_count == 0 {
            return Err(Error::InvalidAnchor);
        }
        range(
            program.structural_start,
            program.structural_count,
            batch.words.len(),
        )?;
        range(program.long_start, program.long_count, batch.longs.len())?;
        range(program.label_start, program.label_count, batch.labels.len())?;
        range(
            program.string_start,
            program.string_count,
            batch.strings.len(),
        )?;
    }
    for string in &batch.strings {
        range(
            string.byte_start,
            string.byte_length,
            batch.string_bytes.len(),
        )?;
    }
    for selector in &grid.selectors {
        let data = &grid.selector_data[range(
            selector.data_start,
            selector.data_count,
            grid.selector_data.len(),
        )?];
        if selector.root_count == 0
            || selector.file as usize >= batch.files.len()
            || selector.reserved != 0
        {
            return Err(Error::InvalidAnchor);
        }
        match selector.kind {
            1 if data.len() == 1 => {
                selector_interval(grid, selector.file, data[0], selector.root_count)?
            }
            2 if !data.is_empty() && data.len().is_multiple_of(3) => {
                let mut total = 0_u32;
                let mut previous = None;
                for run in data.chunks_exact(3) {
                    if run[2] != total || previous.is_some_and(|end| run[0] <= end) {
                        return Err(Error::InvalidAnchor);
                    }
                    selector_interval(grid, selector.file, run[0], run[1])?;
                    previous = Some(run[0].checked_add(run[1]).ok_or(Error::InvalidAnchor)?);
                    total = total.checked_add(run[1]).ok_or(Error::InvalidAnchor)?;
                }
                if total != selector.root_count {
                    return Err(Error::InvalidAnchor);
                }
            }
            3 if data.len() > 2 => {
                let blocks = data[1] as usize;
                let end = 2_usize.checked_add(blocks).ok_or(Error::InvalidAnchor)?;
                let ranks = data.get(2..end).ok_or(Error::InvalidAnchor)?;
                let bitmap = data.get(end..).ok_or(Error::InvalidAnchor)?;
                if blocks == 0
                    || bitmap.is_empty()
                    || bitmap.len().div_ceil(32) != blocks
                    || bitmap[0] & 1 == 0
                    || bitmap.last() == Some(&0)
                {
                    return Err(Error::InvalidAnchor);
                }
                let mut total = 0_u32;
                for (index, words) in bitmap.chunks(32).enumerate() {
                    if ranks[index] != total {
                        return Err(Error::InvalidAnchor);
                    }
                    for word in words {
                        total = total
                            .checked_add(word.count_ones())
                            .ok_or(Error::InvalidAnchor)?;
                    }
                }
                let last = *bitmap.last().ok_or(Error::InvalidAnchor)?;
                let width = (bitmap.len() - 1)
                    .checked_mul(32)
                    .and_then(|width| width.checked_add((32 - last.leading_zeros()) as usize))
                    .and_then(|width| u32::try_from(width).ok())
                    .ok_or(Error::InvalidAnchor)?;
                selector_interval(grid, selector.file, data[0], width)?;
                if total != selector.root_count {
                    return Err(Error::InvalidAnchor);
                }
            }
            4 if data.len() == selector.root_count as usize => {
                if data.windows(2).any(|pair| pair[0] >= pair[1]) {
                    return Err(Error::InvalidAnchor);
                }
                for &root in data {
                    selector_interval(grid, selector.file, root, 1)?;
                }
            }
            _ => return Err(Error::InvalidAnchor),
        }
    }
    let mut ordinal = 0_u32;
    for (index, evaluation) in grid.evaluation_spans.iter().enumerate() {
        let selector = grid
            .selectors
            .get(evaluation.selector as usize)
            .ok_or(Error::InvalidAnchor)?;
        if evaluation.evaluation as usize != index
            || evaluation.program as usize >= batch.programs.len()
            || evaluation.file != selector.file
            || evaluation.root_count != selector.root_count
            || evaluation.semantic_start != ordinal
        {
            return Err(Error::InvalidAnchor);
        }
        ordinal = ordinal
            .checked_add(evaluation.root_count)
            .ok_or(Error::InvalidAnchor)?;
    }
    if ordinal != grid.header.semantic_tuple_count {
        return Err(Error::InvalidAnchor);
    }
    for span in &grid.block_spans {
        let evaluation = grid
            .evaluation_spans
            .get(span.evaluation_span as usize)
            .ok_or(Error::InvalidAnchor)?;
        range(
            span.selector_offset,
            span.root_count,
            evaluation.root_count as usize,
        )?;
        if evaluation.semantic_start.checked_add(span.selector_offset) != Some(span.semantic_start)
        {
            return Err(Error::InvalidAnchor);
        }
    }
    Ok(())
}
