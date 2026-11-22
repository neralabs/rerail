//! Phase 0's cutoff for a whole run (t23 §4): a run whose every read is as it was writes the outputs it wrote.
//!
//! A run consults a known set of things, and nothing else:
//! - its programs' text, with the root's settings;
//! - the library files that text includes;
//! - the workspace's documents;
//! - what its resolution rules listed, read, loaded, or found missing (`world::World::consulted`).
//!
//! After a run, its record keeps all of these, each with its digest, under a key of the text, the options, the
//! root, the modes and the engine, together with the outputs it wrote. The next run under the same key asks phase
//! 0's observer about each consulted thing. That costs a `stat` each, and a read only for what changed. When all
//! are as they were, the run writes the kept outputs and evaluates nothing: the empty delta stops before phase 1.
//!
//! The cutoff is on with `options.reuse`: the command line's runs, and phase 0's. A run that reads something it cannot name does not record. That covers typing inputs,
//! T facts, a program reading another phase's exports, or a run that consulted no file.

use super::{Report, Result, Typing, observe, unchanged};
use super::Error;
use sha2::{Digest as _, Sha256};
use std::path::Path;

/// Whether the cutoff applies to a run with these options and typing inputs.
pub fn wanted(options: &serde_json::Value, typing: &Typing, t_facts: Option<&Path>) -> bool {
    let asked = options.get("reuse").and_then(serde_json::Value::as_bool).unwrap_or(false);
    // A run asked for a debugging dump runs: kept outputs carry none. A profile does not turn the cutoff off: a
    // profiled warm run is measured as it runs, and its profile says the cutoff applied.
    let side_output = std::env::vars_os().any(|(k, _)| k.to_str().is_some_and(|k| k.starts_with("RERAIL_DUMP_")));
    let wanted = asked && !side_output && t_facts.is_none() && typing.declarations.is_empty() && typing.site.is_none() && typing.summarize.is_none() && super::tape_cache_path().is_some();
    if !wanted {
        miss(&format!("not wanted (asked {asked}, dump {side_output}, t facts {}, typing {}, tape cache {})", t_facts.is_some(), !typing.declarations.is_empty() || typing.site.is_some() || typing.summarize.is_some(), super::tape_cache_path().is_some()));
    }
    wanted
}

/// The run's key: everything it is a function of besides what it consults.
/// Switches that change how a run is measured or held, never what it outputs: left out of the key. A profile's path
/// differs every run, so with it in the key no profiled run ever found the last run's record.
const NOT_OUTPUTS: &[&str] = &["RERAIL_TAPE_CACHE", "RERAIL_PROFILE", "RERAIL_REUSE_TRACE", "RERAIL_DEADLINE", "RERAIL_SQL_STRICT", "RERAIL_WARM", "RERAIL_WRITE_QUEUE_MB", "RERAIL_WRITES", "RERAIL_STATS_KEEP_TRACE", "RERAIL_JOURNAL_TRACE", "RERAIL_UPKEEP", "RERAIL_UPKEEP_CPU", "RERAIL_UPKEEP_MB", "RERAIL_UPKEEP_IDLE", "RERAIL_UPKEEP_BATTERY", "RERAIL_UPKEEP_JOBS", "RERAIL_UPKEEP_LOG", "RERAIL_JOINS", "RERAIL_JOINS_LOG", "RERAIL_JOIN_SLOW", "RERAIL_JOIN_SLOW_ROWS", "RERAIL_JOIN_CRITICAL", "RERAIL_JOIN_EXPLORE", "RERAIL_JOIN_MARGIN", "RERAIL_JOIN_SEED", "RERAIL_OBSERVE_TRACE",
    // A profile's and the heap sampler's (`profile::memory`, `rerail_prof`): a measurement's cold runs sample the heap
    // and its warm runs do not, so with these in the key no warm run found the cold run's record (decision 0376).
    "RERAIL_PROF", "RERAIL_HEAP_SAMPLE", "RERAIL_HEAP_AT", "RERAIL_HEAP_FRAMES", "RERAIL_PROF_MEMORY_MS", "RERAIL_TIMELINE",
    // The width census (`profile::memory::width_census`): it counts the bytes of the columns a run releases.
    "RERAIL_WIDTH_CENSUS",
    // The stores a measurement keeps apart (the cost model's, the rule images'): a plan's costs and a program's stored
    // form, never the run's outputs.
    "RERAIL_COSTS", "RERAIL_IMAGE_STORE", "RERAIL_EXPORT_IMAGES",
    "RERAIL_FRAMES_TRACE",
    // The memory ceiling (decision 0462): a run past it is stopped and keeps nothing; one under it outputs the same.
    "RERAIL_MEMORY_MAX_MB"];

