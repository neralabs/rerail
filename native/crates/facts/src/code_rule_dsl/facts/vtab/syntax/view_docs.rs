//! A document's entry for its views' reads (decision 0353 step 4c, `rerail_clif::view_reads::ViewDoc`): in the
//! block its projection's numbering is in, which it refers to, with the derived table of its tape's codes (each
//! code's kind atom and field atom, a few hundred u32s). No per-node column: a node's row is read off the tape. The
//! tape's rows' derived columns (`tape_rows::Derived`: a gap's or a digest's atom, a child's place, the visit order),
//! where its program reads them, are runs of it too.

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
    /// A string's normalized pieces' spans as its host places them (`ViewDoc::placed`), which no word of the entry maps.
    _placed: Option<Cells<u64>>,
    /// An embedder's program's segments (`ViewDoc::segments`).
    _segments: Option<Cells<u32>>,
}

impl ViewEntry {
    /// `p`'s entry, in `block` (its projection's, which its numbering is in), its kinds as `dialect` holds them, and
    /// its codes' names interned in `names` where the batch reads `op`: written whole on the first ask, the same entry
    /// after. No `dialect`: a document with no grammar (a text read as text only), which has no nodes: no kind is
    /// read, and each reads 0.
    pub fn new(block: &Arc<Block>, p: &Projected, dialect: Option<&Dialect>, names: Option<&dyn crate::content_store::AtomTable>) -> Self {
        assert!(Arc::ptr_eq(p.block(), block), "an entry is in its projection's block");
        let (desc, codes) = p.entry();
        codes.get_or_init(|| Self::fill(block, p, dialect, desc, names));
        ViewEntry { desc: desc.as_ptr() }
    }

