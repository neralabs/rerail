//! Case-exact leaf terms and the nodes they do not cover, following tape_store.py. SQLite's B-tree
//! keys retain the full term, so hash collisions need no confirmation path. The store-wide residue
//! cap is applied when the rows are published (`content_store::prefilter`).
use super::LANG_STRIDE;
use crate::code_rule_dsl::tape::TapeDoc;
use crate::content_store::{StoreError, StoreResult, TapePrefilter};
use crate::content_store::prefilter::{INTERIOR, TOO_BIG, UNCLEAN};
use std::collections::BTreeMap;

pub(super) const TERM_CAP: usize = 96;
const RESIDUE_BYTES: usize = crate::content_store::prefilter::RESIDUE_CAP * TERM_CAP;

/// Reads only the tape, so it runs outside the store's write lock.
pub(super) fn build(set: &mut TapePrefilter, doc: &TapeDoc, interrupted: &impl Fn() -> bool) -> StoreResult<()> {
    let base = set.lang * LANG_STRIDE;
    let mut sizes: BTreeMap<u32, usize> = BTreeMap::new();
    for pre in 0..doc.transducer_len() as u32 {
        if pre.is_multiple_of(256) && interrupted() {
            return Err(StoreError::Invalid("index construction interrupted".to_owned()));
        }
        let kind = base
            + u32::from(if doc.transducer_flags(pre) & 8 != 0 {
                0
            } else {
                doc.transducer_kind(pre)
            });
        let text = doc.transducer_text_str(pre);
        let clean = !text.is_empty()
            && text.len() <= TERM_CAP * 4
            && text.chars().count() <= TERM_CAP
            && !text.chars().any(|c| c.is_whitespace() || c < '\u{20}');
        let coverage = set.cover.entry(kind).or_default();
        let end = doc.transducer_descendants(pre).end;
        let mut child = pre + 1;
        let mut interior = false;
        while child < end {
            if doc.transducer_flags(child) & 1 != 0 {
                interior = true;
                break;
            }
            child = doc.transducer_descendants(child).end;
        }
        if interior {
            coverage.0 |= INTERIOR | if clean { 0 } else { UNCLEAN };
            set.missing.entry(kind).or_default().push(pre);
        } else if clean {
            set.postings.entry((kind, text.to_owned())).or_default().push(pre);
        } else {
            coverage.0 |= UNCLEAN;
            set.missing.entry(kind).or_default().push(pre);
            coverage.1 += 1;
            let bytes = sizes.entry(kind).or_default();
            *bytes = bytes.saturating_add(text.len());
            if *bytes > RESIDUE_BYTES {
                coverage.0 |= TOO_BIG;
                set.residue.remove(&kind);
            } else if coverage.0 & TOO_BIG == 0 {
                set.residue.entry(kind).or_default().push((pre, text.to_owned()));
            }
        }
    }
    Ok(())
}
