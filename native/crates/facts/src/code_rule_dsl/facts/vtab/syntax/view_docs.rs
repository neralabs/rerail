//! A document's entry for its views' reads (decision 0353 step 4c, `rerail_clif::view_reads::ViewDoc`): in the
//! block its projection's numbering is in, which it refers to, with the derived table of its tape's codes (each
//! code's kind atom and field atom, a few hundred u32s). No per-node column: a node's row is read off the tape.

use super::cells::{Block, Cells, Desc};
use super::{Dialect, Placement, Projected, field_atom};
use rerail_clif::ids::Run;
use rerail_clif::view_reads::ViewDoc;
use std::sync::Arc;

/// A document's entry and its codes' table, in `block`.
pub struct ViewEntry {
    desc: Desc<ViewDoc>,
    _kinds: Cells<u32>,
    _fields: Cells<u32>,
    /// A guest's `Loc` rows' host spans, where its placement is no copy or point (`PLACED_SPANS`).
    _hosted: Option<Cells<u64>>,
}

impl ViewEntry {
    /// `p`'s entry, in `block` (the block its numbering is in), its kinds as `dialect` holds them.
    pub fn new(block: &Arc<Block>, p: &Projected, dialect: &Dialect) -> Self {
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
                *k = field_atom(p.tag, f.code_entry(c as u32).1);
            }
            n
        })
        .expect("a batch's block holds its entries");
        // An embedder's segments and a string's normalized pieces map a span through a search of their own: each `Loc`
        // row's host span is derived here once (`Placement::host`, as the host's `Locs::place` reads it), and the
        // views' reads read it by row.
        let hosted = Self::locs(p).and_then(|x| x.host.as_ref()).filter(|(_, place)| matches!(place, Placement::Segments(_) | Placement::Mapped { .. })).map(|(_, place)| {
            let n = p.locs() as usize;
            // copy: derived (each `Loc` row's host span, computed once from the guest's placement)
            Cells::build(block, n.max(1), |out| {
                for (l, v) in out.iter_mut().enumerate().take(n) {
                    let [s, e] = place.host(p.span(l as u32));
                    *v = u64::from(s) << 32 | u64::from(e);
                }
                n
            })
            .expect("a batch's block holds its entries")
        });
        let desc = Desc::<ViewDoc>::new(block);
        let d = desc.as_ptr();
        // SAFETY: the entry is this build's alone, and its runs point at cells of the same block (the numbering's
        // descriptor among them), which live as long as it.
        unsafe {
            (*d).key = p.key;
            (*d).file = p.file;
            (*d).tag = u32::from(p.tag);
            (*d).preorder = u32::from(p.defs.is_none());
            (*d).nodes = p.len();
            Run::set(&raw mut (*d).numbering, p.ranks().desc().cast::<rerail_clif::ids::Numbering>(), 1);
            Run::set(&raw mut (*d).kinds, kinds.as_ptr(), kinds.len());
            Run::set(&raw mut (*d).fields, fields.as_ptr(), fields.len());
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
                        Placement::Segments(_) | Placement::Mapped { .. } => {
                            let hosted = hosted.as_ref().expect("its spans derived above");
                            (*d).placement = rerail_clif::view_reads::PLACED_SPANS;
                            Run::set(&raw mut (*d).hosted, hosted.as_ptr(), hosted.len());
                        }
                    }
                }
            }
        }
        ViewEntry { desc, _kinds: kinds, _fields: fields, _hosted: hosted }
    }

    /// `p`'s positions, which the views' reads answer its `Loc` rows by: core.rerail's; a guest's through its host's
    /// lines, by its placement (a copied stretch or a point read as it is, a string's normalized pieces and an
    /// embedder's segments by each row's host span, `hosted`).
    pub fn locs(p: &Projected) -> Option<&super::Locs> {
        p.extra.as_deref()
    }

    /// The entry's address, for the reads written once.
    pub fn desc(&self) -> *const u8 {
        self.desc.as_ptr().cast()
    }
}