    /// Writes the rest of `p`'s entry `desc` (its key, file and tag are there from the start), and returns its codes'
    /// table.
    fn fill(block: &Arc<Block>, p: &Projected, dialect: Option<&Dialect>, desc: &Desc<ViewDoc>, names: Option<&dyn crate::content_store::AtomTable>) -> Codes {
        let f = p.f();
        let n = f.code_count();
        // copy: derived (each code's kind and field atom, computed once from the tape's code table and the dialect)
        let kinds = Cells::build(block, n.max(1), |out| {
            for (c, k) in out.iter_mut().enumerate().take(n) {
                *k = dialect.map_or(0, |d| d.kind_of_atom(f.code_entry(c as u32).0));
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
        // Each code's name atom: an interned text, the `op` view's `s` of an anonymous child of that code (decision
        // 0353 row 7), which the host and the kernels read here alone.
        // A document with no grammar has no nodes, so no names.
        let names = names.zip(dialect).map(|(atoms, dialect)| {
            let named: Vec<&str> = (0..n).map(|c| dialect.atom_name(f.code_entry(c as u32).0)).collect();
            let interned = atoms.intern_all(&named).expect("the store's atoms intern the kinds' names");
            Cells::build(block, n.max(1), |out| {
                // copy: derived (the names' atoms, interned here: the tape holds the codes, not their names' atoms)
                out[..n].copy_from_slice(&interned[..n]);
                n
            })
            .expect("a batch's block holds its entries")
        });
        // A string's normalized pieces, which no word maps: each position's span placed as the host places it
        // (`Locs::place`), the table the `Loc` views read.
        let placed = Self::locs(p).and_then(|x| x.host.as_ref()).filter(|(_, place)| matches!(place, Placement::Mapped { .. })).map(|(_, place)| {
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
        // The numbering's slot entries by bucket (`number_of_by_buckets`): a granule numbering's alone.
        let slot_buckets = (p.defs.is_some() && p.len() > 0).then(|| {
            let num = p.ranks().numbering();
            // SAFETY: the projection's numbering, read as every read of it.
            let n = unsafe { num.slot_buckets_len() };
            // copy: derived (each slot bucket's first entry, computed once from the numbering: about two words a
            // granule's slot, no word a node)
            Cells::build(block, n, |out: &mut [u32]| unsafe { num.slot_buckets(out) }).expect("a batch's block holds its entries")
        });
        // The packed maps (`rerail_clif::packed`, about 1.25 bytes an entry): a numbered node's tape node and id's low
        // bits by number, a node's number by own place, a field row's tape node.
        let pack = |vals: &[u32]| {
            // copy: derived (the numbering's answers, computed once from it, a byte an entry)
            Cells::build(block, rerail_clif::packed::words(vals.len()), |out: &mut [u32]| rerail_clif::packed::build(vals, out).0)
                .expect("a batch's block holds its entries")
        };
        let (mut xs, mut lows) = (vec![0u32; p.len() as usize], vec![0u32; p.len() as usize]);
        for (l, x, low) in super::granules::ids(p.table(), p.ranks(), super::ranks::Item::Node, p.len()) {
            (xs[l as usize], lows[l as usize]) = (x, low);
        }
        let node_x = (p.len() > 0).then(|| pack(&xs));
        // An id's low bits jump where consecutive numbers lie in different granules (a slot is a hash): two clusters a
        // block (`packed::two`).
        let node_low = (p.defs.is_some() && p.len() > 0).then(|| {
            // copy: derived (each node's id's low bits, computed once from the numbering, two bytes an entry)
            Cells::build(block, rerail_clif::packed::two::words(lows.len()), |out: &mut [u32]| rerail_clif::packed::two::build(&lows, out).0)
                .expect("a batch's block holds its entries")
        });
        let by_place = slot_buckets.as_ref().map(|_| {
            let num = p.ranks().numbering();
            // SAFETY: the projection's numbering, read as every read of it.
            let mut places = vec![0u32; unsafe { num.own_places() }];
            unsafe { num.own_numbers(&mut places) };
            pack(&places)
        });
        let field_x = (p.len() > 0).then(|| {
            let num = p.ranks().numbering();
            // SAFETY: the projection's numbering, read as every read of it.
            let fx: Vec<u32> = (0..num.count[2]).map(|r| unsafe { num.select(2, r) }).collect();
            pack(&fx)
        });
        // copy: derived (an embedder's program's segments, as the reads find them: `SegmentMap::words`, written once
        // in the batch's block beside the entry)
        let segments = Self::locs(p).and_then(|x| match &x.host {
            Some((_, Placement::Segments(map))) => Some(
                Cells::build(block, (map.words().count() * 5).max(1), |out| {
                    for (k, w) in map.words().flatten().enumerate() {
                        out[k] = w;
                    }
                    map.words().count() * 5
                })
                .expect("a batch's block holds its entries"),
            ),
            _ => None,
        });
        let d = desc.as_ptr();
        // SAFETY: the entry is written by this alone (inside its projection's `OnceLock`), and its runs point at cells
        // of the same block (the numbering's descriptor among them), which live as long as it.
        unsafe {
            (*d).preorder = if p.tape_ids() { rerail_clif::view_reads::IDS_TAPE } else { u32::from(p.defs.is_none()) };
            (*d).nodes = p.len();
            // Every numbered node's tape parent numbered (its root's too): the root's children walked as any parent's.
            (*d).parented = u32::from(p.parented() == p.len());
            if let Some(t) = &slot_buckets {
                Run::set(&raw mut (*d).slot_buckets, t.as_ptr(), t.len());
            }
            for (run, cells) in [(&raw mut (*d).node_x, &node_x), (&raw mut (*d).node_low, &node_low), (&raw mut (*d).by_place, &by_place), (&raw mut (*d).field_x, &field_x)] {
                if let Some(c) = cells {
                    Run::set(run, c.as_ptr(), c.len());
                }
            }
            Run::set(&raw mut (*d).numbering, p.ranks().desc().cast::<rerail_clif::ids::Numbering>(), 1);
            Run::set(&raw mut (*d).kinds, kinds.as_ptr(), kinds.len());
            Run::set(&raw mut (*d).fields, fields.as_ptr(), fields.len());
            if let Some(names) = &names {
                Run::set(&raw mut (*d).names, names.as_ptr(), names.len());
            }
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
                        (Placement::Segments(_), _) => {
                            (*d).placement = rerail_clif::view_reads::PLACED_SEGMENTS;
                            let words = segments.as_ref().expect("an embedder's segments, laid out above");
                            Run::set(&raw mut (*d).segments, words.as_ptr(), words.len() / 5);
                        }
                        (_, Some(table)) => {
                            (*d).placement = rerail_clif::view_reads::PLACED_TABLE;
                            Run::set(&raw mut (*d).placed, table.as_ptr(), table.len());
                        }
                        (_, None) => unreachable!("a string's normalized pieces have their table"),
                    }
                }
            }
        }
        Codes { _kinds: kinds, _fields: fields, _names: names, _placed: placed, _segments: segments }
    }

    /// `p`'s positions where the views' reads answer its `Loc` rows: core.rerail's; a guest's through its host's
    /// lines, by its placement: a copied stretch, a point, an embedder's segments (`ViewDoc::segments`), or a
    /// string's normalized pieces, by its spans placed (`ViewDoc::placed`).
    pub fn locs(p: &Projected) -> Option<&super::Locs> {
        p.extra.as_deref()
    }

    /// The entry's address, for the reads written once.
    pub fn desc(&self) -> *const u8 {
        self.desc.cast()
    }

    /// The name atom of each of its tape's codes (`ViewDoc::names`, by `FactsIn::tape_code`): empty where the batch
    /// reads no `op`.
    pub fn names(&self) -> &[u32] {
        // SAFETY: the entry, written whole before `new` returned; its run of u32s points into its block, which lives as
        // long as the projection that holds the entry.
        unsafe { (*self.desc).names.as_slice::<u32>() }
    }
}
