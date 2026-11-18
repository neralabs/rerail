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
fn wanted(prog: &Program, pruned: &dyn Fn() -> Result<super::shape::Shape>) -> Result<Vec<Bridged>> {
    // Only a declared relation named as bridged can be one: a program that declares none is not pruned or read.
    if !prog.decls.keys().any(|rel| rel.starts_with(PREFIX)) {
        return Ok(Vec::new());
    }
    let pruned = pruned()?;
    Ok(pruned
        .read
        .iter()
        .map(String::as_str)
        .filter(|rel| rel.starts_with(PREFIX) && prog.decls.contains_key(*rel) && !pruned.made.contains(*rel))
        .filter_map(|rel| {
            let (program, output) = rel.strip_prefix(PREFIX)?.split_once(SEP)?;
            Some(Bridged { rel: rel.to_owned(), program: program.to_owned(), output: output.to_owned() })
        })
        .collect())
}

/// The typing program `name`'s text.
fn source(name: &str) -> Result<String> {
    let path = rerail_store::source_tree::native().join(format!("src/code_rule_dsl/typing/programs/{name}.rr"));
    // Its `#include`s read in (`python/modules.rr`, …).
    crate::code_rule_dsl::logic::settings::program_text(&path).map_err(Error::Plan)
}

/// The rows of the bridged relations, by relation: added to the rule's facts as they are, not printed and parsed.
pub(crate) type Facts = BTreeMap<String, Vec<Vec<Value>>>;

/// What the bridged runs did: each program's report.
pub(crate) type Runs = Vec<(String, Report)>;

/// `typing_bridge_at(n, k, s, e)`: each node a bridged output names (`columns`: an output's node columns), with its
/// kind and span, read from `node`. Declared, made and output as a text of them would be once read.
fn add_spans(prog: &mut Program, columns: &BTreeMap<&str, Vec<bool>>) {
    use crate::code_rule_dsl::logic::front::{Atom, Rule, Term};
    const AT: &str = "typing_bridge_at";
    let v = |n: &str| Term::Var(n.to_owned().into());
    let atom = |rel: &str, args: Vec<Term>| Atom { rel: rel.to_owned().into(), args: args.into(), neg: false, gated: false };
    prog.decls.insert(AT.to_owned().into(), [("n", "node"), ("k", "sym"), ("s", "int"), ("e", "int")].iter().map(|(c, t)| ((*c).to_owned().into(), (*t).to_owned().into())).collect::<Vec<_>>().into());
    for (output, nodes) in columns {
        let vars: Vec<Term> = (0..nodes.len()).map(|i| v(&format!("typing_v{i}"))).collect();
        for (i, _) in nodes.iter().enumerate().filter(|(_, n)| **n) {
            let n = v(&format!("typing_v{i}"));
            prog.rules.push(Rule {
                head: atom(AT, vec![n.clone(), v("k"), v("s"), v("e")]),
                body: vec![Literal::Atom(atom(output, vars.clone())), Literal::Atom(atom("node", vec![Term::Wild, n, v("k"), v("s"), v("e")]))].into(),
            });
        }
    }
    crate::code_rule_dsl::logic::rr::lower::base_relation(prog, "node");
    if !prog.outputs.iter().any(|o| o == AT) {
        prog.outputs.push(AT.to_owned());
    }
}

/// The facts of the bridged relations `prog` reads, by relation, and each program's run. `scratch` holds the
/// programs' runs (`<scratch>/typing-<program>`).
#[allow(clippy::too_many_arguments)]
/// `prog` is the program's header (`Version::header`); `pruned`, the shape of its rules once pruned (`front::prune`),
/// asked only of a program that declares a bridged relation.
pub(crate) fn facts(prog: &Program, pruned: &dyn Fn() -> Result<super::shape::Shape>, root: &Path, scratch: &Path, modes: &str, binding: Binding, chunk_nodes: usize, typing: &Typing) -> Result<(Facts, Runs)> {
    let wanted = wanted(prog, pruned)?;
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
        // The spans of the nodes the bridged outputs name: `typing_bridge_at(n, k, s, e)` for each node column of
        // each, rules added to the program read (decision 0249), never text after its text.
        let mut handed = program.clone();
        add_spans(&mut handed, &columns);
        let out = scratch.join(format!("typing-{name}"));
        // The program's language is its name's: `calls_rust` reads the `.rs` files, the others the `.py` files.
        let typing = Typing { rust: Some(name.ends_with("_rust")), ..typing.clone() };
        let report = super::run_read(&handed, root, &out, modes, binding, chunk_nodes, &typing, &serde_json::Value::Null)?;
        // The outputs' image, read where it is: a node by its document's path and value, nothing parsed.
        use crate::code_rule_dsl::logic::plan::output;
        let outputs = output::Outputs::open(&out).map_err(|e| Error::Plan(format!("{name}: {e}")))?;
        let rel = |r: &str| outputs.get(r).ok_or_else(|| Error::Plan(format!("{name}: {r}: not in its outputs")));
        let node_key = |v: output::Value<'_>| match v {
            output::Value::Node(output::Node::Of { path: Some(p), value, .. }) => Ok((value, p.to_owned())),
            v => Err(Error::Plan(format!("{name}: node {v:?}: no document's path"))),
        };
        let int = |v: output::Value<'_>, what: &str| v.int().map(Value::Int).ok_or_else(|| Error::Plan(format!("{name}: {what}: not a number: {v:?}")));
        let text = |v: output::Value<'_>, what: &str| v.text().map(|t| Value::Str(t.to_owned().into())).ok_or_else(|| Error::Plan(format!("{name}: {what}: not a text: {v:?}")));
        let mut at: BTreeMap<i64, [Value; 4]> = BTreeMap::new();
        for r in rel("typing_bridge_at")?.rows() {
            let (key, path) = node_key(r.get(0))?;
            at.insert(key, [Value::Str(path.into()), text(r.get(1), "typing_bridge_at")?, int(r.get(2), "typing_bridge_at")?, int(r.get(3), "typing_bridge_at")?]);
        }
        for b in &bridged {
            let nodes = &columns[b.output.as_str()];
            let types: Vec<&str> = program.decls[b.output.as_str()].iter().map(|(_, ty)| ty.as_str()).collect();
            let held = facts.entry(b.rel.clone()).or_default();
            for r in rel(&b.output)?.rows() {
                let mut args = Vec::new();
                for ((v, &node), ty) in r.values().zip(nodes).zip(&types) {
                    if node {
                        let (key, _) = node_key(v)?;
                        let span = at.get(&key).ok_or_else(|| Error::Plan(format!("{name}: no span for {v:?}")))?;
                        args.extend(span.iter().cloned());
                    } else if *ty == "sym" || *ty == "symbol" {
                        args.push(text(v, &b.output)?);
                    } else {
                        args.push(int(v, &b.output)?);
                    }
                }
                held.push(args);
            }
        }
        drop(outputs);
        let _ = std::fs::remove_dir_all(&out);
        runs.push((name.to_owned(), report));
    }
    Ok((facts, runs))
}