/// Whether a switch only measures a run, never changes what it outputs: left out of the run's key.
pub fn measures_only(name: &str) -> bool {
    NOT_OUTPUTS.contains(&name)
}

/// Every engine switch that can change what a run outputs (`RERAIL_*` less `NOT_OUTPUTS`), in name order, as bytes for
/// the run's key.
fn switches() -> Vec<u8> {
    // The process's and those the run's context sets for its runs (`profile::vars`, a test's `switches::set`), by name.
    let switches: Vec<(String, String)> = crate::code_rule_dsl::logic::profile::vars().into_iter().filter(|(k, _)| !measures_only(k)).collect();
    let mut out = Vec::new();
    for (k, v) in switches {
        out.extend_from_slice(k.as_bytes());
        out.push(0);
        out.extend_from_slice(v.as_bytes());
        out.push(0);
    }
    out
}

fn key(given: &super::Texts<'_>, added: &super::added::Added, root: &Path, options: &serde_json::Value, modes: &str, typing: &Typing) -> String {
    let mut h = Sha256::new();
    // v4: the run's settings keyed as the rows it adds (decision 0249), the texts as the files'.
    h.update(b"rerail.run-reuse/v4\0");
    for p in given.programs {
        h.update((p.len() as u64).to_le_bytes());
        h.update(p.as_bytes());
    }
    // A handed program's plan key omits message text. These outputs contain rendered messages.
    if let Some(read) = given.read {
        for (_, template) in crate::code_rule_dsl::logic::program_image::fields::ProgramFields::settings(&**read).iter().filter(|(key, _)| key == crate::code_rule_dsl::logic::rr::lower::findings::MESSAGE) {
            h.update(b"rendered-message\0");
            h.update((template.len() as u64).to_le_bytes());
            h.update(template.as_bytes());
        }
    }
    added.hash(&mut h);
    h.update(options.to_string().as_bytes());
    h.update([0]);
    h.update(crate::git_workspace::snapshot::canonicalize(root).unwrap_or_else(|_| root.to_path_buf()).to_string_lossy().as_bytes());
    h.update([0]);
    h.update(modes.as_bytes());
    h.update(format!("\0{typing:?}\0").as_bytes());
    // A phase's run is a function of the facts handed down to it (its files, for phase 1), which no consulted file
    // names: the same program over other files is another run.
    h.update(super::phases::handed_down_text().as_bytes());
    h.update([0]);
    h.update(crate::code_rule_dsl::build_ids::engine().as_bytes());
    h.update([u8::from(crate::code_rule_dsl::facts::vtab::syntax::definition_ids())]);
    // What can change the outputs from outside the text: every engine switch (a sample of the corpus, decision 0091's
    // guards, the matcher and planner switches). Listing only some reused a run made under others: a run with
    // `RERAIL_AST_PARTS=off` answered with the outputs of one with it on. The cache's own path names the store.
    h.update(switches());
    format!("{CUTOFF}{}", h.finalize().iter().map(|b| format!("{b:02x}")).collect::<String>())
}

