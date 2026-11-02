//! Y2: dead code (`unreachable-internal-symbols`) rewritten as local queries over the reference
//! graph and the symbol table (`facts`), with reachability the one maintained property (`dc`).
//!
//! As copied from facts-b, less `exported-families` (`ef`) and Y2's own replay driver
//! (`facts-b y2 --dump`): what stays reads a greplay stage dump (`gen::y2dump`), the oracle tree's
//! tables with the dataflow's own outputs (`dc__findings`), which the dead-code oracle test reads.
pub mod dc;
pub mod facts;

use facts::{parse_row, Counts, Facts, Row, Tab, Val};
use std::collections::BTreeMap;
use std::io::BufRead;
use std::path::Path;

#[derive(Default)]
pub(crate) struct Event {
    pub seq: usize,
    pub w: u32,
    pub group: String,
    pub kind: String,
    pub paths: Vec<String>,
    pub rows: Vec<(usize, i32, Row)>,
    pub creates: bool,
    /// Y1's edge export: (graph ms, pairs added, pairs removed) of the event's step.
    graph: Option<(f64, usize, usize)>,
}

#[derive(Default)]
pub(crate) struct Dump {
    pub outputs: Vec<String>,
    pub tabs: Vec<Option<Tab>>,
    pub reference_w: Option<u32>,
    pub reference: Vec<(usize, i32, Row)>,
    pub init: BTreeMap<u32, Vec<(usize, i32, Row)>>,
    pub events: Vec<Event>,
}

pub(crate) fn read_dump(p: &Path) -> Dump {
    let f = std::io::BufReader::with_capacity(1 << 22, std::fs::File::open(p).unwrap());
    let mut d = Dump::default();
    for line in f.lines() {
        let line = line.unwrap();
        let mut it = line.splitn(5, '\t');
        match it.next().unwrap() {
            "O" => {
                let i: usize = it.next().unwrap().parse().unwrap();
                let name = it.next().unwrap().to_owned();
                assert_eq!(i, d.outputs.len());
                // An output listed twice (two plan nodes, one name): only the first is read.
                let tab = Tab::of_output(&name).filter(|t| !d.tabs.contains(&Some(*t)));
                d.tabs.push(tab);
                d.outputs.push(name);
            }
            t @ ("R" | "I") => {
                let w: u32 = it.next().unwrap().parse().unwrap();
                let k: usize = it.next().unwrap().parse().unwrap();
                let c: i32 = it.next().unwrap().parse().unwrap();
                let r = parse_row(it.next().unwrap());
                if t == "R" {
                    d.reference_w = Some(w);
                    d.reference.push((k, c, r));
                } else {
                    d.init.entry(w).or_default().push((k, c, r));
                    // I rows right after an event of `w` with no changes listed: `w` was created by it.
                    if let Some(e) = d.events.last_mut() {
                        if e.w == w && e.paths.is_empty() && e.rows.is_empty() {
                            e.creates = true;
                        }
                    }
                }
            }
            "E" => {
                let seq = it.next().unwrap().parse().unwrap();
                let w = it.next().unwrap().parse().unwrap();
                let group = it.next().unwrap().to_owned();
                let rest = it.next().unwrap_or("");
                let mut f = rest.split('\t');
                let kind = f.next().unwrap_or("").to_owned();
                let graph = match (f.next(), f.next(), f.next()) {
                    (Some(a), Some(b), Some(c)) => Some((a.parse().unwrap(), b.parse().unwrap(), c.parse().unwrap())),
                    _ => None,
                };
                let creates = graph.is_some() && (group == "new_worktree" || group == "commit_init");
                d.events.push(Event { seq, w, group, kind, creates, graph, ..Default::default() });
            }
            "P" => d.events.last_mut().unwrap().paths.push(it.next().unwrap().to_owned()),
            "D" => {
                let k: usize = it.next().unwrap().parse().unwrap();
                let c: i32 = it.next().unwrap().parse().unwrap();
                let r = parse_row(it.next().unwrap());
                d.events.last_mut().unwrap().rows.push((k, c, r));
            }
            o => panic!("dump line kind {o}"),
        }
    }
    d
}

/// The dataflow's outputs Y2 checks against (by stage name), as multisets.
#[derive(Default, Clone)]
pub(crate) struct Oracle {
    sets: BTreeMap<String, BTreeMap<Row, i32>>,
}

