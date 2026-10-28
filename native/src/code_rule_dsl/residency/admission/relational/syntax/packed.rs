//! Numeric columns and CSR postings. Host names are dictionaries, never node payloads.
use super::*;
pub(in crate::code_rule_dsl) const NONE: u32 = u32::MAX;
#[derive(Default)]
pub(in crate::code_rule_dsl) struct Columns {
    pub parent: Vec<u32>,
    pub subtree_end: Vec<u32>,
    pub kind: Vec<u32>,
    pub field: Vec<u32>,
    pub start: Vec<u32>,
    pub end: Vec<u32>,
    pub flags: Vec<u32>,
    pub kind_names: Vec<String>,
    pub kind_ids: HashMap<String, u32>,
}
impl Columns {
    pub fn build(doc: &TapeDoc) -> Result<Self> {
        let mut c = Self::default();
        for id in 0..u32::try_from(doc.transducer_len())
            .map_err(|_| invalid("packed document capacity"))?
        {
            if crate::code_rule_dsl::budget::step_budget_exhausted() {
                return Err(invalid("interrupted: packed syntax"));
            }
            let node = doc.node(id);
            let name = node.kind();
            let kind = if let Some(&kind) = c.kind_ids.get(name.as_ref()) {
                kind
            } else {
                let kind = c.kind_names.len() as u32;
                let name = name.into_owned();
                c.kind_names.push(name.clone());
                c.kind_ids.insert(name, kind);
                kind
            };
            let span = doc.transducer_span(id);
            c.parent.push(doc.transducer_parent(id).unwrap_or(NONE));
            c.subtree_end.push(doc.transducer_descendants(id).end);
            c.kind.push(kind);
            c.field.push(u32::from(doc.transducer_field(id)));
            c.start
                .push(u32::try_from(span.start).map_err(|_| invalid("packed byte capacity"))?);
            c.end
                .push(u32::try_from(span.end).map_err(|_| invalid("packed byte capacity"))?);
            c.flags.push(u32::from(doc.transducer_flags(id)));
        }
        Ok(c)
    }
    pub fn parent(&self, id: u32) -> Option<u32> {
        let p = self.parent[id as usize];
        (p != NONE).then_some(p)
    }
    pub fn kind(&self, id: u32) -> &str {
        &self.kind_names[self.kind[id as usize] as usize]
    }
    pub fn span(&self, id: u32) -> std::ops::Range<usize> {
        self.start[id as usize] as usize..self.end[id as usize] as usize
    }
}
#[derive(Default)]
pub(in crate::code_rule_dsl) struct Postings {
    pub offsets: Vec<u32>,
    pub values: Vec<u32>,
}
impl Postings {
    pub fn build(keys: usize, pairs: impl IntoIterator<Item = (u32, usize)>) -> Result<Self> {
        let pairs = pairs.into_iter().collect::<Vec<_>>();
        let mut offsets = vec![
            0_u32;
            keys.checked_add(1)
                .ok_or_else(|| invalid("posting capacity"))?
        ];
        for &(key, _) in &pairs {
            let n = offsets
                .get_mut(key as usize + 1)
                .ok_or_else(|| invalid("posting key"))?;
            *n = n
                .checked_add(1)
                .ok_or_else(|| invalid("posting capacity"))?;
        }
        for k in 1..offsets.len() {
            offsets[k] = offsets[k]
                .checked_add(offsets[k - 1])
                .ok_or_else(|| invalid("posting capacity"))?;
        }
        let mut cursors = offsets.clone();
        let mut values = vec![0; pairs.len()];
        for (key, value) in pairs {
            let at = &mut cursors[key as usize];
            values[*at as usize] = u32::try_from(value).map_err(|_| invalid("posting capacity"))?;
            *at += 1;
        }
        Ok(Self { offsets, values })
    }
    pub fn get(&self, key: u32) -> &[u32] {
        self.offsets
            .get(key as usize..key as usize + 2)
            .map_or(&[], |v| &self.values[v[0] as usize..v[1] as usize])
    }
}
pub(in crate::code_rule_dsl) struct Fields {
    offsets: Vec<u32>,
    codes: Vec<u32>,
    nodes: Vec<u32>,
}
impl Fields {
    pub fn build(c: &Columns) -> Result<Self> {
        let mut links = c
            .field
            .iter()
            .enumerate()
            .filter(|(i, f)| **f != 0 && c.parent[*i] != NONE)
            .map(|(i, &field)| (c.parent[i], field, i as u32))
            .collect::<Vec<_>>();
        links.sort_unstable();
        links.dedup_by_key(|v| (v.0, v.1));
        let postings = Postings::build(
            c.parent.len(),
            links.iter().enumerate().map(|(i, v)| (v.0, i)),
        )?;
        Ok(Self {
            offsets: postings.offsets,
            codes: links.iter().map(|v| v.1).collect(),
            nodes: links.iter().map(|v| v.2).collect(),
        })
    }
    pub fn get(&self, id: u32, field: u16) -> Option<u32> {
        let v = self.offsets.get(id as usize..id as usize + 2)?;
        let start = v[0] as usize;
        let end = v[1] as usize;
        self.codes[start..end]
            .binary_search(&u32::from(field))
            .ok()
            .map(|i| self.nodes[start + i])
    }
}
