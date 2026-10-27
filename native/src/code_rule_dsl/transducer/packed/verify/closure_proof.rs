//! One eligibility proof for capture-free terminal sibling closure.
use super::{Error, FIELD, FLAGS, FORK, GREEDY, KIND, Program, SIZE, split};

/// Guard words borrow the validated Program. This proof cannot outlive that
/// immutable instruction view. Full-value domain grouping remains a separate duty.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Region<'a> {
    pub header: usize,
    pub exit: usize,
    pub repeat: u16,
    pub guards: &'a [u32],
}

pub(super) fn regions(program: &Program) -> Result<Vec<Region<'_>>, Error> {
    // Malformed instructions or query regions are producer errors, not a reason
    // to fall through to general execution. Ordinary programs remain ineligible.
    program.validate()?;
    if !program.tree_sitter_query {
        return Ok(Vec::new());
    }
    let mut regions = Vec::new();
    for (header, &word) in program.words.iter().enumerate() {
        let (FORK, GREEDY, exit) = split(word) else {
            continue;
        };
        // The approved query contract already proves NEXT, the matching repeat
        // bump/return/exit, controlled entry, and capture-free terminal behavior.
        let exit = usize::from(exit);
        let guards = &program.words[header + 2..exit - 2];
        if guards.len() > 64
            || guards
                .iter()
                .any(|&w| !matches!(split(w).0, KIND | FIELD | FLAGS | SIZE))
        {
            continue;
        }
        regions.push(Region {
            header,
            exit,
            repeat: split(program.words[exit]).2,
            guards,
        });
    }
    Ok(regions)
}
