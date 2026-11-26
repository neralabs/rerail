//! A document's index (decision 0656 step 4): what the engine asks of a tape that the tape alone does not answer
//! cheaply, behind one interface. The engine calls `DocIndex` and nothing under it, so the design that answers can be
//! changed here alone: today every answer is the tape's auxiliary index (`aux_index`), read in place in the tape's home,
//! which every tape's writer lays there.
//!
//! The questions:
//! - **counts**: which codes the document holds and how many nodes of each, and whether its named nodes all hang from
//!   named parents (where they do, a projection of the named nodes numbers every one: its counts are these);
//! - **rare codes**: the nodes of a code that few nodes have, listed, so finding them is no scan of the tape;
//! - **text**: whether a node or span of the document can have a given text (a read of `text` by its string reads only
//!   the documents that can);
//! - **places**: a named node's place among its named siblings (or among those that are no comment, T's shape's), and
//!   a parent's i-th named child;
//! - **terms**: a leaf's text as a number of the document's (equal texts, equal terms), so a reader turns each distinct
//!   text into its own value once, not once a leaf.

use crate::aux_index::{self, AuxIndex, Kind, NONE};
use crate::tape::Tape;

/// The sections the reads below need: an index that lacks one is refused (`DocIndex::read`).
const NEEDS: [Kind; 8] = [Kind::Leaves, Kind::Names, Kind::Vocab, Kind::Siblings, Kind::Counts, Kind::Rare, Kind::Named, Kind::Comments];

/// A document's index, read where its bytes lie.
#[derive(Clone, Copy)]
pub struct DocIndex<'a> {
    aux: AuxIndex<'a>,
}

impl<'a> DocIndex<'a> {
    /// The index whose bytes are `b` (a tape home's auxiliary index), with every section its reads need.
    pub fn read(b: &'a [u8]) -> Result<Self, String> {
        let aux = AuxIndex::read(b)?;
        match NEEDS.iter().find(|&&k| !aux.has(k)) {
            Some(k) => Err(format!("an index without section {}", *k as u16)),
            None => Ok(DocIndex { aux }),
        }
    }

    /// The nodes the document has.
    pub fn nodes(&self) -> u32 {
        self.aux.nodes()
    }

    // Counts.