/// The key's parts, each as a short digest, and the options (`RERAIL_REUSE_TRACE`): which part made a run miss.
fn key_parts(programs: &[&str], root: &Path, options: &serde_json::Value, typing: &Typing) -> String {
    if std::env::var_os("RERAIL_REUSE_TRACE").is_none() {
        return String::new();
    }
    let short = |b: &[u8]| Sha256::digest(b).iter().take(3).map(|b| format!("{b:02x}")).collect::<String>();
    let texts: Vec<u8> = programs.iter().flat_map(|p| p.bytes()).collect();
    format!(
        "texts {}, root {} ({}), typing {}, handed down {}, switches {}, options {}",
        short(&texts),
        short(root.to_string_lossy().as_bytes()),
        root.display(),
        short(format!("{typing:?}").as_bytes()),
        short(super::phases::handed_down_text().as_bytes()),
        short(&switches()),
        options.to_string().chars().take(300).collect::<String>()
    )
}

/// The prefix of a whole-run cutoff's record (the observer's `run_record`). Its consulted lines name the plans the
/// run used (`plan`, digest in hex): the observer's store is collected with the content store's, and a record no
/// live plan of which it names is dropped (`observe::collect`).
pub const CUTOFF: &str = "cutoff:";

/// The record's entry of the texts' include graphs (`Texts::graphs`), each one's digest or none: with every rule file
/// as recorded, the same graphs list the same rule files, so the check need not list them again (decision 0275).
const GRAPHS: &str = "graphs";