impl Oracle {
    fn update(&mut self, name: &str, r: &Row, c: i32) {
        let m = self.sets.entry(name.to_owned()).or_default();
        let e = m.entry(r.clone()).or_default();
        *e += c;
        if *e == 0 {
            m.remove(r);
        }
    }
    pub(crate) fn rows(&self, name: &str) -> Vec<Vec<Val>> {
        let mut v: Vec<Vec<Val>> = self.sets.get(name).map(|m| m.iter().flat_map(|(r, c)| std::iter::repeat_n(r.to_vec(), *c as usize)).collect()).unwrap_or_default();
        v.sort();
        v
    }
    fn strings(&self, name: &str) -> Vec<String> {
        let mut v: Vec<String> = self.sets.get(name).map(|m| m.keys().map(|r| facts::string(r[0].s())).collect()).unwrap_or_default();
        v.sort();
        v
    }
}

fn stage(name: &str) -> &str {
    name.rsplit_once("/stage/").map(|x| x.1).unwrap_or(name)
}

pub(crate) struct World {
    counts: Counts,
    pub(crate) facts: Facts,
    pub(crate) oracle: Oracle,
}

impl World {
    /// Load a snapshot (every row with its count).
    fn load(&mut self, rows: Vec<(Tab, Row, i32)>) {
        let ch = self.counts.classify(&rows);
        self.counts.apply(&rows);
        self.facts.apply(&ch);
    }
}

/// `read_dump`, and with `edges` Y1's edge export: the reference edges come from Y1 only, the
/// dataflow's go to the oracle side.
pub(crate) fn read_dumps(dump: &Path, edges: Option<&Path>) -> (Dump, Option<Dump>) {
    let mut d = read_dump(dump);
    let y1 = edges.map(|p| {
        let e = read_dump(p);
        for (k, t) in d.tabs.iter_mut().enumerate() {
            if *t == Some(Tab::Refs) {
                *t = None;
                assert!(d.outputs[k].ends_with("views/reference_edges"));
            }
        }
        e
    });
    (d, y1)
}

/// The dump's worktrees, ascending (the reference's among them).
pub(crate) fn worktrees(d: &Dump) -> Vec<u32> {
    let mut w: Vec<u32> = d.reference_w.into_iter().chain(d.init.keys().copied()).collect();
    w.sort();
    w.dedup();
    w
}

/// Worktree `wt`'s tables at its first dump: the reference's rows, then its own difference, the
/// reference edges from Y1's export when given.
pub(crate) fn load_world(d: &Dump, y1: Option<&Dump>, wt: u32) -> World {
    let mut oracle = Oracle::default();
    let mut rows = split(d, &d.reference, &mut oracle);
    if Some(wt) != d.reference_w {
        rows.extend(split(d, d.init.get(&wt).map_or(&[][..], Vec::as_slice), &mut oracle));
    }
    if let Some(e) = y1 {
        let refs = |rs: &[(usize, i32, Row)]| rs.iter().map(|(_, c, r)| (Tab::Refs, r.clone(), *c)).collect::<Vec<_>>();
        rows.extend(refs(&e.reference));
        if Some(wt) != e.reference_w {
            rows.extend(refs(e.init.get(&wt).map_or(&[][..], Vec::as_slice)));
        }
    }
    let mut world = World { counts: Counts::default(), facts: Facts::default(), oracle };
    world.load(rows);
    world
}

/// `load_world`'s tables alone.
pub(crate) fn load_facts(d: &Dump, y1: Option<&Dump>, wt: u32) -> Facts {
    load_world(d, y1, wt).facts
}

fn split(d: &Dump, rows: &[(usize, i32, Row)], oracle: &mut Oracle) -> Vec<(Tab, Row, i32)> {
    let mut out = Vec::new();
    for (k, c, r) in rows {
        match d.tabs[*k] {
            Some(t) => out.push((t, r.clone(), *c)),
            None => oracle.update(stage(&d.outputs[*k]), r, *c),
        }
    }
    out
}

pub(crate) fn diff_rows(a: &[Vec<Val>], b: &[Vec<Val>]) -> (Vec<String>, Vec<String>) {
    let (mut i, mut j) = (0, 0);
    let (mut only_a, mut only_b) = (Vec::new(), Vec::new());
    while i < a.len() || j < b.len() {
        if j == b.len() || (i < a.len() && a[i] < b[j]) {
            only_a.push(facts::show_row(&a[i]));
            i += 1;
        } else if i == a.len() || b[j] < a[i] {
            only_b.push(facts::show_row(&b[j]));
            j += 1;
        } else {
            i += 1;
            j += 1;
        }
    }
    (only_a, only_b)
}