    /// The nodes of code `code` (0 where it has none).
    pub fn count(&self, code: u32) -> u32 {
        self.aux.count(code)
    }
    /// Whether the document holds a node of code `code`.
    pub fn holds(&self, code: u32) -> bool {
        self.count(code) > 0
    }
    /// The codes the document holds, ascending, each with its count.
    pub fn counts(&self) -> impl Iterator<Item = (u32, u32)> + 'a {
        let aux = self.aux;
        (0..aux.distinct()).map(move |k| aux.count_at(k))
    }
    /// How many codes the document holds.
    pub fn distinct(&self) -> u32 {
        self.aux.distinct()
    }
    /// Whether code `code` is named, as the index's language says.
    pub fn named(&self, code: u32) -> bool {
        self.aux.named(code)
    }
    /// Whether every named node but the root hangs from a named parent: then a walk that numbers the named nodes
    /// from the root numbers each, and the named codes' counts are its.
    pub fn named_all_rooted(&self) -> bool {
        self.aux.strays() == 0
    }

    // Rare codes.

    /// The nodes of code `code`, in preorder, where the index lists them (a code few nodes have; one the document
    /// lacks, with none): `None` where finding them is a scan.
    pub fn listed(&self, code: u32) -> Option<impl Iterator<Item = u32> + 'a> {
        let aux = self.aux;
        aux.rare_range(code).map(move |(a, b)| (a..b).map(move |k| aux.rare_node(k)))
    }

    // Text.

    /// Whether a node or span of the document can have text `text`: a name the document holds, or else text its
    /// source holds. `false` means none can; `true` that one may.
    pub fn may_have_text(&self, text: &[u8], source: &[u8]) -> bool {
        self.aux.term_of_text(text, source) != NONE || memchr::memmem::find(source, text).is_some()
    }

    // Terms.

    /// The document's distinct leaf texts: a term is below this.
    pub fn terms(&self) -> u32 {
        self.aux.terms()
    }
    /// Leaf `node`'s term, the same for every leaf of the same text; `None` for a node that is no leaf.
    pub fn term(&self, node: u32) -> Option<u32> {
        Some(self.aux.term_of_node(node)).filter(|&t| t != NONE)
    }

    // Places.

    /// Named node `node`'s place among its parent's named children (0 for the first, and for the root).
    pub fn place<T: Tape + ?Sized>(&self, t: &T, node: u32) -> u32 {
        aux_index::place(t, &self.aux, node)
    }
    /// Named node `node`'s place among its parent's named children that are no comment (0 for the first, and for the
    /// root): a place as T's shape numbers its nodes, which leaves comments out.
    pub fn place_past_comments<T: Tape + ?Sized>(&self, t: &T, node: u32) -> u32 {
        aux_index::place_past_comments(t, &self.aux, node)
    }
    /// `parent`'s named child `i`, or `None`.
    pub fn child<T: Tape + ?Sized>(&self, t: &T, parent: u32, i: u32) -> Option<u32> {
        aux_index::child(t, &self.aux, parent, i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aux_index::Sections;
    use crate::aux_index::sample;

    /// The facade answers as the document says, from an index with the sections it needs and from one with every
    /// section; one without a section it needs is refused.
    #[test]
    fn a_documents_index_answers_as_the_document_says() {
        let d = sample::Doc::new();
        let named = sample::named();
        let built = aux_index::build(&d, &d.source, 0, &sample::codes(), Sections::of(&NEEDS)).unwrap();
        let full = aux_index::build(&d, &d.source, 0, &sample::codes(), Sections::ALL).unwrap();
        let partial = aux_index::build(&d, &d.source, 0, &sample::codes(), Sections::of(&[Kind::Counts])).unwrap();
        assert!(DocIndex::read(&partial).is_err());
        let is_named = |c: u32| (named[(c / 64) as usize] >> (c % 64)) & 1 != 0;
        for b in [&built, &full] {
            let x = DocIndex::read(b).unwrap();
            let mut by = std::collections::BTreeMap::<u32, Vec<u32>>::new();
            (0..d.len() as u32).for_each(|i| by.entry(d.code(i as usize)).or_default().push(i));
            assert_eq!(x.counts().collect::<Vec<_>>(), by.iter().map(|(c, v)| (*c, v.len() as u32)).collect::<Vec<_>>());
            assert_eq!(x.distinct() as usize, by.len());
            let mut listed = 0;
            for c in 0..70 {
                assert_eq!(x.holds(c), by.contains_key(&c));
                if let Some(nodes) = x.listed(c) {
                    assert_eq!(nodes.collect::<Vec<_>>(), by.get(&c).cloned().unwrap_or_default(), "code {c}");
                    listed += 1;
                }
            }
            assert!(listed > 0);
            assert!(!x.named_all_rooted(), "the sample has named nodes under unnamed ones");
            // Text: every leaf's, and text across leaves, and text the document lacks.
            for i in [3usize, 40, 200] {
                let t = &d.source[d.start(i) as usize..d.end(i) as usize];
                assert!(x.may_have_text(t, &d.source));
            }
            let across = &d.source[d.start(5) as usize..d.end(7) as usize];
            assert!(x.may_have_text(across, &d.source));
            assert!(!x.may_have_text(b"no such text", &d.source));
            // Terms: a leaf's is its text's, the same for equal texts; no other node has one.
            let mut texts = std::collections::HashMap::<&[u8], u32>::new();
            for i in 0..d.len() {
                let leaf = crate::tape::children(&d, i).next().is_none();
                match x.term(i as u32) {
                    Some(t) => {
                        assert!(leaf && t < x.terms());
                        assert_eq!(*texts.entry(&d.source[d.start(i) as usize..d.end(i) as usize]).or_insert(t), t, "node {i}");
                    }
                    None => assert!(!leaf, "leaf {i}"),
                }
            }
            assert_eq!(texts.len() as u32, x.terms());
            // Places among named siblings.
            for p in 0..d.len() as u32 {
                let mut k = 0;
                for c in crate::tape::children(&d, p as usize) {
                    if is_named(d.code(c)) {
                        assert_eq!((x.place(&d, c as u32), x.child(&d, p, k)), (k, Some(c as u32)));
                        k += 1;
                    }
                }
                assert_eq!(x.child(&d, p, k), None);
            }
        }
    }
}