/// The texts' include graphs as the record's entry holds them.
fn graphs_line(graphs: &[Option<[u8; 32]>]) -> String {
    let mut h = Sha256::new();
    for g in graphs {
        match g {
            Some(d) => {
                h.update([1]);
                h.update(d);
            }
            None => h.update([0]),
        }
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// The record's entry of the program's parts the check reads (`corpus::Summary`), as JSON.
const SUMMARY: &str = "program";

/// The root's settings (`<root>/rerail.env`, the `env` option), as the rows the run adds (decision 0249).
fn settings(root: &Path, options: &serde_json::Value) -> Result<super::added::Added> {
    let flags: Vec<String> = options.get("env").and_then(|v| v.as_array()).into_iter().flatten().filter_map(|v| v.as_str().map(str::to_owned)).collect();
    let settings = crate::code_rule_dsl::logic::settings::settings(root, &flags).map_err(Error::Plan)?;
    Ok(super::added::Added::of(&settings, options))
}

/// The library files the programs include, each with its digest now: the record's `rule` entries.
///
/// Read off phase 0's graphs (decision 0246): each text's includes (`Graph::digests`) and the phase programs its
/// declarations name, with theirs (`Graph::rules`), which `bootstrap.rerail` resolved as it loaded the modules. Nothing is
/// parsed: listing them by reading every module again was 4.3 G of a one-file run, the parse the shipped bundle saves.
/// A text phase 0 has no graph of (it includes nothing, or the run was handed it read), or a `phase_dir` the graph
/// does not search, lists them as before (`listed`). `RERAIL_REUSE_CHECK` lists both and says where they differ.
pub fn rule_files(given: &super::Texts<'_>, options: &serde_json::Value) -> Result<Vec<(String, String, String)>> {
    let dir = options.get("phase_dir").and_then(|v| v.as_str()).map(std::path::PathBuf::from);
    let (paths, missed) = match graph_files(given, dir.as_deref()) {
        Some((paths, missed)) => {
            if std::env::var_os("RERAIL_REUSE_CHECK").is_some() {
                let (old, old_missed) = listed(given, dir.as_deref())?;
                let old: std::collections::BTreeSet<_> = old.into_iter().collect();
                let new: std::collections::BTreeSet<_> = paths.iter().cloned().collect();
                eprintln!("RERAIL_REUSE_CHECK rule files: {} from the graph, {} listed; only the graph's {:?}; only listed {:?}", new.len(), old.len(), new.difference(&old).collect::<Vec<_>>(), old.difference(&new).collect::<Vec<_>>());
                let old_missed: std::collections::BTreeSet<_> = old_missed.into_iter().collect();
                let new_missed: std::collections::BTreeSet<_> = missed.iter().cloned().collect();
                eprintln!("RERAIL_REUSE_CHECK missing phase programs: {} from the graph, {} listed; only the graph's {:?}; only listed {:?}", new_missed.len(), old_missed.len(), new_missed.difference(&old_missed).collect::<Vec<_>>(), old_missed.difference(&new_missed).collect::<Vec<_>>());
            }
            (paths, missed)
        }
        None => listed(given, dir.as_deref())?,
    };
    let mut out = Vec::new();
    // Phase 1's program (`sources.rerail`), which every run's documents come from: its includes are core's, as the
    // programs' are.
    let sources = super::phases::sources_path();
    let digest = observe(|o| o.digest(&sources)).map(|d| crate::content_store::observe::hex(&d)).unwrap_or_default();
    out.push(("rule".to_owned(), sources.to_string_lossy().into_owned(), digest));
    for path in paths {
        let digest = observe(|o| o.digest(&path)).map(|d| crate::content_store::observe::hex(&d)).unwrap_or_default();
        out.push(("rule".to_owned(), path.to_string_lossy().into_owned(), digest));
    }
    // The places a phase program was looked for and not found: no digest, so one appearing there differs.
    out.extend(missed.into_iter().map(|path| ("rule".to_owned(), path.to_string_lossy().into_owned(), String::new())));
    out.sort();
    out.dedup();
    Ok(out)
}

/// The programs' rule files from phase 0's graphs, where every text has one and no `phase_dir` shadows the phase
/// programs' directory the graphs were resolved in; with the phase programs' candidates that were not there
/// (`Graph::absent`), which a run checks are still missing (decision 0275), as `listed`'s.
#[allow(clippy::type_complexity)]
fn graph_files(given: &super::Texts<'_>, dir: Option<&Path>) -> Option<(Vec<std::path::PathBuf>, Vec<std::path::PathBuf>)> {
    if dir.is_some() || given.graphs.is_empty() || given.graphs.iter().any(Option::is_none) || given.derived.len() != given.graphs.len() {
        return None;
    }
    let mut paths: Vec<std::path::PathBuf> = given.derived.iter().flat_map(|g| g.digests.keys().chain(g.rules.keys()).cloned()).collect();
    paths.extend(crate::code_rule_dsl::logic::rr::engine_files());
    let missed = given.derived.iter().flat_map(|g| g.phases.values().flatten().map(|(_, c)| c).filter(|c| g.absent.contains(c)).cloned()).collect();
    Some((paths, missed))
}

/// The programs' rule files read off their texts (`Texts::names`, which resolves each text's includes again) and
/// the phase programs their declarations name (`phases::phase_files`, which reads each), with the places a phase
/// program was looked for and not found.
#[allow(clippy::type_complexity)]
fn listed(given: &super::Texts<'_>, dir: Option<&Path>) -> Result<(Vec<std::path::PathBuf>, Vec<std::path::PathBuf>)> {
    let read = given.names()?;
    let (mut paths, missed) = super::phases::phase_files(read, dir)?;
    for t in read {
        paths.extend(t.included.iter().cloned());
    }
    Ok((paths, missed))
}

/// A record's lists as bytes: the observer's codec (one definition for every record, `observe::encode_lists`).
pub use crate::content_store::observe::{decode_lists, encode_lists};

/// A record's consulted list as bytes (`encode_lists`): each entry its kind, its path and its digest.
pub fn encode_consulted(consulted: &[(String, String, String)]) -> Vec<u8> {
    let flat: Vec<&str> = consulted.iter().flat_map(|(w, p, d)| [w.as_str(), p, d]).collect();
    encode_lists(&[&flat[..]])
}

/// The list `encode_consulted` made, or none where the bytes are not one.
pub fn decode_consulted(bytes: &[u8]) -> Option<Vec<(String, String, String)>> {
    let [flat]: [Vec<String>; 1] = decode_lists(bytes)?.try_into().ok()?;
    if flat.len() % 3 != 0 {
        return None;
    }
    let mut it = flat.into_iter();
    Some(std::iter::from_fn(|| Some((it.next()?, it.next()?, it.next()?))).collect())
}

/// The outputs a run wrote, as one file: a base of the relations `names` of the stack in `out` (decision 0239).
fn outputs_of<'a>(out: &Path, names: impl Iterator<Item = &'a str>) -> Vec<(String, Vec<u8>)> {
    crate::code_rule_dsl::logic::plan::output::base_of(out, names).map(|bytes| vec![(crate::code_rule_dsl::logic::plan::output::FILE.to_owned(), bytes)]).unwrap_or_default()
}

pub fn encode_outputs(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, bytes) in files {
        out.extend((name.len() as u32).to_le_bytes());
        out.extend(name.as_bytes());
        out.extend((bytes.len() as u64).to_le_bytes());
        out.extend(bytes);
    }
    out
}

