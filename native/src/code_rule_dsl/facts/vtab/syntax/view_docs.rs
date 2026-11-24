//! A document's entry for its views' reads (decision 0353 step 4c, `rerail_clif::view_reads::ViewDoc`): in the
//! block its projection's numbering is in, which it refers to, with the derived table of its tape's codes (each
//! code's kind atom and field atom, a few hundred u32s). No per-node column: a node's row is read off the tape.

use super::cells::{Block, Cells, Desc};
use super::{Dialect, Placement, Projected, field_atom};
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

/// An entry's codes' table: each code's kind atom, field atom and (where `op` is read) name atom.
pub struct Codes {
    _kinds: Cells<u32>,
    _fields: Cells<u32>,
    _names: Option<Cells<u32>>,
}

impl ViewEntry {
    /// `p`'s entry, in `block` (its projection's, which its numbering is in), its kinds as `dialect` holds them, and
    /// its codes' names interned in `names` where the batch reads `op`: written whole on the first ask, the same entry
    /// after.
    pub fn new(block: &Arc<Block>, p: &Projected, dialect: &Dialect, names: Option<&crate::content_store::atoms::Atoms>) -> Self {
        assert!(Arc::ptr_eq(p.block(), block), "an entry is in its projection's block");
        let (desc, codes) = p.entry();
        codes.get_or_init(|| Self::fill(block, p, dialect, desc, names));
        ViewEntry { desc: desc.as_ptr() }
    }

    /// Writes the rest of `p`'s entry `desc` (its key, file and tag are there from the start), and returns its codes'
    /// table.
    fn fill(block: &Arc<Block>, p: &Projected, dialect: &Dialect, desc: &Desc<ViewDoc>, names: Option<&crate::content_store::atoms::Atoms>) -> Codes {
        let f = &*p.facts;
        let n = f.code_count();
        // copy: derived (each code's kind and field atom, computed once from the tape's code table and the dialect)
        let kinds = Cells::build(block, n.max(1), |out| {
            for (c, k) in out.iter_mut().enumerate().take(n) {
                *k = dialect.kind_of_atom(f.code_entry(c as u32).0);
            }
            n
        })
        .expect("a batch's block holds its entries");
        let fields = Cells::build(block, n.max(1), |out| {
            for (c, k) in out.iter_mut().enumerate().take(n) {
                *k = field_atom(p.tag(), f.code_entry(c as u32).1);
            }
            n
        })
        .expect("a batch's block holds its entries");
        // Each code's name atom: an interned text, the `op` view's `s` of a token of that code (decision 0353 step 3).
        let names = names.map(|atoms| {
            Cells::build(block, n.max(1), |out| {
                for (c, a) in out.iter_mut().enumerate().take(n) {
                    *a = atoms.intern(dialect.atom_name(f.code_entry(c as u32).0)).unwrap_or(0);
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
            Run::set(&raw mut (*d).kinds, kinds.as_ptr(), kinds.len());
            Run::set(&raw mut (*d).fields, fields.as_ptr(), fields.len());
            if let Some(names) = &names {
                Run::set(&raw mut (*d).names, names.as_ptr(), names.len());
            }
            let places = p.places_column();
            Run::set(&raw mut (*d).places, places.as_ptr(), places.len());
            // core.rerail's positions (`Locs`), for the `Loc` views: their places are read in the lines the document's
            // row points at (`ViewRow::lines`: the file's `LineTable`, a guest's host's).
            if let Some(x) = Self::locs(p) {
                (*d).tokens = x.tokens;
                (*d).own_text = u32::from(x.own_text);
                (*d).ends = x.ends;
                (*d).source_len = u32::try_from(f.source.len()).expect("a file within 4 GB");
                // A guest's places are its host's lines, through its placement (decision 0055).
                if let Some((_, place)) = &x.host {
                    match *place {
                        Placement::Copied { delta, within } => {
                            (*d).placement = rerail_clif::view_reads::PLACED_COPIED;
                            (*d).delta = delta;
                            ((*d).within_lo, (*d).within_hi) = (within[0], within[1]);
                        }
                        Placement::At(at) => {
                            (*d).placement = rerail_clif::view_reads::PLACED_AT;
                            ((*d).within_lo, (*d).within_hi) = (at[0], at[1]);
                        }
                        _ => unreachable!("`locs` takes these placements alone"),
                    }
                }
            }
        }
        Codes { _kinds: kinds, _fields: fields, _names: names }
    }

    /// `p`'s positions where the views' reads answer its `Loc` rows: core.rerail's; a guest's through its host's
    /// lines, where its placement is a copied stretch or a point (a string's normalized pieces and an embedder's
    /// segments are read apart from any block: its places are made).
    pub fn locs(p: &Projected) -> Option<&super::Locs> {
        p.extra.as_deref().filter(|x| matches!(&x.host, None | Some((_, Placement::Copied { .. } | Placement::At(_)))))
    }

    /// The entry's address, for the reads written once.
    pub fn desc(&self) -> *const u8 {
        self.desc.cast()
    }
}
