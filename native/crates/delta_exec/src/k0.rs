//! L1's K0 reading on the host (decision 0686): the twins' walk (`rerail_delta_twins::k0::walk`) over a document's
//! tape, its symbols read from its symbols block and the atoms, never made. `rows` collects a document's rows for
//! checks.

use crate::kform::Refused;
use crate::tape::{Text, reads};

/// Walks document `d`'s tape (`walk`), its node → symbol facts in `syms` and the names it compares in `atoms` (both
/// blocks' bytes, where they lie).
pub fn walk(text: &Text, d: i64, syms: &[u8], atoms: &[u8], put: &mut dyn FnMut(usize, &[i64])) -> Result<(), Refused> {
    rerail_delta_twins::k0::walk(&reads(text), d, syms, atoms, put).map_err(|(n, why)| Refused { module: d, rule: n as i64, why: why.to_string() })
}

/// The K-form rows of one document, by relation, for checks and tests.
pub fn rows(text: &Text, d: i64, syms: &[u8], atoms: &[u8]) -> Result<[std::collections::BTreeSet<Vec<i64>>; 8], Refused> {
    let mut out: [std::collections::BTreeSet<Vec<i64>>; 8] = Default::default();
    // copy: not block data (a check's rows)
    walk(text, d, syms, atoms, &mut |r, row| {
        out[r].insert(row.to_vec());
    })?;
    Ok(out)
}