pub fn decode_outputs(mut b: &[u8]) -> Option<Vec<(String, Vec<u8>)>> {
    let mut files = Vec::new();
    while !b.is_empty() {
        let n = u32::from_le_bytes(b.get(..4)?.try_into().ok()?) as usize;
        let name = String::from_utf8(b.get(4..4 + n)?.to_vec()).ok()?;
        b = &b[4 + n..];
        let m = u64::from_le_bytes(b.get(..8)?.try_into().ok()?) as usize;
        files.push((name, b.get(8..8 + m)?.to_vec()));
        b = &b[8 + m..];
    }
    Some(files)
}

/// The kept outputs written into `out`, when every read of the last run under this key is as it was.
#[allow(clippy::too_many_arguments)]
pub fn reused(given: &super::Texts<'_>, root: &Path, out: &Path, modes: &str, typing: &Typing, options: &serde_json::Value, mut captured: Option<&mut Vec<u8>>) -> Result<Option<Report>> {
    if crate::git_workspace::snapshot::contains(root) {
        return Ok(None);
    }
    // A run an analysis run owns (decision 0547) publishes what it recorded as the owner finishes: a hit re-owns the
    // manifests the recorded run wrote (`own_recorded_manifests`), below. Skipped instead, every `logic` command's
    // warm run evaluated again (each begins an analysis run).
    let owner = super::ThreadStore::owner(root).filter(|owner| owner.is_staged());
    let programs = given.programs;
    let checking = super::thread_usage();
    let key = key(given, &settings(root, options)?, root, options, modes, typing);
    let Some((consulted, outputs)) = observe(|o| o.run_record(&key)) else {
        miss(&format!("no record under the run's key {} (programs {}; {})", &key[..CUTOFF.len() + 12], programs.len(), key_parts(programs, root, options, typing)));
        return Ok(None);
    };
    let Some(consulted) = decode_consulted(&consulted) else {
        miss("the record's consulted list does not decode");
        return Ok(None);
    };
    // The plans it names are for collection (`CUTOFF`), not reads to check: the reused run names them, as the run
    // that made the record did, so its outer run records the manifest it recorded then.
    let (plans, consulted): (Vec<_>, Vec<_>) = consulted.into_iter().partition(|c| c.0 == "plan");
    let (graphs, consulted): (Vec<_>, Vec<_>) = consulted.into_iter().partition(|c| c.0 == GRAPHS);
    let (summary, consulted): (Vec<_>, Vec<_>) = consulted.into_iter().partition(|c| c.0 == SUMMARY);
    let plans: Vec<[u8; 32]> = plans.iter().filter_map(|c| crate::content_store::observe::unhex(&c.2)).collect();
    let Some(outputs) = decode_outputs(&outputs) else {
        miss("the record's outputs do not decode");
        return Ok(None);
    };
    // The library files first (a `stat` each), then the documents and the resolution rules' reads.
    let (rules, rest): (Vec<_>, Vec<_>) = consulted.into_iter().partition(|c| c.0 == "rule");
    // What the record names, by its digest, first: an edit's document or rule file differs here, before the rule
    // files are listed again (each phase program parsed) or the programs combined. Each is the observer's answer,
    // which the journal trusts unread where nothing changed.
    if let Some(c) = rules.iter().chain(rest.iter().filter(|c| c.0 == "doc" && !c.2.is_empty())).find(|c| super::world::bytes_digest(&root.join(&c.1)) != c.2) {
        miss(&format!("{} {} changed", c.0, c.1));
        return Ok(None);
    }
    // Decision 0275, rederived until a key settles it: with every rule file as recorded (a `stat` each, then its
    // sha), the same include graphs list the same rule files; the record's are compared, and the rule files are
    // listed again only for a record that has none.
    match graphs.first() {
        Some(g) if g.2 != graphs_line(&given.graphs) => {
            miss("the include graphs differ");
            return Ok(None);
        }
        Some(_) => {}
        None if rule_files(given, options)? != rules => {
            miss("the rule files differ");
            return Ok(None);
        }
        None => {}
    }
    // The program's parts the check reads, as recorded: a function of the texts (the key) and the rule files (as
    // recorded), so the program is not combined again to read them.
    let summary = summary.first().and_then(|c| super::Summary::decode(c.2.as_bytes()));
    if !unchanged(programs, &given.graphs, given.read, root, typing, options, &rest, summary)? {
        miss("a consulted file or read changed");
        return Ok(None);
    }
    if let Some(owner) = &owner {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        let owned = crate::code_rule_dsl::logic::profile::around_the_run(|| -> Result<usize> {
            let Some(store) = super::tape_cache().map_err(|e| Error::Plan(format!("the tape cache: {e}")))? else { return Ok(0) };
            store.own_recorded_manifests(&super::manifest_root(root), &plans, owner, now).map_err(|e| Error::Plan(format!("the recorded run's manifests: {e}")))
        })?;
        if owned == 0 {
            miss("the recorded run's manifests are gone, or its owner may no longer publish");
            return Ok(None);
        }
    }
    crate::code_rule_dsl::logic::profile::decision("whole-run cutoff (t23 §4)", || "every read as it was: the kept outputs written, nothing evaluated".into());
    if captured.is_none() { std::fs::create_dir_all(out).map_err(|e| Error::Plan(format!("{}: {e}", out.display())))?; }
    let mut report = Report { plans, ..Report::default() };
    for (name, bytes) in &outputs {
        if name != crate::code_rule_dsl::logic::plan::output::FILE {
            return Err(Error::Plan(format!("{name}: no output file of this run's")));
        }
        if let Some(target) = captured.as_deref_mut() {
            target.extend_from_slice(bytes);
        } else {
            crate::code_rule_dsl::logic::plan::output::replace(out, &[bytes]).map_err(|e| Error::Plan(format!("{name}: {e}")))?;
        }
        let image = crate::code_rule_dsl::logic::plan::output::Image::one(bytes).map_err(|e| Error::Plan(format!("{name}: {e}")))?;
        for rel in image.names() {
            report.rows.insert(rel.to_owned(), image.rel(rel).expect("named").rows);
        }
    }
    let now = super::thread_usage();
    report.phases.push(("unchanged: phase 0 checked every read, outputs kept", now.0 - checking.0, now.1 - checking.1, super::usage().2, 0));
    // The documents the run read: the workspace's and those its requests loaded, each once. Phase 1 loads a
    // workspace document by its absolute path, which the run lists by its path from the root: counted twice, a warm
    // run of one file reported two.
    let base = crate::git_workspace::snapshot::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let read: std::collections::BTreeSet<std::path::PathBuf> =
        rest.iter().filter(|c| (c.0 == "doc" && !c.2.is_empty()) || c.0 == "load").map(|c| base.join(&c.1)).collect();
    report.files = read.len();
    report.consulted = rest;
    Ok(Some(report))
}

