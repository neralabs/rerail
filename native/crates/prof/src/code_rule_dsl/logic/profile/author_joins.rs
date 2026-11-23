//! Source locations belong to the current invocation. Prepared decisions contain only plan names,
//! so reusing a plan in a different checkout still points to the file the author opened.
use super::State;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;
use std::path::PathBuf;

pub const TOPIC: &str = "authored join plan";
type Sites = BTreeSet<(PathBuf, usize)>;
#[derive(Default)]
pub struct Locations {
    pub definitions: BTreeMap<String, (String, Sites)>,
    pub reads: BTreeMap<(String, String), Sites>,
}

/// The invocation's source locations, bound in the run's context (decision 0442: no state of the process's) while
/// `Sources` is held: every job the run takes reads them, and a run outside reads none.
#[derive(Default)]
struct Held(Locations);

/// Held while the locations are bound: the binding ends when it drops.
pub struct Sources {
    _held: super::Bound,
}

impl Sources {
    pub fn enter(locations: Locations) -> Self {
        Self { _held: super::bind(Held(locations)) }
    }
}

fn with_locations<R>(f: impl FnOnce(&Locations) -> R) -> R {
    super::with_bound::<Held, R>(|b| f(&b.0))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Join {
    pub relation: String,
    pub access: Access,
    pub readers: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Access {
    Requests {
        table: String,
        columns: Vec<String>,
    },
    Factors {
        cache: Option<(String, String, usize)>,
    },
    Full,
}

impl Join {
    pub fn record(&self) {
        super::decision(TOPIC, || {
            serde_json::to_string(self).expect("join decision")
        });
    }
}

/// Cardinality metadata from a completed set, never a count of candidate tuples or a sketch.
/// Several chunks can publish different sets. Keep their range; adding them would double-count.
pub fn observe(table: &str, rows: impl FnOnce() -> usize) {
    if super::on()
        && [
            "rr_pair_cache_",
            "rr_product_demand_",
            "rr_reader_demand_",
            "rr_deferred_pairs_",
        ]
        .iter()
        .any(|p| table.starts_with(p))
    {
        let rows = rows();
        let mut state = super::state();
        let entry = state
            .join_rows
            .entry(table.to_owned())
            .or_insert((rows, rows));
        entry.0 = entry.0.min(rows);
        entry.1 = entry.1.max(rows);
    }
}

fn count(range: &(usize, usize)) -> String {
    if range.0 == range.1 {
        range.0.to_string()
    } else {
        format!("{}–{}", range.0, range.1)
    }
}

fn label(locations: &Locations, rel: &str, read: Option<&str>) -> Option<String> {
    let (name, sites) = locations.definitions.get(rel)?;
    let sites = match read {
        Some(target) => locations.reads.get(&(rel.to_owned(), target.to_owned()))?,
        None => sites,
    };
    Some(format!(
        "{name} ({})",
        sites
            .iter()
            .map(|(p, line)| format!("{}:{line}", p.display()))
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

/// Resolve a refusal against the source files of the current invocation, when known.
pub fn source_name(rel: &str, read: Option<&str>) -> String {
    with_locations(|locations| {
        let read = read
            .and_then(|name| locations.definitions.get(name))
            .map(|(name, _)| name.as_str());
        label(locations, rel, read).unwrap_or_else(|| rel.to_owned())
    })
}

pub fn render(report: &mut String, state: &State) {
    with_locations(|locations| {
        let mut lines = Vec::new();
        for (topic, data) in state.decisions.keys() {
            if topic != TOPIC { continue; }
            let Ok(join) = serde_json::from_str::<Join>(data) else { continue };
            let Some(definition) = label(locations, &join.relation, None) else { continue };
            let authored = &locations.definitions[&join.relation].0;
            let mut line = format!("{definition}\n  ");
            match join.access {
                Access::Requests { table, columns } => {
                    let unit = if columns.len() == 2 { "pairs" } else { "keys" };
                    if let Some(n) = state.join_rows.get(&table) {
                        write!(line, "Readers requested {} distinct {unit} per observed set ({}). ", count(n), columns.join(", ")).unwrap();
                    } else {
                        write!(line, "Readers supply {}. This run recorded no request count. ", columns.join(", ")).unwrap();
                    }
                    line.push_str("The engine restricts this join to those requests.");
                    if columns.len() == 2 {
                        line.push_str(" No rule change needed.");
                    } else {
                        line.push_str(" Each key can match several values of the other column.");
                    }
                }
                Access::Factors { cache } => {
                    match cache {
                        None => line.push_str("Readers join the two factors directly."),
                        Some((guard, pairs, limit)) => {
                            match state.join_rows.get(&guard) {
                                Some((0, 0)) => line.push_str("Readers joined the factors directly."),
                                Some((1, 1)) => line.push_str("Readers used the pair cache."),
                                Some(_) => line.push_str("Some evaluations used the pair cache and others joined the factors directly."),
                                None => line.push_str("The plan chooses between a pair cache and direct factor reads. This run recorded no cache decision."),
                            }
                            if let Some(n) = state.join_rows.get(&pairs) {
                                write!(line, " Observed {} cached pairs per evaluation.", count(n)).unwrap();
                            } else if state.join_rows.get(&guard) == Some(&(0, 0)) {
                                line.push_str(" The empty cache guard allows zero cached pairs.");
                            }
                            write!(line, " Cache limit {limit} pair witnesses.").unwrap();
                        }
                    }
                }
                Access::Full => line.push_str("These readers have no common lookup restriction. The deferred join pass keeps the relation in its original form."),
            }
            if let Some(attribute) = join.placement {
                write!(line, " The planner disabled shared caches for this plan to keep {attribute} in its source document.").unwrap();
            }
            for (rel, reason) in join.readers {
                if let Some(reader) = label(locations, &rel, Some(authored)) {
                    write!(line, "\n  Reader {reader} {reason}.").unwrap();
                }
            }
            lines.push(line);
        }
        if !lines.is_empty() {
            lines.sort();
            lines.dedup();
            writeln!(report, "\n== Joins in authored rules\n{}", lines.join("\n")).unwrap();
        }
    });
}
