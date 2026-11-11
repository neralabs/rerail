//! Persistent case-exact vocabulary and delta-coded value postings, following tape_store.py.
//! SQLite's B-tree keys retain the full term, so hash collisions need no confirmation path.
use crate::code_rule_dsl::tape::TapeDoc;
use rusqlite::{Connection, Result};
use std::collections::BTreeMap;

pub(super) const TERM_CAP: usize = 96;
const RESIDUE_CAP: usize = 4096;
const RESIDUE_BYTES: usize = RESIDUE_CAP * TERM_CAP;
pub(super) const INTERIOR: u8 = 1;
const UNCLEAN: u8 = 2;
const TOO_BIG: u8 = 4;

#[derive(Default)]
pub(super) struct Counts {
    totals: BTreeMap<(u32, u16), usize>,
    files: BTreeMap<i64, (u32, Vec<(u16, usize)>)>,
}
impl Counts {
    pub fn load(db: &Connection) -> Result<Self> {
        let mut counts = Self::default();
        let mut q = db.prepare("SELECT c.fid,f.lang,c.kind,c.residue FROM vcov c JOIN file f USING(fid) WHERE c.residue>0")?;
        for row in q.query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get::<_, u32>(3)? as usize,
            ))
        })? {
            let (fid, lang, kind, n) = row?;
            *counts.totals.entry((lang, kind)).or_default() += n;
            counts
                .files
                .entry(fid)
                .or_insert_with(|| (lang, Vec::new()))
                .1
                .push((kind, n));
        }
        Ok(counts)
    }
    fn prior(&self, lang: u32, kind: u16, previous: Option<i64>) -> usize {
        let old = previous
            .and_then(|fid| self.files.get(&fid))
            .filter(|(l, _)| *l == lang)
            .and_then(|(_, rows)| rows.iter().find(|(k, _)| *k == kind))
            .map_or(0, |(_, n)| *n);
        self.totals
            .get(&(lang, kind))
            .copied()
            .unwrap_or(0)
            .saturating_sub(old)
    }
    pub fn remove(&mut self, fid: i64) {
        if let Some((lang, rows)) = self.files.remove(&fid) {
            for (kind, n) in rows {
                *self
                    .totals
                    .get_mut(&(lang, kind))
                    .expect("published coverage") -= n;
            }
        }
    }
    pub fn publish(&mut self, fid: i64, lang: u32, image: &Image<'_>) {
        let rows: Vec<_> = image
            .cover
            .iter()
            .filter(|(_, (_, n))| *n > 0)
            .map(|(&kind, &(_, n))| (kind, n))
            .collect();
        for &(kind, n) in &rows {
            *self.totals.entry((lang, kind)).or_default() += n;
        }
        self.files.insert(fid, (lang, rows));
    }
}

pub(super) struct Image<'a> {
    postings: BTreeMap<(u16, &'a str), Vec<u32>>,
    cover: BTreeMap<u16, (u8, usize)>,
    missing: BTreeMap<u16, Vec<u32>>,
}
impl<'a> Image<'a> {
    pub fn build(
        doc: &'a TapeDoc,
        lang: u32,
        previous: Option<i64>,
        counts: &Counts,
        interrupted: &impl Fn() -> bool,
    ) -> Result<Self> {
        let mut image = Self {
            postings: BTreeMap::new(),
            cover: BTreeMap::new(),
            missing: BTreeMap::new(),
        };
        let mut residue: BTreeMap<u16, Vec<(u32, &str)>> = BTreeMap::new();
        let mut sizes: BTreeMap<u16, usize> = BTreeMap::new();
        for pre in 0..doc.transducer_len() as u32 {
            if pre.is_multiple_of(256) && interrupted() {
                return Err(rusqlite::Error::SqliteFailure(
                    rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_INTERRUPT),
                    None,
                ));
            }
            let kind = if doc.transducer_flags(pre) & 8 != 0 {
                0
            } else {
                doc.transducer_kind(pre)
            };
            let text = doc.transducer_text_str(pre);
            let clean = !text.is_empty()
                && text.len() <= TERM_CAP * 4
                && text.chars().count() <= TERM_CAP
                && !text.chars().any(|c| c.is_whitespace() || c < '\u{20}');
            let coverage = image.cover.entry(kind).or_default();
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
                image.missing.entry(kind).or_default().push(pre);
            } else if clean {
                image.postings.entry((kind, text)).or_default().push(pre);
            } else {
                coverage.0 |= UNCLEAN;
                image.missing.entry(kind).or_default().push(pre);
                coverage.1 += 1;
                let bytes = sizes.entry(kind).or_default();
                *bytes = bytes.saturating_add(text.len());
                if counts
                    .prior(lang, kind, previous)
                    .saturating_add(coverage.1)
                    > RESIDUE_CAP
                    || *bytes > RESIDUE_BYTES
                {
                    coverage.0 |= TOO_BIG;
                    residue.remove(&kind);
                } else if coverage.0 & TOO_BIG == 0 {
                    residue.entry(kind).or_default().push((pre, text));
                }
            }
        }
        for (kind, nodes) in residue {
            if image.cover[&kind].0 & (INTERIOR | TOO_BIG) == 0 {
                for (pre, text) in nodes {
                    image.postings.entry((kind, text)).or_default().push(pre);
                }
                image.missing.remove(&kind);
            }
        }
        Ok(image)
    }
    pub fn put(&self, db: &Connection, fid: i64, lang: u32) -> Result<()> {
        super::store::publication::insert(
            db,
            "INSERT INTO vidx(lang,term,fid,kind,postings,n)",
            "",
            self.postings.iter().map(|(&(kind, term), pres)| {
                let mut sorted = pres.clone();
                sorted.sort_unstable();
                [
                    i64::from(lang).into(),
                    term.to_owned().into(),
                    fid.into(),
                    i64::from(kind).into(),
                    pack(&sorted).into(),
                    (sorted.len() as i64).into(),
                ]
            }),
        )?;
        super::store::publication::insert(
            db,
            "INSERT INTO vcov",
            "",
            self.cover.iter().map(|(&kind, &(bad, residue))| {
                [
                    fid.into(),
                    i64::from(kind).into(),
                    i64::from(bad).into(),
                    (residue as i64).into(),
                    pack(self.missing.get(&kind).map_or(&[], Vec::as_slice)).into(),
                ]
            }),
        )?;
        Ok(())
    }
}

fn pack(pres: &[u32]) -> Vec<u8> {
    let mut packed = Vec::new();
    let mut previous = 0;
    for &pre in pres {
        let delta = pre - previous;
        previous = pre;
        super::postings::append_delta(&mut packed, delta);
    }
    packed
}
