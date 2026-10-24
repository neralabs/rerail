//! `FACTS_Y2_DUMP=<file>` (diagnosis, Y2): the rows of selected outputs (`FACTS_Y2_OUTPUTS`, a
//! comma-separated list of output-name suffixes, usually `/stage/<name>` with
//! `FACTS_STAGE_OUTPUTS=1`) per worktree, written as a reference snapshot, per-worktree initial
//! differences against it, and per-event deltas of the edited worktree. Y2's local-query rules
//! read these as their graph and facts until Y1's maintained graph lands. Lines, tab-separated:
//! - `O <i> <output name>`: output `i`;
//! - `R <w> <i> <count> <row>`: the reference snapshot (the first worktree dumped);
//! - `I <w> <i> <diff> <row>`: worktree `w` at its first dump, as a difference from the reference;
//! - `E <seq> <w> <group> <kind>` then `P <path> <old> <new>` per change and `D <i> <diff> <row>`
//!   per changed row of `w`, after the event.
//!
//! Rows are JSON arrays (NULL `null`, INTEGER/REAL numbers, TEXT strings).
use super::lower::{self, Out};
use super::tags::W;
use super::val::{self, Row};
use rerail_facts::sql::value::V;
use std::collections::BTreeMap;
use std::io::Write;

type Snap = Vec<BTreeMap<Row, i32>>;

pub struct Dump {
    out: std::io::BufWriter<std::fs::File>,
    sel: Vec<usize>,
    reference: Option<Snap>,
    prev: BTreeMap<W, Snap>,
}

fn row_json(r: &Row) -> String {
    let vs: Vec<serde_json::Value> = r
        .iter()
        .map(|v| match val::to_v(*v) {
            V::Null => serde_json::Value::Null,
            V::Int(i) => serde_json::json!(i),
            V::Real(f) => serde_json::json!(f),
            V::Text(s) => serde_json::Value::String(s.to_string()),
        })
        .collect();
    serde_json::to_string(&vs).unwrap()
}

/// `a - b` as signed counts, in row order.
fn diff<'a>(a: &'a BTreeMap<Row, i32>, b: &'a BTreeMap<Row, i32>) -> Vec<(&'a Row, i32)> {
    let mut d = Vec::new();
    for (r, c) in a {
        let o = b.get(r).copied().unwrap_or(0);
        if *c != o {
            d.push((r, c - o));
        }
    }
    for (r, c) in b {
        if !a.contains_key(r) {
            d.push((r, -c));
        }
    }
    d
}

impl Dump {
    pub fn from_env(outputs: &[(String, Out)]) -> Option<Dump> {
        let path = std::env::var_os("FACTS_Y2_DUMP")?;
        let wanted: Vec<String> = std::env::var("FACTS_Y2_OUTPUTS").unwrap_or_default().split(',').filter(|s| !s.is_empty()).map(str::to_owned).collect();
        let sel: Vec<usize> = outputs.iter().enumerate().filter(|(_, (n, _))| wanted.iter().any(|w| n.ends_with(w.as_str()))).map(|(i, _)| i).collect();
        let mut out = std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(path).unwrap());
        for (k, &i) in sel.iter().enumerate() {
            writeln!(out, "O\t{k}\t{}", outputs[i].0).unwrap();
        }
        eprintln!("y2 dump: {} outputs selected", sel.len());
        Some(Dump { out, sel, reference: None, prev: BTreeMap::new() })
    }

    fn snap(&self, outputs: &[(String, Out)], w: W) -> Snap {
        self.sel.iter().map(|&i| lower::full(&outputs[i].1, w).unwrap_or_else(|e| panic!("y2 dump {}: {e}", outputs[i].0))).collect()
    }

    /// Worktree `w`'s first dump (once): the reference snapshot, or its difference from it.
    pub fn touch(&mut self, outputs: &[(String, Out)], w: W) {
        if self.prev.contains_key(&w) {
            return;
        }
        let s = self.snap(outputs, w);
        match &self.reference {
            None => {
                for (k, m) in s.iter().enumerate() {
                    for (r, c) in m {
                        writeln!(self.out, "R\t{w}\t{k}\t{c}\t{}", row_json(r)).unwrap();
                    }
                }
                self.reference = Some(s.clone());
            }
            Some(rf) => {
                for (k, (m, b)) in s.iter().zip(rf).enumerate() {
                    for (r, d) in diff(m, b) {
                        writeln!(self.out, "I\t{w}\t{k}\t{d}\t{}", row_json(r)).unwrap();
                    }
                }
            }
        }
        self.prev.insert(w, s);
    }

    /// After an event on `w` (which must have been touched, or be new): its changed rows.
    pub fn event(&mut self, outputs: &[(String, Out)], seq: usize, w: W, group: &str, kind: &str, changes: &[(String, Option<String>, Option<String>)]) {
        if !self.prev.contains_key(&w) {
            // A new worktree: its first dump is after the event that creates it.
            writeln!(self.out, "E\t{seq}\t{w}\t{group}\t{kind}").unwrap();
            self.touch(outputs, w);
            return;
        }
        writeln!(self.out, "E\t{seq}\t{w}\t{group}\t{kind}").unwrap();
        for (p, o, n) in changes {
            writeln!(self.out, "P\t{p}\t{}\t{}", o.as_deref().unwrap_or("-"), n.as_deref().unwrap_or("-")).unwrap();
        }
        let s = self.snap(outputs, w);
        let before = self.prev.get(&w).unwrap();
        for (k, (m, b)) in s.iter().zip(before).enumerate() {
            for (r, d) in diff(m, b) {
                writeln!(self.out, "D\t{k}\t{d}\t{}", row_json(r)).unwrap();
            }
        }
        self.prev.insert(w, s);
    }

    pub fn flush(&mut self) {
        self.out.flush().unwrap();
    }
}
