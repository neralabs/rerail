//! Typing programs' answers as a rule's facts (decision 0047, point 2; 03 §13: "their results are facts").
//!
//! A rule reads a typing program's output by declaring an input relation named `typing_<program>__<output>`. In it,
//! each node column of the output becomes four columns: the node's path, kind, start byte and end byte. Other
//! columns are kept as they are.
//!
//! ```text
//! // typing/programs/calls.rr                      // the rule (lib/python/calls.rr)
//! .decl call_target(c: node, f: node)              .decl typing_calls__call_target(c_path: symbol, c_kind: symbol,
//!                                                      c_start: number, c_end: number, f_path: symbol,
//!                                                      f_kind: symbol, f_start: number, f_end: number)
//! ```
//!
//! When a rule that reaches an output reads such a relation, the corpus runner runs the program
//! (`typing/programs/<program>.rr`) first, over the same root and with the same typing inputs, in its own bounded
//! cross-file rounds. Each output row becomes a fact of the relation. The rule joins the spans back to its own nodes
//! (`node(d, n, kind, start, end)`), so the program's facts (T's shape) and the rule's (`core.rr`'s) need not
//! number nodes alike.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::super::{Error, Result};
use super::{Report, Typing};
use crate::code_rule_dsl::logic::front::{self, Literal, Program, Value};
use crate::code_rule_dsl::typing::file_local::Binding;

const PREFIX: &str = "typing_";
const SEP: &str = "__";

/// Whether `rel` is a bridged relation's name (`typing_<program>__<output>`). Such a relation is one fact family
/// shared by every program of a run (`corpus::combine` does not prefix it), as `core.rr`'s are: the program's answers
/// are the same whichever rule reads them.
pub fn is_bridged(rel: &str) -> bool {
    rel.strip_prefix(PREFIX).is_some_and(|rest| rest.split_once(SEP).is_some_and(|(p, o)| !p.is_empty() && !o.is_empty()))
}

/// A bridged relation: the program, its output, and the output's node columns.
struct Bridged {
    rel: String,
    program: String,
    output: String,
}

/// The bridged relations a rule reaching one of `prog`'s outputs reads.
fn wanted(prog: &Program) -> Vec<Bridged> {
    // Only a declared relation named as bridged can be one: a program that declares none is not pruned or read.
    if !prog.decls.keys().any(|rel| rel.starts_with(PREFIX)) {
        return Vec::new();
    }
    let pruned = front::prune(prog.clone());
    let read: BTreeSet<&str> = pruned.rules.iter().flat_map(|r| r.body.iter().filter_map(Literal::atom).map(|a| a.rel.as_str())).collect();
    let derived: BTreeSet<&str> = pruned.rules.iter().map(|r| r.head.rel.as_str()).collect();
    read.into_iter()
        .filter(|rel| rel.starts_with(PREFIX) && prog.decls.contains_key(*rel) && !derived.contains(rel))
        .filter_map(|rel| {
            let (program, output) = rel.strip_prefix(PREFIX)?.split_once(SEP)?;
            Some(Bridged { rel: rel.to_owned(), program: program.to_owned(), output: output.to_owned() })
        })
        .collect()
}

/// The typing program `name`'s text.
fn source(name: &str) -> Result<String> {
    let path = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).join(format!("src/code_rule_dsl/typing/programs/{name}.rr"));
    // Its `#include`s read in (`python/modules.rr`, …).
    crate::code_rule_dsl::logic::settings::program_text(&path).map_err(Error::Plan)
}

/// The rows of the bridged relations, by relation: added to the rule's facts as they are, not printed and parsed.
pub(crate) type Facts = BTreeMap<String, Vec<Vec<Value>>>;

/// What the bridged runs did: each program's report.
pub(crate) type Runs = Vec<(String, Report)>;

