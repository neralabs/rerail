//! A document's entry for its views' reads (decision 0353 step 4c, `rerail_clif::view_reads::ViewDoc`): in the
//! block its projection's numbering is in, which it refers to. A code's kind atom and field atom are its language's,
//! in the codes' directory the binary holds (decision 0697), which the reads are passed: no entry makes them. No
//! per-node column: a node's row is read off the tape. The
//! tape's rows' derived columns (`tape_rows::Derived`: a gap's or a digest's atom, a child's place, the visit order),
//! where its program reads them, are runs of it too.

use super::cells::{Block, Cells, Desc};
use super::{Placement, Projected};
use rerail_clif::ids::Run;
use rerail_clif::view_reads::ViewDoc;
use std::sync::Arc;

/// A document's entry, in its projection's block (`Projected::entry`): its address.
pub struct ViewEntry {
    desc: *const ViewDoc,
}

// SAFETY: the entry is written once, inside its projection's `OnceLock` (`new`), and only read after.
unsafe impl Send for ViewEntry {}
unsafe impl Sync for ViewEntry {}

/// What an entry's runs point at that it made itself: a guest's spans as its host places them (`ViewDoc::placed`),
/// where no word of the entry maps them. A code's kind and field are its language's, in the codes' directory the
/// binary holds (decision 0697): no entry makes them.
pub struct Filled {
    _placed: Option<Cells<u64>>,
}

impl ViewEntry {
    /// `p`'s entry, in `block` (its projection's, which its numbering is in): written whole on the first ask, the same
    /// entry after.
    pub fn new(block: &Arc<Block>, p: &Projected) -> Self {
        assert!(Arc::ptr_eq(p.block(), block), "an entry is in its projection's block");
        let (desc, filled) = p.entry();
        filled.get_or_init(|| Self::fill(block, p, desc));
        ViewEntry { desc: desc.as_ptr() }
    }

    /// Writes the rest of `p`'s entry `desc` (its key, file and tag are there from the start), and returns what it
    /// made itself.
    fn fill(block: &Arc<Block>, p: &Projected, desc: &Desc<ViewDoc>) -> Filled {
        let f = p.f();
        // A guest whose placement no word maps (an embedder's segments, a string's normalized pieces): each position's
        // span placed as the host places it (`Locs::place`), the table the `Loc` views read.
        let placed = Self::locs(p).and_then(|x| x.host.as_ref()).filter(|(_, place)| !matches!(place, Placement::Copied { .. } | Placement::At(_))).map(|(_, place)| {
            let n = p.locs() as usize;
            // copy: derived (each position's host span, computed once from the guest's placement; no block holds it)
            Cells::build(block, n.max(1), |out| {
                for (l, o) in out.iter_mut().enumerate().take(n) {
                    let [s, e] = place.host(p.span(l as u32));
                    *o = u64::from(s) << 32 | u64::from(e);
                }
                n
            })
            .expect("a batch's block holds its entries")
        });
        let d = desc.as_ptr();
        // SAFETY: the entry is written by this alone (inside its projection's `OnceLock`), and its runs point at cells
        // of the same block (the numbering's descriptor among them), which live as long as it.
        unsafe {
            (*d).preorder = u32::from(p.defs.is_none());
            (*d).nodes = p.len();
            Run::set(&raw mut (*d).numbering, p.ranks().desc().cast::<rerail_clif::ids::Numbering>(), 1);
            // A node's place, read in its tape's auxiliary index (`tape_place`, `tape_place_past_comments`).
            (*d).place = p.place_kind() as u32;
            // The tape's rows' derived columns made by now (`tape_rows::derive`: those its program reads).
            let (columns, named) = p.derived.made();
            let [before, after, visits, digests] = columns;
            for (run, cells) in [(&raw mut (*d).gaps[0], before), (&raw mut (*d).gaps[1], after), (&raw mut (*d).visits, visits), (&raw mut (*d).digests, digests)] {
                if let Some(c) = cells {
                    Run::set(run, c.as_ptr(), c.len());
                }
            }
            if let Some(c) = named {
                Run::set(&raw mut (*d).named, c.as_ptr(), c.len());
            }
            // core.rerail's positions (`Locs`), for the `Loc` views: their places are read in the lines the document's
            // row points at (`ViewRow::lines`: the file's `LineTable`, a guest's host's).
            if let Some(x) = Self::locs(p) {
                (*d).tokens = x.tokens;
                (*d).own_text = u32::from(x.own_text);
                (*d).ends = x.ends;
                (*d).source_len = u32::try_from(f.source.len()).expect("a file within 4 GB");
                // A guest's places are its host's lines, through its placement (decision 0055).
                if let Some((_, place)) = &x.host {
                    match (place, &placed) {
                        (&Placement::Copied { delta, within }, _) => {
                            (*d).placement = rerail_clif::view_reads::PLACED_COPIED;
                            (*d).delta = delta;
                            ((*d).within_lo, (*d).within_hi) = (within[0], within[1]);
                        }
                        (&Placement::At(at), _) => {
                            (*d).placement = rerail_clif::view_reads::PLACED_AT;
                            ((*d).within_lo, (*d).within_hi) = (at[0], at[1]);
                        }
                        (_, Some(table)) => {
                            (*d).placement = rerail_clif::view_reads::PLACED_TABLE;
                            Run::set(&raw mut (*d).placed, table.as_ptr(), table.len());
                        }
                        (_, None) => unreachable!("a placement no word maps has its table"),
                    }
                }
            }
        }
        Filled { _placed: placed }
    }

    /// `p`'s positions where the views' reads answer its `Loc` rows: core.rerail's; a guest's through its host's
    /// lines, by its placement (a copied stretch, a point) or, where no word maps it, by its spans placed
    /// (`ViewDoc::placed`).
    pub fn locs(p: &Projected) -> Option<&super::Locs> {
        p.extra.as_deref()
    }

    /// The entry's address, for the reads written once.
    pub fn desc(&self) -> *const u8 {
        self.desc.cast()
    }
}