/// Keeps the record of a run that ended: what it consulted (and the library files its text includes), and its
/// outputs.
/// Why the cutoff did not apply: a profile line, and stderr under `RERAIL_REUSE_TRACE`.
fn miss(why: &str) {
    crate::code_rule_dsl::logic::profile::decision("whole-run cutoff (t23 §4)", || format!("not applied: {why}"));
    if std::env::var_os("RERAIL_REUSE_TRACE").is_some() {
        eprintln!("rerail: whole-run cutoff not applied: {why}");
    }
    // A warm run asked for the cutoff and evaluates again (`RERAIL_WARM`). One that did not ask (a session's, which
    // maintains its strata instead) is checked by its deltas.
    if !why.starts_with("not wanted") {
        crate::code_rule_dsl::logic::profile::warm_fault("the whole-run cutoff did not apply", || why.to_owned());
    }
}

pub fn record(given: &super::Texts<'_>, root: &Path, out: &Path, modes: &str, typing: &Typing, options: &serde_json::Value, report: &Report, captured: Option<&[u8]>) -> Result<()> {
    if crate::git_workspace::snapshot::contains(root) {
        return Ok(());
    }
    let programs = given.programs;
    // Every phase's program is among the rule files (`rule_files`), and what each phase consulted is the record's
    // too: a phase's exports are a function of its program and what it consulted, so a run whose every read, its
    // phases' included, is as it was derives the same (O-cutoff-phases). Phase 1's are the source files, and the
    // directories, files and library documents its imports read.
    if report.consulted.is_empty() {
        miss("not recorded: nothing consulted");
        return Ok(());
    }
    let key = key(given, &settings(root, options)?, root, options, modes, typing);
    let mut consulted = rule_files(given, options)?;
    let mut read: Vec<(String, String, String)> = report.consulted.iter().cloned().chain(report.phase_runs.iter().flat_map(|r| r.report.consulted.iter().cloned())).collect();
    read.sort();
    read.dedup();
    // One entry a document: a phase that read its bytes names its digest, where another listed it unread.
    let read_docs: std::collections::BTreeSet<String> = read.iter().filter(|c| c.0 == "doc" && !c.2.is_empty()).map(|c| c.1.clone()).collect();
    read.retain(|c| !(c.0 == "doc" && c.2.is_empty() && read_docs.contains(&c.1)));
    consulted.extend(read);
    consulted.extend(report.plans.iter().map(|plan| ("plan".to_owned(), String::new(), crate::content_store::observe::hex(plan))));
    consulted.push((GRAPHS.to_owned(), String::new(), graphs_line(&given.graphs)));
    if let Some(summary) = super::Summary::of(programs, &given.graphs, given.read, root, options)?.encode().and_then(|b| String::from_utf8(b).ok()) {
        consulted.push((SUMMARY.to_owned(), String::new(), summary));
    }
    let files = match captured {
        Some(bytes) => vec![(crate::code_rule_dsl::logic::plan::output::FILE.to_owned(), bytes.to_vec())],
        None => outputs_of(out, report.rows.keys().map(String::as_str)),
    };
    let outputs = encode_outputs(&files);
    if std::env::var_os("RERAIL_REUSE_TRACE").is_some() {
        eprintln!("rerail: whole-run cutoff: recorded under {} ({} consulted)", &key[..CUTOFF.len() + 12], consulted.len());
    }
    observe(|o| o.put_run_record(&key, &encode_consulted(&consulted), &outputs));
    Ok(())
}

// Its tests run in the engine's test binary (its facade of this module mounts `tests.rs`): they read what the
// engine registers as it loads (the languages, their `.rerail` libraries).