/// The facts of the bridged relations `prog` reads, by relation, and each program's run. `scratch` holds the
/// programs' runs (`<scratch>/typing-<program>`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn facts(prog: &Program, root: &Path, scratch: &Path, modes: &str, binding: Binding, chunk_nodes: usize, typing: &Typing) -> Result<(Facts, Runs)> {
    let wanted = wanted(prog);
    let mut by_program: BTreeMap<&str, Vec<&Bridged>> = BTreeMap::new();
    for b in &wanted {
        by_program.entry(b.program.as_str()).or_default().push(b);
    }
    let (mut facts, mut runs) = (Facts::new(), Vec::new());
    for (name, bridged) in by_program {
        let src = source(name)?;
        let program = crate::code_rule_dsl::logic::rr::parse(&src)?;
        // Each output's node columns, checked against the rule's declaration.
        let mut columns: BTreeMap<&str, Vec<bool>> = BTreeMap::new();
        for b in &bridged {
            if !program.outputs.contains(&b.output) {
                return Err(Error::Plan(format!("{}: {name}.rr has no output {}", b.rel, b.output)));
            }
            let nodes: Vec<bool> = program.decls[b.output.as_str()].iter().map(|(_, ty)| ty == "node").collect();
            // Lowered, `symbol` is `sym` and `number` is `int`.
            let want: Vec<&str> = program.decls[b.output.as_str()]
                .iter()
                .flat_map(|(_, ty)| if ty == "node" { vec!["sym", "sym", "int", "int"] } else { vec![ty.as_str()] })
                .collect();
            let have = &prog.decls[b.rel.as_str()];
            let fits = have.len() == want.len() && have.iter().zip(&want).all(|((_, ty), w)| ty == w);
            if !fits {
                return Err(Error::Plan(format!(
                    "{}: {} columns, where {name}.rr's {} gives {} (each node as path, kind, start, end)",
                    b.rel,
                    have.len(),
                    b.output,
                    want.len()
                )));
            }
            columns.insert(b.output.as_str(), nodes);
        }
        // The spans of the nodes the bridged outputs name.
        let mut spans = String::from("\n.decl typing_bridge_at(n: node, k: symbol, s: number, e: number)\n");
        for (output, nodes) in &columns {
            let vars: Vec<String> = (0..nodes.len()).map(|i| format!("typing_v{i}")).collect();
            for (i, _) in nodes.iter().enumerate().filter(|(_, n)| **n) {
                spans.push_str(&format!("typing_bridge_at(typing_v{i}, k, s, e) :- {output}({}), node(_, typing_v{i}, k, s, e).\n", vars.join(", ")));
            }
        }
        spans.push_str(".output typing_bridge_at\n");
        let out = scratch.join(format!("typing-{name}"));
        // The program's language is its name's: `calls_rust` reads the `.rs` files, the others the `.py` files.
        let typing = Typing { rust: Some(name.ends_with("_rust")), ..typing.clone() };
        let report = super::run_typed(&[&format!("{src}\n{spans}")], root, None, &out, modes, binding, chunk_nodes, &typing)?;
        let rows = |rel: &str| -> Result<Vec<Vec<String>>> {
            let text = std::fs::read_to_string(out.join(format!("{rel}.csv"))).map_err(|e| Error::Plan(format!("{name}: {rel}: {e}")))?;
            Ok(text.lines().map(|l| l.split('\t').map(str::to_owned).collect()).collect())
        };
        let mut at: BTreeMap<String, [String; 4]> = BTreeMap::new();
        for r in rows("typing_bridge_at")? {
            let path = r[0].rsplit_once(':').map(|(p, _)| p.to_owned()).ok_or_else(|| Error::Plan(format!("{name}: node {}", r[0])))?;
            at.insert(r[0].clone(), [path, r[1].clone(), r[2].clone(), r[3].clone()]);
        }
        for b in &bridged {
            let nodes = &columns[b.output.as_str()];
            let types: Vec<&str> = program.decls[b.output.as_str()].iter().map(|(_, ty)| ty.as_str()).collect();
            let int = |v: &str| v.parse::<i64>().map(Value::Int).map_err(|_| Error::Plan(format!("{name}: {}: not a number: {v}", b.output)));
            let held = facts.entry(b.rel.clone()).or_default();
            for r in rows(&b.output)? {
                let mut args = Vec::new();
                for ((v, &node), ty) in r.iter().zip(nodes).zip(&types) {
                    if node {
                        let [p, k, s, e] = at.get(v).ok_or_else(|| Error::Plan(format!("{name}: no span for {v}")))?;
                        args.extend([Value::Str(p.clone().into()), Value::Str(k.clone().into()), int(s)?, int(e)?]);
                    } else if *ty == "sym" || *ty == "symbol" {
                        args.push(Value::Str(v.clone().into()));
                    } else {
                        args.push(int(v)?);
                    }
                }
                held.push(args);
            }
        }
        let _ = std::fs::remove_dir_all(&out);
        runs.push((name.to_owned(), report));
    }
    Ok((facts, runs))
}
