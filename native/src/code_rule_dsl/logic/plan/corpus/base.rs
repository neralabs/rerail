//! The corpus's base, below the language's root (`logic::rr`, rerail-rr-root's) and the corpus runner (`corpus.rs`,
//! rerail-logic's): the store a run keeps tapes in and its selection (`ThreadStore`, `tape_cache_path`,
//! `build_store`), phase 0's observer of the files read (`observe`), the programs combined from a run's texts
//! (`combine_*`, `Combined`), a run's report and summary (`Report`, `Summary`, `changes_of`), and the runner the
//! language reads, behind the registered `RUNNER`. `corpus.rs` re-exports it (through rerail-rr-root's `corpus`).

use super::*;

/// The corpus runner, as the corpus's lower modules call it: phase 0 runs its bootstrap program through it to read the
/// rule library's imports (`phase0`), so the language's loading runs a program, and the runner reads the language.
/// The runner is rerail-logic's (`corpus.rs`, `Corpus`), above the language's root; the program's composition
/// registers it (`RUNNER`, boot), as each test binary that runs a program does.
pub trait Runner: Send + Sync {
    /// `programs`' run over `root` (`corpus::run_handed`): its report, and its outputs' image into `image`.
    #[allow(clippy::too_many_arguments)]
    fn run_handed(&self, programs: &[&str], read: Option<&Arc<crate::code_rule_dsl::logic::program_image::Version>>, sources: Option<&texts::Sources>, root: &Path, t_facts: Option<&Path>, out: &Path, modes: &str, binding: Binding, chunk_nodes: usize, typing: &Typing, options: &serde_json::Value, image: Option<&mut Vec<u8>>, phase: FactPhase<'_>) -> Result<Report>;
}

/// The registered corpus runner (`Runner`).
pub static RUNNER: rerail_registry::Slot<&'static dyn Runner> = rerail_registry::Slot::new("the corpus runner (plan::corpus::RUNNER)");

/// `programs`' run through the registered runner (`RUNNER`).
#[allow(clippy::too_many_arguments)]
pub fn run_handed(programs: &[&str], read: Option<&Arc<crate::code_rule_dsl::logic::program_image::Version>>, sources: Option<&texts::Sources>, root: &Path, t_facts: Option<&Path>, out: &Path, modes: &str, binding: Binding, chunk_nodes: usize, typing: &Typing, options: &serde_json::Value, image: Option<&mut Vec<u8>>, phase: FactPhase<'_>) -> Result<Report> {
    let runner = RUNNER.get().unwrap_or_else(|missing| panic!("{missing}: no program registered the corpus runner"));
    runner.run_handed(programs, read, sources, root, t_facts, out, modes, binding, chunk_nodes, typing, options, image, phase)
}

/// The switches that change the combined program (`combine_tail`, with the `.rerail` lowering its `rr::parse` runs): its
/// key (`texts_digest`, in memory and in the store) holds their values, so a combined program kept or stored under
/// one value is not another's. Every prepared key holds the combined program's.
pub const COMBINING: &[&str] = &[
    // `combine_tail`: the copies to compare (`=copied`) and decision 0091's measurement (`FactSpaces`).
    "RERAIL_LIBRARIES",
    "RERAIL_REQUESTS",
    "RERAIL_SCOPE_GUARDS",
    // The combine's tail kept or run whole (`=whole`), or checked against the whole (`=check`, decision 0393).
    "RERAIL_COMBINE_TAIL",
    // The `.rerail` lowering: source attributes answered eagerly (`on_request::SourcesOnRequest`), node ids compared raw
    // (`lowering_tail::OrderNodes`).
    "RERAIL_NODE_SOURCES",
    "RERAIL_ORDER_RAW",
    // Node ids by definition or in preorder (`syntax::definition_ids`, read by `order_nodes`).
    "RERAIL_NODE_IDS",
    // Identical relations merged (`combining::same_relations`, in combining and again in preparing).
    "RERAIL_SAME",
    // The `.rerail` lowering's rewrites, `=off` to leave the rules as written: a min or max bounded by its probe
    // (`lower::bounded`, decision 0511) and a group's first by an order (`lower::first`, decision 0515).
    "RERAIL_ASOF",
    "RERAIL_FIRST",
    // A narrow copy derived again, `=off`, not diffed from its original (`combining::Diffed`, decision 0575).
    "RERAIL_NARROW_DIFF",
];

pub fn text(v: &SqlValue) -> String {
    super::super::text(v)
}

/// The corpus batch's documents in a run (decision 0458): the memo hits' documents built from their stored tapes or
/// parsed again, those a session held from its last run, and of the batch's documents those whose rows were made
/// (decision 0494). A warm run builds, parses and makes nothing of a file that did not change (Q332).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CorpusDocs {
    /// The memo hits' documents built from the tapes the store kept: each reads its file and builds its facts.
    pub from_tape: usize,
    /// The memo hits' documents whose file was parsed again (the store kept no tape of it).
    pub parsed: usize,
    /// The documents a session held from its last run, unchanged.
    pub held: usize,
    /// The batch's documents whose rows were made (`Batch::documents_made`), and all it holds.
    pub made: usize,
    pub documents: usize,
}

/// What a corpus run did: its phases' CPU (user, system) and the peak RSS after each, and the outcome.
#[derive(Default)]
pub struct Report {
    /// (phase, user, sys, peak RSS so far, SQLite statements run).
    pub phases: Vec<(&'static str, f64, f64, f64, usize)>,
    /// Per phase, the whole process's retired instructions while it ran (its workers' too; macOS): steadier than CPU
    /// time on a loaded machine, so a change is attributed to a phase by them.
    pub phase_instructions: Vec<(&'static str, u64)>,
    /// Per phase of the run's thread, its clocks at the phase's bounds (decision 0288): wall, the thread's own CPU and
    /// helping, the process's CPU, its threads' time ready with no core, and its involuntary switches.
    pub phase_clock: Vec<(&'static str, crate::code_rule_dsl::logic::profile::PhaseClock)>,
    pub files: usize,
    pub nodes: usize,
    pub native: usize,
    pub rows: BTreeMap<String, usize>,
    pub chunks: usize,
    /// Declarations, summaries and terms (`corpus::typing`).
    pub typing: TypingReport,
    /// Text tests (decision 0037): relations, distinct strings tested, and the CPU testing them (seconds).
    pub text: (usize, usize, f64),
    /// The earlier phases this run read (decision 0083 §1), each its own run.
    pub phase_runs: Vec<phases::PhaseRun>,
    /// The plans this run and the runs nested in it used (its memo plan, its prepared programs, its phases', its
    /// constants' and phase 0's runs'): the outermost run's record names them all, so they stay live while a root's
    /// last runs name it (`keep_corpus_runs`). One process records a run per nested run, so recorded alone the
    /// phases' plans fell out of a root's last two runs after every process and were prepared again by the next.
    pub plans: Vec<[u8; 32]>,
    /// The per-file memo: files read from it, and chunks whose rows it could not keep (a row naming no document).
    pub memo: (usize, usize),
    /// Of the files the memo held, those read as partial hits (decision 0225 §2): their entries kept but the
    /// relations an edit changed, which their chunk derives again.
    pub memo_partial: usize,
    /// Of the files the memo held, those read for their outputs alone (decision 0117 step 1), and the values of
    /// their export and base rows decoded all the same: none, as the kept inputs hold those rows.
    pub memo_kept: (usize, usize),
    /// The statistics keeper's end (decision 0096 §3): the files it counted in and out this run, or why it gave them
    /// up (the next run counts every file in again). `None`: no keeper ran.
    pub stats_kept: Option<std::result::Result<(usize, usize), String>>,
    /// Where the prepared program came from: `prepared` by this run, kept in this process's `memory`, or read from the
    /// `store` (decision 0073).
    pub prepared: &'static str,
    /// A session's run: the cross-file strata it evaluated, of all (decision 0077; `None` outside a session).
    pub cross_strata: Option<(usize, usize)>,
    /// A run with no session, its strata kept in the tape cache (decision 0077 §4): the cross-file strata it
    /// evaluated, of all.
    pub cross_kept: Option<(usize, usize)>,
    /// The cone of requests' strata evaluated, of all, when they are kept (t33 K3); `None` otherwise.
    pub cone_kept: Option<(usize, usize)>,
    /// The files built in its waves whose tape was read before the first wave, and those parsed (Q22): a run over
    /// unchanged files parses none, whatever its programs.
    pub file_tapes: (usize, usize),
    /// The corpus batch's documents (decision 0458) and what the run did to make them: what a warm run must not do to
    /// an unchanged file (Q332, `warm_tests`).
    pub corpus_docs: CorpusDocs,
    /// The files the run parsed, by the path that parsed them and their language (`core::parses`): a warm run
    /// parses only what changed.
    pub parsed: std::collections::BTreeMap<(&'static str, String), usize>,
    /// The guests chunks of partial hits did not make, their cones unable to see them (`guests::guests_seen`).
    pub guests_unseen: usize,
    /// The guest texts parsed (decision 0431 R6): a text a run parsed before is not parsed again.
    pub guest_parses: usize,
    /// Decision 0431 R11 (step 6): the files' programs the batches read off a file's record (made at its parse, or read
    /// with its stored tape), and those an embedder made in a batch, the file's record not known.
    pub programs: (usize, usize),
    /// Decision 0431 §8, R10: the tape values the chunks' batches admitted at a lookup, each once a chunk (`@parse`'s
    /// and `@splice`'s too, Q325: a chunk's batch is built once, and grows only by what its fixpoint asks).
    pub values_admitted: usize,
    /// The rounds the cone ran from the last run's final round (decision 0145); 0 when it ran the gated fixpoint.
    pub cone_rounds: usize,
    /// The requests a language's answer answered this run (`language::Answerer`), each with its key's first column (a
    /// path), in the order asked.
    pub language_asked: Vec<(String, String)>,
    /// Of those, strata maintained from deltas, of them recursive, and strata derived again (decision 0077 §3).
    pub cross_maintained: (usize, usize, usize),
    /// Of those, the strata passed on as quiet, and the relations they held with no column measured (`Outcome::quiet`).
    pub cross_quiet: (usize, usize),
    /// A session's maintained run: the strata it skipped, no read of them changed (0364 R9; `Outcome::walked`).
    pub cross_skipped: usize,
    /// Of those, the strata a changed read reached that no rule of saw a row of its delta, by kind or key (decision
    /// 0563 B; `Outcome::unseen`), by their keys.
    pub cross_unseen: Vec<String>,
    /// The cross-file relations handed off as the run went, their frames made once no later stratum read them and
    /// their rows let go (Q74): a run not driven by deltas.
    pub cross_handed: usize,
    /// The cross-file inputs let go after their last reader, their frames written before the run (Q74).
    pub cross_unloaded: usize,
    /// A session's run: the cross part's inputs that changed, and why its strata derived again were (each count).
    pub cross_changed: Vec<String>,
    pub cross_again: BTreeMap<&'static str, usize>,
    /// With `RERAIL_CROSS_DELTA_COUNTS`: rows over-deleted, inserted and held by the maintained relations, the rows of
    /// the deltas of their inputs, and the rows truly deleted and inserted.
    pub cross_delta_counts: [usize; 6],
    /// What the run consulted, as (what, path, digest): each workspace document's bytes, and what its requests read
    /// (`world::World::consulted`). A run whose consulted things are as they were derives the same (`unchanged`).
    pub consulted: Vec<(String, String, String)>,
    /// Each document's rows of the per-file relations the requests' cone reads, as a digest, by path: node ids by
    /// their rank in the document, so an edit that renumbers nodes and nothing else leaves it. Two runs whose
    /// documents give the same digests, and whose requests are answered alike, load the same documents.
    pub cone_inputs: BTreeMap<String, String>,
    /// An output that is not a union of file-local rows, if any. With none, each document's rows are its own,
    /// whatever else is read, and can be replaced on their own.
    pub not_per_document: Option<String>,
    /// What phase 0's observer did during the run: stats, answers reused, files hashed, directories listed.
    pub files_observed: crate::content_store::observe::Counts,
    /// The run's own record (decision 0441: what it met of its limits, the trees it decoded), where it was the
    /// outermost run.
    pub run: Option<crate::code_rule_dsl::logic::run::Record>,
}

/// Whether a run of `programs` over `root` that consulted `consulted` (`Report::consulted`) would consult the same
/// now: the same workspace documents with the same bytes, and every request answered alike (decision 0083 §2). It
/// reads what the run read, and derives nothing. A program whose inputs come from elsewhere too (declarations,
/// earlier phases, an explicit batch) is never judged unchanged.
/// `graphs`: each program text's include graph's digest, as the run found it (`run_handed`).
pub fn unchanged(programs: &[&str], graphs: &[Option<[u8; 32]>], read: Option<&Arc<crate::code_rule_dsl::logic::program_image::Version>>, root: &Path, typing: &Typing, options: &serde_json::Value, consulted: &[(String, String, String)], summary: Option<Summary>) -> Result<bool> {
    Ok(changes_of(programs, graphs, read, root, typing, options, consulted, summary)?.is_some_and(|c| c.is_empty()))
}

/// The parts of a run's combined program that `changes` reads, and nothing else: whether it reads `declared_ret`,
/// its settings (its language, its artifacts), its scopes and text globs, its JSON policies (their scopes), and its
/// `env` facts (the world's roots). A function of the texts and the rule files they read: the whole-run cutoff's
/// record keeps it beside the run's (decision 0275), so its check does not combine the program again to read them.
#[derive(Clone, Debug, Default)]
pub struct Summary {
    declared_ret: bool,
    env: Vec<front::List<front::Value>>,
    settings: Vec<(String, String)>,
    policies: Vec<serde_json::Value>,
    doc_scopes: Option<Vec<(Vec<String>, Vec<String>)>>,
    text_globs: Vec<String>,
    /// Whether, over a Git revision, the outputs read Git metadata alone (`git::metadata_only`): no source listed.
    git_metadata_only: bool,
}

impl Summary {
    /// The summary of the program `programs` combine to, as `changes` combines them (the root's settings in each).
    pub(crate) fn of(programs: &[&str], graphs: &[Option<[u8; 32]>], read: Option<&Arc<crate::code_rule_dsl::logic::program_image::Version>>, root: &Path, options: &serde_json::Value) -> Result<Summary> {
        Ok(Self::from_fields(&*combined_with_settings(programs, graphs, read, root, options)?))
    }

    fn from_fields(p: &crate::code_rule_dsl::logic::program_image::Version) -> Summary {
        Summary {
            declared_ret: p.declares("declared_ret"),
            env: ProgramFields::fact_rows(p, "env"),
            settings: ProgramFields::settings(p),
            policies: ProgramFields::policies(p),
            doc_scopes: ProgramFields::doc_scopes(p),
            text_globs: ProgramFields::text_globs(p),
            git_metadata_only: git::metadata_only(p),
        }
    }

    /// Its bytes: the parts as JSON.
    pub(crate) fn encode(&self) -> Option<Vec<u8>> {
        let env: Vec<Vec<serde_json::Value>> = self.env.iter().map(|row| row.iter().map(|v| match v {
            front::Value::Int(i) => serde_json::json!({ "i": i }),
            // An atom's value at run time is its integer (decision 0270); the settings' rows hold none.
            front::Value::Branded(b) => serde_json::json!({ "i": b.value() }),
            front::Value::Str(s) => serde_json::json!({ "s": s.as_str() }),
        }).collect()).collect();
        let parts = serde_json::json!({
            "declared_ret": self.declared_ret,
            "git_metadata_only": self.git_metadata_only,
            "env": env,
            "settings": self.settings,
            "policies": self.policies,
            "doc_scopes": self.doc_scopes,
            "text_globs": self.text_globs,
        });
        serde_json::to_vec(&parts).ok()
    }

    pub(crate) fn decode(bytes: &[u8]) -> Option<Summary> {
        let v: serde_json::Value = serde_json::from_slice(bytes).ok()?;
        let mut env = Vec::new();
        for row in v["env"].as_array()? {
            let mut out = front::List::default();
            for c in row.as_array()? {
                out.push(match (c.get("i").and_then(serde_json::Value::as_i64), c.get("s").and_then(serde_json::Value::as_str)) {
                    (Some(i), _) => front::Value::Int(i),
                    (_, Some(s)) => front::Value::Str(front::Name::new(s)),
                    _ => return None,
                });
            }
            env.push(out);
        }
        Some(Summary {
            declared_ret: v["declared_ret"].as_bool()?,
            env,
            settings: serde_json::from_value(v["settings"].clone()).ok()?,
            policies: serde_json::from_value(v["policies"].clone()).ok()?,
            doc_scopes: serde_json::from_value(v["doc_scopes"].clone()).ok()?,
            text_globs: serde_json::from_value(v["text_globs"].clone()).ok()?,
            git_metadata_only: v["git_metadata_only"].as_bool()?,
        })
    }
}

/// The summary's fields as a program's, for the readers `changes` shares with the run (`scoped`, `World`, `inputs`).
impl ProgramFields for Summary {
    fn input_names(&self) -> Vec<String> {
        Vec::new()
    }
    fn output_names(&self) -> Vec<String> {
        Vec::new()
    }
    fn declares(&self, rel: &str) -> bool {
        rel == "declared_ret" && self.declared_ret
    }
    fn columns_of(&self, rel: &str) -> Option<Vec<(String, String)>> {
        self.declares(rel).then(Vec::new)
    }
    fn declared_names(&self) -> Vec<String> {
        if self.declared_ret { vec!["declared_ret".to_owned()] } else { Vec::new() }
    }
    fn fact_names(&self) -> Vec<String> {
        if self.env.is_empty() { Vec::new() } else { vec!["env".to_owned()] }
    }
    fn states_facts(&self, rel: &str) -> bool {
        rel == "env" && !self.env.is_empty()
    }
    fn fact_rows(&self, rel: &str) -> Vec<front::List<front::Value>> {
        if rel == "env" { self.env.clone() } else { Vec::new() }
    }
    fn about(&self, _: &[&str]) -> crate::code_rule_dsl::logic::program_facts::Facts {
        Default::default()
    }
    fn settings(&self) -> Vec<(String, String)> {
        self.settings.clone()
    }
    fn policies(&self) -> Vec<serde_json::Value> {
        self.policies.clone()
    }
    fn text_globs(&self) -> Vec<String> {
        self.text_globs.clone()
    }
    fn doc_scopes(&self) -> Option<Vec<(Vec<String>, Vec<String>)>> {
        self.doc_scopes.clone()
    }
}

/// The program `programs` combine to, as `changes` combines them: the root's settings (`<root>/rerail.env`, the `env`
/// option) as the rows the run adds (decision 0249), the texts as the run's keys read them, messages masked
/// (`texts_digest`, decision 0225 §3).
pub fn combined_with_settings(programs: &[&str], graphs: &[Option<[u8; 32]>], read: Option<&Arc<crate::code_rule_dsl::logic::program_image::Version>>, root: &Path, options: &serde_json::Value) -> Result<Arc<crate::code_rule_dsl::logic::program_image::Version>> {
    let flags: Vec<String> = options.get("env").and_then(|v| v.as_array()).into_iter().flatten().filter_map(|v| v.as_str().map(str::to_owned)).collect();
    let settings = crate::code_rule_dsl::logic::settings::settings(root, &flags).map_err(Error::Plan)?;
    let masked = masked(programs);
    Ok(combine_version(programs, &masked.iter().map(|t| t.as_ref()).collect::<Vec<_>>(), graphs, &added::Added::of(&settings, options), read)?.0)
}

/// `changes`, reading the program's parts off `summary` where one is given (the whole-run cutoff's record), else off
/// the program combined again.
#[allow(clippy::too_many_arguments)]
pub fn changes_of(programs: &[&str], graphs: &[Option<[u8; 32]>], read: Option<&Arc<crate::code_rule_dsl::logic::program_image::Version>>, root: &Path, typing: &Typing, options: &serde_json::Value, consulted: &[(String, String, String)], summary: Option<Summary>) -> Result<Option<Vec<String>>> {
    let trace = |why: String| if std::env::var_os("RERAIL_REUSE_TRACE").is_some() { eprintln!("rerail: changes: {why}"); };
    // A run given no file of its own (`inputFiles: []`, a phase's: phase 1 loads its documents by request) has no
    // listed documents to compare; what it loaded is among its requests, checked below. Other named files are not
    // compared.
    let none_named = options.get("inputFiles").and_then(|v| v.as_array()).is_some_and(Vec::is_empty);
    if (options.get("inputFiles").is_some() && !none_named) || consulted.is_empty() {
        trace(format!("not comparable: input files {}, {} consulted", options.get("inputFiles").is_some(), consulted.len()));
        return Ok(None);
    }
    let prog = match summary {
        Some(summary) => summary,
        None => Summary::from_fields(&*combined_with_settings(programs, graphs, read, root, options)?),
    };
    let metadata_only = prog.git_metadata_only;
    // A program reading earlier phases is comparable: its record names every phase's program and what each phase
    // consulted (`reuse::record`, O-cutoff-phases).
    if prog.declared_ret {
        trace("not comparable: declared_ret".to_owned());
        return Ok(None);
    }
    let language = prog.settings.iter().find(|(key, _)| key == "language").map(|(_, value)| value.trim().trim_matches('"').to_owned());
    let files = if none_named || metadata_only && crate::git_workspace::snapshot::contains(root) {
        Vec::new()
    } else {
        let found = scoped(corpus_files(root)?, &prog, options);
        let label = corpus_language(language.as_deref(), typing, &found);
        found.of(label)
    };
    let docs: Vec<&(String, String, String)> = consulted.iter().filter(|c| c.0 == "doc").collect();
    // The workspace's documents are listed by their path from the root; a library document a phase loaded is named
    // by its absolute path, and is checked by its digest below, as every document is (O-cutoff-phases).
    let workspace: Vec<&&(String, String, String)> = docs.iter().filter(|d| !Path::new(&d.1).is_absolute()).collect();
    if files.len() != workspace.len() || files.iter().zip(&workspace).any(|(f, d)| *f != d.1) {
        trace(format!("the documents differ: {} files, {} kept", files.len(), workspace.len()));
        return Ok(None);
    }
    // A document listed without a digest was not read (none of the run's rules reads a document): its edit changes nothing.
    let changed: Vec<String> = docs.iter().filter(|d| !d.2.is_empty() && world::bytes_digest(&root.join(&d.1)) != d.2).map(|d| d.1.clone()).collect();
    const READERS: [&str; 12] = ["git_object_predicate", "package_marker", "input", "input_dir", "artifact", "base", "git_rev", "git_index", "git_tracking_index", "git_revision_rows", "git_root_repo", analysis_history::INPUT];
    let (read, requests): (Vec<_>, Vec<_>) = consulted.iter().filter(|c| c.0 != "doc").cloned().partition(|c| READERS.contains(&c.0.as_str()));
    let mut world = world::World::new(root, &prog);
    let moved = world.changed(&requests);
    let (inputs_still, world_still) = (inputs::still(&prog, root, &read), moved.is_none());
    if !(inputs_still && world_still) {
        trace(format!("inputs as they were {inputs_still} ({} read), requests as they were {world_still} ({} requests; first changed {moved:?})", read.len(), requests.len()));
    } else if !changed.is_empty() {
        trace(format!("{} documents changed: {:?}", changed.len(), changed.iter().take(3).collect::<Vec<_>>()));
    }
    Ok((inputs_still && world_still).then_some(changed))
}

/// The `core.rerail` relations a rule may read that the engine does not make yet (O76): each is empty. None now:
/// `string_annotation` is made (`facts::vtab::syntax::annotation`, Ruff's table) and `parsed` is a guest (0055).
pub const NOT_MADE: [(&str, &str); 0] = [];

/// The run's program texts, each read once (`rr::read_seed`, in `run_with_options`): their include graphs' digests,
/// and their library files and names, resolved when the whole-run cutoff first asks (`reuse::rule_files`). Held by
/// the run, and gone with it.
pub struct Texts<'a> {
    pub programs: &'a [&'a str],
    /// Each text's include graph's digest (`phase0::Graph::digest`), `None` for a text that includes nothing.
    pub graphs: Vec<Option<[u8; 32]>>,
    /// Phase 0's graphs for the texts that include something, one run of `bootstrap.rerail` for all of them.
    pub derived: Vec<Arc<phase0::Graph>>,
    /// The program the run was handed, already read (`run_read`): the run holds it, and `programs` is its key.
    pub read: Option<&'a Arc<crate::code_rule_dsl::logic::program_image::Version>>,
    /// The source texts of the run inside which this one runs (phase 1's: the outer run's, decision 0300), which its
    /// builder borrows.
    pub sources: Option<&'a texts::Sources>,
    pub phase: FactPhase<'a>,
    pub reads: std::cell::RefCell<Vec<crate::code_rule_dsl::logic::rr::TextRead>>,
    pub names: std::cell::OnceCell<Vec<crate::code_rule_dsl::logic::rr::TextNames>>,
}

impl<'a> Texts<'a> {
    /// A program already read, keyed by `key` (`read_key`): it includes nothing to derive a graph of, and its names
    /// are its own (`rr::TextNames::of_program`), as a print of it had them.
    pub fn of_read(key: &'a [&'a str], read: &'a Arc<crate::code_rule_dsl::logic::program_image::Version>) -> Self {
        let names = std::cell::OnceCell::new();
        let _ = names.set(vec![crate::code_rule_dsl::logic::rr::TextNames::of_program(&**read)]);
        Texts { programs: key, graphs: vec![None], derived: Vec::new(), read: Some(read), sources: None, phase: Default::default(), reads: std::cell::RefCell::new(Vec::new()), names }
    }

    /// `programs`, each parsed once, and phase 0's graphs derived for those that include something.
    pub fn of(programs: &'a [&'a str]) -> Result<Self> {
        let reads: Vec<crate::code_rule_dsl::logic::rr::TextRead> = programs.iter().map(|t| crate::code_rule_dsl::logic::rr::read_seed(t)).collect::<std::result::Result<_, _>>()?;
        let seeds: Vec<phase0::Seed> = reads.iter().filter_map(|r| r.seed.clone()).collect();
        let derived = if phase0::active() { Vec::new() } else { phase0::graphs(&seeds)? };
        let mut at = derived.iter();
        let graphs = reads.iter().map(|r| r.seed.as_ref().and_then(|_| at.next()).map(|g| g.digest())).collect();
        Ok(Texts { programs, graphs, derived, read: None, sources: None, phase: Default::default(), reads: std::cell::RefCell::new(reads), names: std::cell::OnceCell::new() })
    }

    /// Each text's library files, declared relations and phase (`rr::TextRead::resolved`).
    pub fn names(&self) -> Result<&[crate::code_rule_dsl::logic::rr::TextNames]> {
        if self.names.get().is_none() {
            let reads = std::mem::take(&mut *self.reads.borrow_mut());
            let names = reads.into_iter().map(crate::code_rule_dsl::logic::rr::TextRead::resolved).collect::<std::result::Result<Vec<_>, _>>()?;
            let _ = self.names.set(names);
        }
        Ok(self.names.get().expect("set above"))
    }
}

/// A combined program kept for the process (`COMBINED`): its version (`combine_version`'s).
#[derive(Default)]
pub struct Combined {
    version: Option<Arc<crate::code_rule_dsl::logic::program_image::Version>>,
}

/// The combined programs kept by their texts' digest, the caller's (decision 0442: the shared object's): a function
/// of the texts alone (`rr::parse` reads no file), so a session or a test that runs one program over many roots
/// parses and hides it once.
#[derive(Default)]
pub struct CombinedPrograms(std::sync::Mutex<HashMap<[u8; 32], Combined>>);

pub fn combined_kept() -> std::sync::Arc<CombinedPrograms> {
    crate::code_rule_dsl::logic::profile::shared::<CombinedPrograms>()
}

/// `edit` of the program `key` kept for the process (`combined_kept`). A few programs at a time are kept.
pub fn keep_combined(key: [u8; 32], edit: impl FnOnce(&mut Combined)) {
    const KEPT: usize = 32;
    let held = combined_kept();
    let mut kept = held.0.lock().expect("the combined programs");
    if kept.len() >= KEPT && !kept.contains_key(&key) {
        kept.clear();
    }
    edit(kept.entry(key).or_default());
}

/// `combine`, the texts' include graphs' digests given (`Texts::graphs`), each program given the rows `extra` adds
/// (`added::Added`), with the digest of its texts, messages masked (`masked`, decision 0225 §3), and those rows
/// (`texts_digest`) it is kept by: a run keys what it prepares by the same digest. `read` is the program the one text
/// prints from, already lowered (`Texts::read`): the program itself, not its text lowered again. The image is made
/// once per combined program and kept for the process (`COMBINED`).
pub fn combine_version(programs: &[&str], masked: &[&str], graphs: &[Option<[u8; 32]>], extra: &added::Added, read: Option<&Arc<crate::code_rule_dsl::logic::program_image::Version>>) -> Result<(Arc<crate::code_rule_dsl::logic::program_image::Version>, [u8; 32])> {
    let key = texts_digest(masked, graphs, extra);
    if let Some(v) = combined_kept().0.lock().expect("the combined programs").get(&key).and_then(|k| k.version.clone()) {
        crate::code_rule_dsl::prof::count("corpus.combined_reused", 1);
        return Ok((v, key));
    }
    // The one text a run was handed the program of: its hidden program kept under the texts' digest too, as a pack's
    // is (decision 0275): the lowered program is the key, and hiding it again is the step a hit saves. The rows the
    // run adds (decision 0249) are in the key.
    let one = read.filter(|_| programs.len() == 1);
    let v = combine_stored_version(programs, extra, &key, one)?;
    keep_combined(key, |k| k.version = Some(Arc::clone(&v)));
    Ok((v, key))
}

/// The programs linked into one (`combine_linked_as`), and the combine's tail run on it (`tail_version`), with the kept
/// tail run it was made over or kept, if any (`tail_kept`).
pub fn combine_tail(programs: &[&str], extra: &added::Added, read: Option<&Arc<crate::code_rule_dsl::logic::program_image::Version>>, images_used: Option<&mut Vec<crate::content_store::HashKey>>) -> Result<(Arc<crate::code_rule_dsl::logic::program_image::Version>, Option<[u8; 32]>)> {
    let mut images_used = images_used;
    let (v, tail) = combine_linked_as::<link::Versioned>(programs, extra, read, images_used.as_deref_mut())?;
    // `RERAIL_COMBINE_LINK=check`: the splice (decision 0373) against the programs linked whole.
    if combine_link_mode() == "check" {
        let (whole, whole_tail) = crate::code_rule_dsl::logic::profile::quiet(|| combine_linked_spliced::<link::Versioned>(programs, extra, read, None, false))?;
        assert_eq!((whole.size(), whole.digest()), (v.size(), v.digest()), "the combine's splice is its whole link: {}", crate::code_rule_dsl::logic::program_image::differences(&whole, &v));
        assert_eq!(whole_tail.shape(), tail.shape(), "the combine's splice has its whole link's tail");
        crate::code_rule_dsl::logic::profile::decision("the combine's last link (decision 0373)", || "checked: the splice is the whole link (RERAIL_COMBINE_LINK=check)".into());
    }
    // The parts linked as trees beside (decision 0281's path, step 7a-2): the same program.
    let (v, run) = match &tail {
        Tail::Many { kept: Some(kept), .. } if combine_tail_mode() != "whole" => {
            let kept = *kept;
            let (spliced, run) = tail_kept(v.clone(), &tail, kept, images_used)?;
            // `RERAIL_COMBINE_TAIL=check`: the tail kept (decision 0393) against the tail run whole.
            if combine_tail_mode() == "check" {
                let whole = crate::code_rule_dsl::logic::profile::quiet(|| tail_version(v, &tail))?;
                assert_eq!((whole.size(), whole.digest()), (spliced.size(), spliced.digest()), "the combine's kept tail is its whole tail: {}", crate::code_rule_dsl::logic::program_image::differences(&whole, &spliced));
                crate::code_rule_dsl::logic::profile::decision("the combine's tail (decision 0393)", || "checked: the kept tail is the whole tail (RERAIL_COMBINE_TAIL=check)".into());
            }
            (spliced, run)
        }
        _ => (tail_version(v, &tail)?, None),
    };
    Ok((v, run))
}

/// The combine kept in the persistent store across processes for a pack of programs (`combined`, decision 0094 §6):
/// keyed by the texts and checked against the library files the combine read. The store's entry opened as a version
/// (`combined::lookup_version`), or the combine's tail run on the version (`combine_tail`) and its image kept as the
/// entry (`combined::record_version`).
pub fn combine_stored_version(programs: &[&str], extra: &added::Added, key: &[u8; 32], read: Option<&Arc<crate::code_rule_dsl::logic::program_image::Version>>) -> Result<Arc<crate::code_rule_dsl::logic::program_image::Version>> {
    let store = if cfg!(feature = "tape-cache") && (programs.len() > 1 || read.is_some()) && std::env::var_os("RERAIL_DUMP_PROGRAMS").is_none() {
        phases::reading_store()
    } else {
        None
    };
    let Some(store) = store else { return combine_tail(programs, extra, read, None).map(|(v, _)| v) };
    let _reading = store.hold_collection().map_err(|e| Error::Plan(e.to_string()))?;
    let found = {
        let _lookup = crate::code_rule_dsl::logic::profile::span(|| "the combined program, from the store (decision 0094 §6)".into());
        store.settle(&combined::queued(key));
        combined::lookup_version(&store, key).map_err(|e| Error::Plan(e.to_string()))?
    };
    if let Some(v) = found {
        crate::code_rule_dsl::prof::count("corpus.combined_stored", 1);
        crate::code_rule_dsl::logic::profile::decision("combined program (decision 0094 §6)", || "read from the store: the same texts, and every library file they read reads the same".into());
        return Ok(v);
    }
    let mut images = Vec::new();
    let (made, read) = crate::code_rule_dsl::logic::rr::recording(|| combine_tail(programs, extra, read, Some(&mut images)));
    let (v, run) = made?;
    {
        let _keep = crate::code_rule_dsl::logic::profile::span(|| "keep the combined program in the store, queued".into());
        // Its image, as data, written by the store's writer (decisions 0102, 0306), naming the tail run it was made
        // over or kept: a run that reads the entry names that run as the one that made it did (Q101).
        combined::record_version_ran(&store, "the combined program", key, &read, &v, &images, run);
    }
    crate::code_rule_dsl::logic::profile::decision("combined program (decision 0094 §6)", || format!("combined; kept in the store behind the run, with the {} library files it read", read.len()));
    Ok(v)
}

/// The digest of `combine`'s texts and the rows it adds: what a combined program is a function of. `graphs`: each
/// text's include graph's digest (`Texts::graphs`).
/// `programs` are the texts with their messages masked (`masked`): a finding's message by its placeholders alone, its
/// words the writer's, not the program's (decision 0225 §3), so an edit of them finds the program combined, prepared
/// and memoised.
pub fn texts_digest(programs: &[&str], graphs: &[Option<[u8; 32]>], extra: &added::Added) -> [u8; 32] {
    use sha2::Digest as _;
    let mut hash = sha2::Sha256::new();
    // Under another value of a switch `combine_tail` reads, the same texts combine to another program (`COMBINING`).
    // Only those set: an unset switch keeps the key every run has had.
    for switch in COMBINING {
        if let Some(v) = std::env::var_os(switch) {
            hash.update(format!("\0{switch}={}", v.to_string_lossy()).as_bytes());
        }
    }
    debug_assert_eq!(programs.len(), graphs.len(), "a graph digest per program text");
    for (text, graph) in programs.iter().copied().zip(graphs.iter().copied()) {
        hash.update((text.len() as u64).to_le_bytes());
        hash.update(text.as_bytes());
        // What the text includes, as phase 0 found it: its modules' paths and digests, and the graph's edges. A text
        // is a function of these, not of its `#include` lines alone.
        if let Some(graph) = graph {
            hash.update(graph);
        }
    }
    // The rows the run adds, none for most runs (`added`).
    if !extra.is_empty() {
        extra.hash(&mut hash);
    }
    hash.finalize().into()
}

/// What the combine does to the linked program, on the whole (decision 0281's path, step 7a-1): one program's
/// relations that are one merged; several programs' projected inputs, helpers retyped, attributes on request, the
/// bound once where the libraries are held once, the fact spaces, and the relations that are one merged.
#[derive(Debug)]
pub enum Tail {
    One,
    /// `kept`: the kept link's family (decision 0373), where the link is kept: the tail's run is kept under it too
    /// (decision 0393).
    Many { hide: Option<u32>, policies: Vec<scoping::Policy>, kept: Option<TailKept> },
}

impl Tail {
    /// What the tail does, without where its run is kept: two tails of one shape make one program.
    fn shape(&self) -> String {
        match self {
            Tail::One => "one".to_owned(),
            Tail::Many { hide, policies, .. } => format!("{hide:?} {policies:?}"),
        }
    }
}

/// The family the combine's tail run is kept under (decision 0393): the kept link's (decision 0373).
#[derive(Clone, Copy)]
pub struct TailKept([u8; 32]);

impl std::fmt::Debug for TailKept {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("kept")
    }
}

/// The fact spaces' decision (decision 0091).
pub fn fact_spaces_decision(report: &scoping::Report) {
    crate::code_rule_dsl::logic::profile::decision("fact spaces", || {
        let guarded: Vec<String> = report.guarded.iter().map(|(p, n)| format!("{p}: {n}")).collect();
        format!(
            "{} global relations one copy for the run ({} policies' copies), and {} copies of engine relations; {} library relations a policy changed, its own{}; guards on the reads of the policies narrower than the run: {} (decision 0091)",
            report.canonical.len(),
            report.canonical.values().sum::<usize>(),
            report.engine,
            report.demoted.len(),
            if report.demoted.is_empty() { String::new() } else { format!(" ({})", report.demoted.iter().take(6).cloned().collect::<Vec<_>>().join(", ")) },
            if guarded.is_empty() { "none".to_owned() } else { guarded.join(", ") },
        )
    });
}

/// What the combine does to the linked program, on the version. Of one program, the requests asked where the
/// documents are, and the relations that are one merged. Of several, first `kind` and `docof` no inputs where a part
/// projects them, then each step below.
pub fn tail_version(v: Arc<crate::code_rule_dsl::logic::program_image::Version>, tail: &Tail) -> Result<Arc<crate::code_rule_dsl::logic::program_image::Version>> {
    let p = tail_passes(crate::code_rule_dsl::logic::program_image::manager::Pipeline::new("combine", v), tail)?;
    Ok(p.finish().map_err(Error::Plan)?.0)
}

/// `tail_version` as a run kept under the kept link's family (decision 0393): the last run's opened from the image
/// store, and where the link is an edit of its input (`manager::rebase`), the passes run again over the edit
/// (`Pipeline::again`): each step's analyses carried across it, and a pass that can (`ProgramPass::again`) writing
/// the edit onto its last output. The run is kept again, behind the run.
/// `used`: where the pointer to the tail's run (`keep_facts`) is named, read or kept, for the combined program's entry
/// to name it (`combined::image_sources`): a run that reads the entry keeps it as the run that combined did.
pub fn tail_kept(v: Arc<crate::code_rule_dsl::logic::program_image::Version>, tail: &Tail, kept: TailKept, used: Option<&mut Vec<crate::content_store::HashKey>>) -> Result<(Arc<crate::code_rule_dsl::logic::program_image::Version>, Option<[u8; 32]>)> {
    use crate::code_rule_dsl::logic::program_image::manager::{Delta, Pipeline, Runs as _, rebase};
    let Some(store) = crate::code_rule_dsl::logic::rr::image_store() else { return Ok((tail_version(v, tail)?, None)) };
    let key = {
        use sha2::Digest as _;
        sha2::Sha256::new()
            .chain_update(b"rerail/combine tail run v2\0")
            .chain_update(kept.0)
            .chain_update(crate::code_rule_dsl::build_ids::engine().as_bytes())
            .chain_update(tail.shape().as_bytes())
            .finalize()
            .into()
    };
    let runs = crate::code_rule_dsl::logic::program_text::StoredRuns(&store);
    // The run the key names (decision 0385): kept under its own input's identity (`anchor`), the key a replaceable
    // pointer to it, so a run kept later replaces it where a run's own facts are kept once.
    let anchor = |run: &crate::code_rule_dsl::logic::program_image::manager::Run| -> [u8; 32] {
        use sha2::Digest as _;
        sha2::Sha256::new().chain_update(b"rerail/combine tail run kept\0").chain_update(key).chain_update(crate::code_rule_dsl::logic::program_image::manager::identity(run.input())).finalize().into()
    };
    let last = {
        let _s = crate::code_rule_dsl::logic::profile::span(|| "the combine's last tail run, from the store (decision 0393)".into());
        if let Some(used) = used {
            used.push(preparation::parts::kept_facts_hash(&key));
        }
        let named = preparation::parts::kept_facts_alone(&store, &key).and_then(|(f, _)| f.get("anchor").and_then(|r| r.first()).and_then(|r| match r.first() {
            Some(front::Value::Str(h)) => preparation::parts::unhex(h),
            _ => None,
        }));
        // Its versions a few edits deep made flat (decision 0385): the edit runs over an edit, not whole.
        named.and_then(|a| runs.open(&a, "combine").ok().flatten().map(|r| (a, crate::code_rule_dsl::logic::program_image::manager::Run::shallow(r))))
    };
    let rebased = {
        let _s = crate::code_rule_dsl::logic::profile::span(|| "the link, as an edit of the last tail run's input (decision 0393)".into());
        last.and_then(|(a, last)| rebase(last.input(), &v).ok().flatten().map(|r| (a, last, r)))
    };
    let (p, edit, over) = match rebased {
        Some((a, last, r)) => {
            let n = Delta::of(last.input(), &r).size();
            crate::code_rule_dsl::logic::profile::decision("the combine's tail (decision 0393)", || format!("run again over an edit of the last run's input ({n} rules)"));
            (Pipeline::again(last, r), Some(n), Some(a))
        }
        None => {
            crate::code_rule_dsl::logic::profile::decision("the combine's tail (decision 0393)", || "run whole: no last run of the link's family, or the link is no edit of its input".into());
            (Pipeline::keeping("combine", v), None, None)
        }
    };
    let run = tail_passes(p, tail)?.done().map_err(Error::Plan)?;
    for s in run.stages() {
        crate::code_rule_dsl::logic::profile::decision("the combine's tail: a pass (decision 0393)", || format!("{}: {:?}, {:?}, {} rules visited", s.pass, s.ran, s.form, s.visited));
    }
    // Kept where it pays (decision 0385): a run made whole, or one whose edit from the kept run's input reached
    // `TAIL_REKEPT` rules. Otherwise the kept run stands, and the next edit is an edit of its input as this one was:
    // nothing is written.
    // The run this one was made over (it stands) or the one kept here: the combined program's entry names it.
    let named = match edit {
        Some(n) if n < TAIL_REKEPT => {
            crate::code_rule_dsl::logic::profile::decision("the combine's tail run, kept (decision 0385)", || format!("no: the kept run stands, the edit from its input {n} rules (kept again from {TAIL_REKEPT})"));
            over
        }
        _ => {
            let _s = crate::code_rule_dsl::logic::profile::span(|| "keep the combine's tail run (decision 0393)".into());
            let at = anchor(&run);
            crate::code_rule_dsl::logic::program_text::record_runs(&store, &[(at, &run)]);
            let pointer: crate::code_rule_dsl::logic::program_facts::Facts = std::iter::once(("anchor".to_owned(), vec![vec![preparation::parts::str(&preparation::parts::hex(&at))].into()])).collect();
            preparation::parts::keep_facts(&store, &key, &pointer);
            crate::code_rule_dsl::logic::profile::decision("the combine's tail run, kept (decision 0385)", || match edit {
                Some(n) => format!("yes: the edit from the kept run's input reached {n} rules"),
                None => "yes: a run made whole".into(),
            });
            Some(at)
        }
    };
    let out = run.output().clone();
    let out = match out.base_bytes() {
        Some(_) => out,
        None => out.fold().map_err(Error::Plan)?,
    };
    Ok((out, named))
}

/// How many rules an edit of the kept tail run's input reaches before the run over it is kept in its place
/// (decision 0385): below it, each edit runs again from the kept run, and keeping costs nothing.
pub const TAIL_REKEPT: usize = 64;

/// Whether `rules` (each rule's head and the relations it reads) read a guest relation (`guests::READS`).
pub fn reads_guests(rules: Vec<(String, Vec<String>)>) -> bool {
    rules.iter().any(|(_, reads)| reads.iter().any(|r| guests::READS.contains(&r.as_str())))
}

/// The tail's passes on `p` (`tail_version`), their decisions stated.
pub fn tail_passes<'s>(mut p: crate::code_rule_dsl::logic::program_image::manager::Pipeline<'s>, tail: &Tail) -> Result<crate::code_rule_dsl::logic::program_image::manager::Pipeline<'s>> {
    use preparation::{combining, hide, on_request, whole};
    let mut spaces_report = None;
    if let Tail::Many { hide: bound, policies, .. } = tail {
        // core.rerail's `kind(n, k)` and `docof(n, d)` are projections of `node` in a part that reads them (`rr::lower`'s
        // `projections`) and inputs in one that does not: once one part projects them, they are no inputs of the
        // combined program. Both would give `kind` two domains.
        p.program(&mut whole::Projected);
        // The helpers' columns, typed by the library's declarations, which each program was lowered without.
        p.program(&mut whole::RetypeHelpers);
        // The attributes on request over the combined program's readers (decision 0174; undone in each part).
        for round in crate::code_rule_dsl::logic::rr::lower::ON_REQUEST {
            p.program(&mut on_request::SourcesOnRequest { round });
        }
        // A request the cross-file part would derive asks within the node's document instead (O-cross-file-answers,
        // decision 0286); an answer still cross-file after that is made eager (0174 §8b).
        p = requests_where_documents_are(p)?;
        // The combined program bounded once to the documents it shows (decision 0058 §1, 0089 §4): a library's rules
        // are the same for every program, so one analysis gives what each program's would.
        if let Some(bound) = *bound {
            let rules = p.current().map_err(Error::Plan)?.size();
            let _hiding = crate::code_rule_dsl::logic::profile::span(|| format!("bound the combined program to the documents it shows ({rules} rules; decision 0058)"));
            p.program(&mut hide::Hide::new(bound));
        }
        // The library's relations once for the run, each policy's rules over its own scope's documents (decision 0091).
        let spacing = crate::code_rule_dsl::logic::profile::span(|| "fact spaces".into());
        let mut spaces = combining::FactSpaces::new(policies);
        p.program(&mut spaces);
        drop(spacing);
        spaces_report = Some(spaces.report);
    }
    if matches!(tail, Tail::One) {
        p = requests_where_documents_are(p)?;
    }
    p.current().map_err(Error::Plan)?;
    if let Some(report) = &spaces_report {
        fact_spaces_decision(report);
    }
    combining::same_relations(&mut p).map_err(Error::Plan)?;
    for d in p.decisions() {
        crate::code_rule_dsl::logic::profile::decision(&d.what, || d.detail.clone());
    }
    Ok(p)
}

/// The requests `on_request::SourcesOnRequest` made, narrowed where the split finds them cross-file to the asked node's own
/// document (`file_local::RequestsFileLocal`): a node's attribute is its document's, answered in its chunk while the
/// chunk's batch holds the text (O-cross-file-answers). Then the answers still cross-file made eager
/// (`answers_where_documents_are_version`). A program that answers no attribute on request is left as it is. Each a pass of `p`: a kept tail keeps them and the steps
/// after them (decision 0393).
pub fn requests_where_documents_are<'s>(mut p: crate::code_rule_dsl::logic::program_image::manager::Pipeline<'s>) -> Result<crate::code_rule_dsl::logic::program_image::manager::Pipeline<'s>> {
    use crate::code_rule_dsl::logic::rr::lower::ON_REQUEST;
    let v = std::sync::Arc::clone(p.current().map_err(Error::Plan)?);
    let asks = ON_REQUEST.iter().flat_map(|round| round.iter()).any(|a| v.rel(a.req).is_some());
    if !asks {
        return Ok(p);
    }
    if !std::env::var("RERAIL_NODE_SOURCES").is_ok_and(|v| v == "eager") {
        let requests: Vec<&'static str> = ON_REQUEST.iter().flat_map(|round| round.iter()).map(|a| a.req).filter(|r| v.rel(r).is_some()).collect();
        let mut pass = crate::code_rule_dsl::typing::file_local::RequestsFileLocal { requests, narrowed: Vec::new() };
        p.program(&mut pass);
        p.current().map_err(Error::Plan)?;
    }
    // In the same pipeline (decision 0393): a kept tail keeps the steps after this one too.
    p.program(&mut AnswersWhereDocumentsAre);
    Ok(p)
}

/// `answers_where_documents_are_version` as a program pass: the rounds it undoes, written as one edit of its input
/// (their headers and the heads they drop), and nothing where it undoes none.
pub struct AnswersWhereDocumentsAre;

impl crate::code_rule_dsl::logic::program_image::manager::ProgramPass for AnswersWhereDocumentsAre {
    const NAME: &'static str = "answers where documents are";
    /// Over an edit (decision 0385): the edit as it is, where the last run undid no round, the edit leaves the header
    /// as it was, and it defines no answer a round is undone for, nor what one reads, down.
    fn again(&mut self, cx: &crate::code_rule_dsl::logic::program_image::manager::Cx<'_>, last: &crate::code_rule_dsl::logic::program_image::manager::Last<'_>, w: &mut crate::code_rule_dsl::logic::program_image::manager::ProgramW<'_>) -> std::result::Result<bool, String> {
        use crate::code_rule_dsl::logic::program_image::manager::{Reads, edit_as_is};
        use crate::code_rule_dsl::logic::rr::lower::{Answer, ON_REQUEST};
        if last.delta.touched != Reads::RULES || !Arc::ptr_eq(last.input, last.output) {
            return Ok(false);
        }
        // The split places a relation from its rules and what they read, down: where the edit defines none of the
        // answers' relations nor what they read, it places them as it did, file-local, and is not asked for.
        let v = cx.v();
        let mut down: std::collections::BTreeSet<&str> = Default::default();
        let mut todo: Vec<&str> = ON_REQUEST.iter().flat_map(|round| round.iter()).filter(|a| matches!(a.answer, Answer::Function(_))).map(|a| a.rel).collect();
        while let Some(rel) = todo.pop() {
            if !down.insert(rel) {
                continue;
            }
            let Some(id) = v.rel(rel) else { continue };
            for r in v.heads(id) {
                todo.extend(r.body().filter_map(|l| l.atom()).map(|a| v.rel_name(a.rel())).filter(|x| !down.contains(x)));
            }
        }
        let head = |of: &crate::code_rule_dsl::logic::program_image::Version, r: crate::code_rule_dsl::logic::program_image::RuleRef<'_>| down.contains(of.rel_name(r.head().rel()));
        if last.delta.removed.iter().any(|id| last.input.rule(id).is_none_or(|r| head(last.input, r))) || last.delta.added.iter().any(|&id| v.rule(id).is_none_or(|r| head(v, r))) {
            return Ok(false);
        }
        edit_as_is(cx, last, w)
    }
    fn run(&mut self, _cx: &crate::code_rule_dsl::logic::program_image::manager::Cx<'_>, w: &mut crate::code_rule_dsl::logic::program_image::manager::ProgramW<'_>) -> std::result::Result<(), String> {
        let v = w.input().clone();
        let out = answers_where_documents_are_version(v.clone()).map_err(|e| e.to_string())?;
        if Arc::ptr_eq(&out, &v) {
            return Ok(());
        }
        let b = w.on()?;
        b.fields().state_fields_as(&v, &out);
        b.retain(|r| out.contains(r.id()));
        Ok(())
    }
}

/// The attributes answered on request by a function of the node's document (decision 0174: `value`, `node_text`)
/// whose answers are not file-local, made eager again: their rounds undone, so the projection makes them for every
/// node. A function's answer reads the node's document from the chunk's batch, which the cross-file part does not
/// hold: there `@rr_node_value` answered nothing, without an error, and every reader of `value` saw no value (the
/// Python pack: `__all__`'s and `exports`' readers ask across modules, so `value` went cross-file, and `1 if a else
/// 0`'s branches compared the same in RUF034). Until a cross-file answer reads its documents (0174 §6), an attribute a
/// cross-file reader asks for is eager. Each round whose function's answer a cross-file reader asks for is undone
/// (`rr::lower::undone_with` over `undo_round`), by the version's file-local split (`FileLocal`), to a fixpoint
/// (decision 0281's path, step 7b).
pub fn answers_where_documents_are_version(mut v: Arc<crate::code_rule_dsl::logic::program_image::Version>) -> Result<Arc<crate::code_rule_dsl::logic::program_image::Version>> {
    use crate::code_rule_dsl::logic::rr::lower::{Answer, ON_REQUEST, undo_round};
    loop {
        let split = crate::code_rule_dsl::logic::program_image::manager::analysis::<crate::code_rule_dsl::typing::file_local::FileLocal>(&v).map_err(Error::Plan)?;
        let declared = |rel: &str| v.declares(rel);
        let Some((round, rel)) = ON_REQUEST.iter().find_map(|round| round.iter().find(|a| matches!(a.answer, Answer::Function(_)) && declared(a.req) && !split.local.contains(a.rel)).map(|a| (*round, a.rel))) else {
            return Ok(v);
        };
        crate::code_rule_dsl::logic::profile::decision(&format!("{rel} on request"), || format!("no: its answer is not file-local (a cross-file reader asks), and the cross-file part holds no documents to read; {} eager (decision 0174 §6)", round.iter().map(|a| a.rel).collect::<Vec<_>>().join(", ")));
        v = crate::code_rule_dsl::logic::rr::lower::undone_with(&v, |head, rules| undo_round(head, rules, round)).map_err(Error::Plan)?;
    }
}

/// The programs `programs` linked into one (`link::Part`: versions, `link::Versioned`, decision 0281's path, step
/// 7a-2), and what the combine's tail does to that (`Tail`). `images_used`: where the aliases of the module and
/// program images the combine read or kept are added (decision 0246 §6), for the combined program's entry to name.
pub fn combine_linked_as<P: link::Part>(programs: &[&str], extra: &added::Added, read: Option<&Arc<crate::code_rule_dsl::logic::program_image::Version>>, images_used: Option<&mut Vec<crate::content_store::HashKey>>) -> Result<(P::Linked, Tail)> {
    combine_linked_spliced::<P>(programs, extra, read, images_used, combine_link_mode() != "whole")
}

/// `RERAIL_COMBINE_LINK`: `whole` links every program each run (no splice, decision 0373); `check` splices and links
/// whole beside it, and checks the two are one program with one tail.
pub fn combine_link_mode() -> String {
    std::env::var("RERAIL_COMBINE_LINK").unwrap_or_default()
}

/// `RERAIL_COMBINE_TAIL`: `whole` runs the combine's tail whole each run (no kept run, decision 0393); `check` runs it
/// kept and whole beside it, and checks the two are one program.
pub fn combine_tail_mode() -> String {
    std::env::var("RERAIL_COMBINE_TAIL").unwrap_or_default()
}

/// `combine_linked_as`, splicing into the last link where `splice` (decision 0373).
pub fn combine_linked_spliced<P: link::Part>(programs: &[&str], extra: &added::Added, read: Option<&Arc<crate::code_rule_dsl::logic::program_image::Version>>, images_used: Option<&mut Vec<crate::content_store::HashKey>>, splice: bool) -> Result<(P::Linked, Tail)> {
    // Each program is given the rows the run adds once it is read (decision 0249): rows, not text after it, stated on
    // its version. A program reads only relations it declares: an error here, not the planner's panic later
    // (`front::declared_on`).
    let given_version = |v: std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>| -> Result<std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>> {
        let mut b = crate::code_rule_dsl::logic::program_image::Builder::on(&v);
        extra.add_to(&mut b.fields());
        let v = if b.is_empty() { v } else { b.finish().map_err(Error::Plan)?.0 };
        front::declared_on(&v)?;
        Ok(v)
    };
    let parse = |text: &str| -> Result<P> { P::of_version(given_version(crate::code_rule_dsl::logic::rr::parse_version(text)?)?) };
    let parsing = crate::code_rule_dsl::logic::profile::span(|| format!("parse each program ({})", programs.len()));
    // Each text read once, for its modules and its program.
    let reading = crate::code_rule_dsl::logic::profile::span(|| "read each text (tape, statements, includes)".to_owned());
    // Each program read and lowered on the pool's threads: what each read from the library is noted here after, in
    // the programs' order (`rr::recording`, decision 0094 §6), and the first program's error in that order is the one
    // returned, as one thread gives them.
    use rayon::prelude::*;
    use crate::code_rule_dsl::logic::rr::{recorded_on_worker, replay};
    // The run's context on each job's thread (`run_state::Carry`): its observer and store, a test's own included.
    // Without it a read on the pool asked the process's store, not the test's.
    let carry = crate::code_rule_dsl::logic::run_state::Carry::here();
    let mut texts: Vec<Option<crate::code_rule_dsl::logic::rr::Text>> = if programs.len() > 1 {
        use crate::code_rule_dsl::logic::rr::{read_parsed, read_seed};
        let parsed: Vec<_> = crate::code_rule_dsl::logic::profile::on_cores(|| {
            programs
                .par_iter()
                .map(|text| {
                    let _in = carry.enter();
                    read_seed(text)
                })
                .collect()
        });
        // Every program's include graph by one run of phase 0, here, before the reads: each read on the pool then
        // finds its graph kept. Derived there, each was a nested run of its own on the run's store, beside the others
        // (`content store writer is busy`; a memo made under another atom table than its writer's).
        let seeds: Vec<_> = parsed.iter().filter_map(|p| p.as_ref().ok().and_then(|r| r.seed.clone())).collect();
        if !seeds.is_empty() {
            // An error is left to the read that meets it, which reports it in its program's place.
            let _ = crate::code_rule_dsl::logic::plan::corpus::phase0::graphs(&seeds);
        }
        let each: Vec<_> = crate::code_rule_dsl::logic::profile::on_cores(|| {
            programs
                .par_iter()
                .zip(parsed)
                .map(|(text, parsed)| {
                    let _in = carry.enter();
                    recorded_on_worker(|| parsed.and_then(|r| read_parsed(r.statements)).map(Some).map_err(|e| in_policy(text, e)))
                })
                .collect()
        });
        each.into_iter().map(|(got, read)| {
            replay(read);
            got
        }).collect::<std::result::Result<_, _>>()?
    } else {
        programs.iter().map(|_| None).collect()
    };
    // Each program's library modules (decision 0089), when it names them by `#include`.
    let modular: Vec<Option<crate::code_rule_dsl::logic::rr::Modules>> = if programs.len() > 1 {
        programs
            .iter()
            .zip(&texts)
            .map(|(text, read)| match read {
                Some(read) => Ok(read.modules()),
                None => crate::code_rule_dsl::logic::rr::modules(text),
            })
            .collect::<std::result::Result<_, _>>()?
    } else {
        vec![None]
    };
    let copied = std::env::var("RERAIL_LIBRARIES").is_ok_and(|v| v == "copied");
    let core = crate::code_rule_dsl::logic::rr::core_names();
    // A program that adds to no library relation is read with its libraries' declarations only: the run holds their
    // rules once (§5). One that asks (§3) is read whole, since its cone is copied from what it read.
    let asks = |modules: &crate::code_rule_dsl::logic::rr::Modules| {
        modules.heads.iter().any(|h| !core.contains(h) && modules.files.iter().any(|(_, _, names)| names.contains(h)))
    };
    let own_only: Vec<bool> = modular.iter().map(|m| !copied && m.as_ref().is_some_and(|m| !asks(m))).collect();
    drop(reading);
    // The run's library files, each once, includes first, and the library lowered from them, before the programs: a
    // program that asks nothing of it is lowered against its modules (decision 0246 §2), not with its declarations.
    let mut library_files: Vec<std::path::PathBuf> = Vec::new();
    let mut lowered_library: Option<std::sync::Arc<crate::code_rule_dsl::logic::rr::Library>> = None;
    // The store rule images are read from and kept in (decision 0264): the engine build's, dropped at the end of the
    // combine, which writes what it queued.
    let mut image_store: Option<std::sync::Arc<crate::content_store::ContentStore>> = None;
    if !copied && modular.iter().flatten().count() > 1 {
        let _library = crate::code_rule_dsl::logic::profile::span(|| "the libraries, once (decision 0089)".into());
        let mut declared_in: BTreeMap<&str, &std::path::Path> = BTreeMap::new();
        for modules in modular.iter().flatten() {
            for (path, _, names) in &modules.files {
                if library_files.contains(path) {
                    continue;
                }
                for name in names.iter().filter(|n| !core.contains(*n)) {
                    if let Some(other) = declared_in.insert(name, path) {
                        return Err(Error::Plan(format!("{name} is declared by two library files of the run, {} and {}", other.display(), path.display())));
                    }
                }
                library_files.push(path.clone());
            }
        }
        image_store = crate::code_rule_dsl::logic::rr::image_store();
        lowered_library = Some(crate::code_rule_dsl::logic::rr::lower_library(&library_files, image_store.as_deref())?);
    }
    let in_scope = lowered_library.as_deref().filter(|l| !l.modules.is_empty());
    // Each program lowered in scope is kept in the store as its image (decision 0246 §6).
    let images = image_store.filter(|_| in_scope.is_some());
    let lowering = crate::code_rule_dsl::logic::profile::span(|| "lower each program".to_owned());
    // Each distinct scope seeded once (decision 0246 §2), before the programs lowered in it.
    let mut scoped: Vec<crate::code_rule_dsl::logic::rr::Scoped> = Vec::new();
    let scope_of: Vec<Option<usize>> = match in_scope {
        Some(library) => texts.iter().map(|t| t.as_ref().and_then(|t| t.files())).map(|files| {
            let files = files?;
            let keys = library.scope(&files).1;
            Some(scoped.iter().position(|s| s.keys == keys).unwrap_or_else(|| {
                scoped.push(library.scoped(&files));
                scoped.len() - 1
            }))
        }).collect(),
        None => vec![None; texts.len()],
    };
    // The combine's last link (decision 0373): each program's slot is its image's key, and the run's shape the key the
    // link is kept under. Where it is the last run's, only the programs whose image changed are lowered.
    let per_program = std::env::var("RERAIL_REQUESTS").is_ok_and(|v| v == "copied");
    let keeping: Option<(Vec<[u8; 32]>, [u8; 32])> = (|| {
        let library = in_scope?;
        images.as_ref()?;
        if !splice || !P::KEEPS || programs.len() < 2 || read.is_some() || copied {
            return None;
        }
        let keys = programs.iter().zip(&scope_of).zip(&texts).map(|((text, k), t)| t.as_ref().and_then(|_| scoped[(*k)?].program_image(text).sha256_bytes())).collect::<Option<Vec<_>>>()?;
        Some((keys, kept_link::family(programs.len(), extra, &library.image_keys())))
    })();
    if let (Some((keys, family)), Some(store), Some(library)) = (&keeping, images.as_deref(), in_scope)
        && let Some(got) = combine_spliced::<P>(store, family, keys, programs, &mut texts, &scope_of, &scoped, library, &modular, extra, per_program)?
    {
        drop(lowering);
        if let Some(out) = images_used {
            out.extend(library.image_keys());
            out.extend(programs.iter().zip(&scope_of).filter_map(|(text, k)| Some(scoped[(*k)?].program_image(text))));
        }
        return Ok(got);
    }
    // Each program's kept image, read for all of them at once (a few statements, not three each from the pool's
    // threads), its lease renewed in one more.
    let mut prefetched: Vec<Option<Option<crate::code_rule_dsl::logic::rr::images::ImageFound>>> = (0..programs.len()).map(|_| None).collect();
    if let Some(store) = images.as_deref().filter(|_| in_scope.is_some() && !(read.is_some() && programs.len() == 1)) {
        let _s = crate::code_rule_dsl::logic::profile::span(|| "the programs' images, from the store".to_owned());
        let asked: Vec<(usize, crate::content_store::HashKey)> = programs.iter().zip(&scope_of).zip(&texts).enumerate().filter_map(|(i, ((text, k), t))| {
            let image = scoped[(*k)?].program_image(text);
            t.is_some().then_some((i, image))
        }).collect();
        let found = crate::code_rule_dsl::logic::rr::images::read_images(Some(store), &asked.iter().map(|(_, k)| *k).collect::<Vec<_>>());
        let ids: Vec<crate::content_store::SourceId> = found.iter().flatten().filter_map(|(_, id)| *id).collect();
        crate::code_rule_dsl::logic::rr::renew_images(store, &ids);
        for ((i, _), f) in asked.into_iter().zip(found) {
            prefetched[i] = Some(f);
        }
    }
    let each: Vec<_> = crate::code_rule_dsl::logic::profile::on_cores(|| {
        programs
            .par_iter()
            .zip(&own_only)
            .zip(texts.par_iter_mut())
            .zip(&scope_of)
            .zip(prefetched.into_par_iter())
            .map(|((((text, &own_only), slot), scope_of), found)| {
                let _in = carry.enter();
                recorded_on_worker(|| -> Result<P> {
                if let Some(read) = read.filter(|_| programs.len() == 1) {
                    return P::of_version(given_version(Arc::clone(read))?);
                }
                if let Some(read) = slot.take() {
                    if let Some(library) = in_scope && let Some(k) = *scope_of {
                        let _in_scope = crate::code_rule_dsl::logic::profile::span(|| "lower the program in its libraries' scope (decision 0246 §2)".to_owned());
                        // Its library's relations are the library's, declared there, not in the program: it is handed the
                        // run's rows (0249) but not checked for its own declarations (`lowering_tail::Declared`).
                        return P::lowered_in(read, text, library, &scoped[k], images.as_deref(), found, extra);
                    }
                    return P::of_version(given_version(read.lower_version(!own_only)?)?);
                }
                if !own_only {
                    return parse(text);
                }
                P::of_version(given_version(crate::code_rule_dsl::logic::rr::parse_own_version(text)?)?)
                })
            })
            .collect()
    });
    let mut parsed = each
        .into_iter()
        .map(|(got, read)| {
            replay(read);
            got
        })
        .collect::<std::result::Result<Vec<_>, Error>>()?;
    drop(lowering);
    if let (Some(out), Some(library), Some(_)) = (images_used, in_scope, images.as_ref()) {
        out.extend(library.image_keys());
        out.extend(programs.iter().zip(&scope_of).filter_map(|(text, k)| Some(scoped[(*k)?].program_image(text))));
    }
    // `RERAIL_DUMP_PROGRAMS=<file>`: each program's prefix (`p<i>_`) and its first policy or line (a debugging aid).
    if let Ok(path) = std::env::var("RERAIL_DUMP_PROGRAMS") {
        let lines: String = programs
            .iter()
            .enumerate()
            .map(|(i, text)| {
                let first = text.lines().find(|l| l.trim_start().starts_with(".policy")).or_else(|| text.lines().find(|l| !l.trim().is_empty())).unwrap_or("");
                format!("p{i}_\t{}\n", first.trim())
            })
            .collect();
        let _ = std::fs::write(path, lines);
    }
    drop(parsing);
    // A program makes guests where it asks for one, or reads a guest relation: a file's programs are its grammar's
    // (decision 0431 §4), made whatever a rule asks. And under `.embedded on`: it reads other languages' files, whose
    // trees are guests of theirs it is not shown (decision 0512).
    let guests_of: Vec<bool> = parsed.iter().map(|p| guests::REQUESTS.iter().any(|r| p.has_head(r)) || reads_guests(p.rules()) || embedded_on(p.head())).collect();
    let makes_guests = guests_of.iter().any(|&g| g);
    // The libraries once (decision 0089): two or more programs name theirs as modules. Every program is read under one
    // bound of the documents it shows (decision 0523), so the library's relations are the same for each.
    // `RERAIL_LIBRARIES=copied` copies them into each program, as before, to compare.
    let shared = !copied && modular.iter().flatten().count() > 1;
    // Not held once after all: a program read without its libraries' rules is read whole.
    if !shared {
        for ((prog, text), &own_only) in parsed.iter_mut().zip(programs).zip(&own_only) {
            if own_only || in_scope.is_some() {
                *prog = parse(text)?;
            }
        }
    }
    crate::code_rule_dsl::logic::profile::decision("libraries (decision 0089)", || {
        if shared {
            format!("held once: {} of {} programs name their libraries as modules", modular.iter().flatten().count(), programs.len())
        } else if copied {
            "copied into each program (RERAIL_LIBRARIES=copied)".to_owned()
        } else {
            "copied into each program: fewer than two programs name their libraries as modules".to_owned()
        }
    });
    let mut library: Option<P> = None;
    // A program's request (§3): its key's number and the relations it holds, named `q<k>_` for every asker.
    let mut requests: Vec<Option<(usize, BTreeSet<String>)>> = vec![None; parsed.len()];
    // Each program's request as its interface reads it (decision 0373): its key, number, and whether it asked first.
    let mut asked: Vec<Option<([u8; 32], usize, bool)>> = vec![None; parsed.len()];
    let mut first_askers: BTreeMap<[u8; 32], (usize, usize)> = BTreeMap::new();
    if shared {
        for (i, modules) in modular.iter().enumerate() {
            let Some(modules) = modules else { continue };
            for name in &modules.declared {
                if let Some(file) = modules.files.iter().find(|(_, _, names)| names.contains(name) && !core.contains(name)) {
                    return Err(Error::Plan(format!("program #{i} declares {name}, which {} it includes declares", file.0.display())));
                }
            }
        }
        library = Some(P::library(lowered_library.as_ref().expect("made with the programs' modules"))?);
        let mut cones = 0;
        let mut kept = 0;
        // Each program's request (§3): the cone's library relations and the program's own relations they read, held
        // once per distinct key. `RERAIL_REQUESTS=copied` keeps one copy per program, to compare.
        let mut keys: BTreeMap<[u8; 32], usize> = BTreeMap::new();
        let mut copies: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
        let mut asking = 0;
        for (i, (prog, modules)) in parsed.iter_mut().zip(&modular).enumerate() {
            let Some(modules) = modules else { continue };
            let (dropped, cone) = own_part(prog, modules, core)?;
            kept += dropped;
            cones += cone.len();
            if cone.is_empty() {
                continue;
            }
            asking += 1;
            let (unit, key) = request_unit(prog, &cone, core, i, per_program);
            let n = keys.len();
            let k = *keys.entry(key).or_insert(n);
            asked[i] = Some((key, k, k == n));
            if k == n {
                first_askers.insert(key, (k, i));
            }
            for rel in cone.iter() {
                copies.entry(rel.clone()).or_default().insert(k);
            }
            if k != n {
                // Another program asked the same: read its copy.
                drop_relations(prog, &unit)?;
            }
            requests[i] = Some((k, unit));
        }
        let rules = library.as_ref().map_or(0, |l| l.rule_count());
        crate::code_rule_dsl::logic::profile::decision("library relations", || {
            format!("{rules} library rules once; {kept} relations taken from the library instead of a program's copy; {cones} cone relations in {asking} programs' requests (§3)")
        });
        crate::code_rule_dsl::logic::profile::decision("requests (decision 0089 §3)", || {
            let most: Vec<String> = {
                let mut by: Vec<(&String, usize)> = copies.iter().map(|(r, ks)| (r, ks.len())).collect();
                by.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
                by.iter().take(8).map(|(r, n)| format!("{r} {n}")).collect()
            };
            format!("{} distinct requests of {asking} asking programs{}; copies per relation: {}", keys.len(), if per_program { " (RERAIL_REQUESTS=copied)" } else { "" }, most.join(", "))
        });
    }
    // Programs combined have their node attributes answered on request over the combined program (below), each
    // part's own requests undone first: before hiding, which copies a relation's rules into its grammar-scoped copies
    // (`rr_in_<g>_token`). Undone after it, the copies kept the answer's rules, reading the request's inputs
    // (`rr_token_anon`) that the undo no longer declared, and the combined program had nothing to answer them (Q67,
    // the data-layer pack).
    if parsed.len() > 1 {
        for p in parsed.iter_mut() {
            p.undo_on_request()?;
        }
    }
    // A run that makes guests bounds each program's rules to the documents it shows (decision 0058 §1); a run that
    // makes none is left as written. With the libraries once, the combined program is bounded once (below).
    if makes_guests && !shared {
        let _hiding = crate::code_rule_dsl::logic::profile::span(|| {
            format!("bound each program to the documents it shows ({} programs, {} rules; decision 0058)", parsed.len(), parsed.iter().map(|p| p.rule_count()).sum::<usize>())
        });
        for (i, p) in parsed.iter_mut().enumerate() {
            let _one = crate::code_rule_dsl::logic::profile::span(|| format!("program #{i} ({} rules)", p.rule_count()));
            p.hide(hiding::BOUND)?;
        }
    }
    let _merging = crate::code_rule_dsl::logic::profile::span(|| "merge the programs".into());
    if parsed.len() == 1 {
        let one = parsed.pop().expect("one program");
        return Ok((one.alone()?, Tail::One));
    }
    let mut all = link::Merged::new(P::fields());
    let mut sets: BTreeMap<Vec<String>, String> = BTreeMap::new();
    // The library part's names for its kind sets, as the merge renamed them.
    let mut library_sets: BTreeMap<String, String> = BTreeMap::new();
    let core = crate::code_rule_dsl::logic::rr::core_names();
    let mut policies: Vec<scoping::Policy> = Vec::new();
    // A library's rule may ask for guests or read them too: read once, it is no program's.
    let makes_guests = makes_guests || library.as_ref().is_some_and(|l| guests::REQUESTS.iter().any(|r| l.has_head(r)) || reads_guests(l.rules()));
    // The node attributes answered on request (decision 0174) are the combined program's, made once below over all
    // its readers: a part lowered on its own (a program's own part, without the library's rules) could not see
    // which library relations read an attribute, so its requests read them as they are, not through their widened
    // copies, and a request was recursive with its attribute (PYI062 with PYI059: `literal_down` negates
    // `typing_subscript`, which reads `value` through `exports`, and `rr_value_req` read `literal_leaf`).
    if let Some(l) = library.as_mut() {
        l.undo_on_request()?;
    }
    // Each part's own requests were undone before hiding (above).
    // The library, when held once, is the first part: its names are the run's (decision 0089 §2).
    let units = library.into_iter().map(|l| (None, l)).chain(parsed.into_iter().enumerate().map(|(i, p)| (Some(i), p)));
    let mut rules: Vec<P::Rules> = Vec::new();
    let mut interfaces: Vec<kept_link::Interface> = Vec::new();
    for (unit, part) in units {
        let i = unit.unwrap_or(usize::MAX);
        let prog = part.head();
        let scope = prog.doc_scopes();
        let (named, library) = front::globals(&prog.settings());
        // The documents read: those some policy's scope selects; every one when a policy has none.
        all.doc_scopes = match (i, all.doc_scopes.take(), scope.clone()) {
            (0, _, mine) => mine,
            (_, Some(mut a), Some(b)) => {
                a.extend(b);
                Some(a)
            }
            _ => None,
        };
        let globs = prog.text_globs();
        for g in &globs {
            if !all.text_globs.contains(g) {
                all.text_globs.push(g.clone());
            }
        }
        // Kind sets by their kinds: this program's name for each, and the shared one. A set the program reads but
        // does not hold (a library rule its request copied, `rr::with_cone_version`) is the library's: named as the
        // library part names it, not left on a name the merge renamed (PR #508's afeb076e5, t47).
        let mut set_of: BTreeMap<String, String> = if unit.is_some() { library_sets.clone() } else { BTreeMap::new() };
        let kinds_of = prog.kindsets();
        for (name, kinds) in kinds_of.clone() {
            let n = sets.len();
            let shared = sets.entry(kinds.clone()).or_insert_with(|| format!("ks{n}")).clone();
            all.kindsets.insert(shared.clone(), kinds);
            set_of.insert(name, shared);
        }
        if unit.is_none() {
            library_sets = set_of.clone();
        }
        // The library's relations keep their names (decision 0089 §2). Named repository readers belong to their
        // policy (`own_names`): equal relation names alone do not mean equal input files or selectors.
        let own: BTreeSet<String> = if unit.is_some() { own_names(&part, core) } else { BTreeSet::new() };
        let request = unit.and_then(|u| requests[u].clone());
        // The library held once is no policy's: canonical by construction, made over every document read (0091 §3).
        if unit.is_some() {
            policies.push(scoping::Policy { prefix: format!("p{i}_"), own: own.iter().map(|r| format!("p{i}_{r}")).collect(), named, library, scope });
        }
        let core_rules = if keeping.is_some() && unit.is_some() { part.shown(core) } else { Vec::new() };
        let names = link::Names { i, own, request, set_of };
        let (header, renamed) = part.into_parts(&names);
        if keeping.is_some() && let Some(u) = unit {
            interfaces.push(kept_link::interface(&[&u, &hiding::BOUND, &guests_of[u], &asked[u], &policies.last(), &globs, &kinds_of, &names.set_of, &names.own, &names.request, &crate::code_rule_dsl::logic::program_image::fields::Shown(P::header_fields(&header)), &core_rules]));
        }
        rules.push(renamed);
        all.merge(P::header_fields(&header), &names);
    }
    let (all, counts) = P::link_counted(all.finish(), rules, core)?;
    let hide = (shared && makes_guests).then_some(hiding::BOUND);
    // The link kept for the next run's splice (decision 0373), where the libraries are held once.
    let mut kept = None;
    if let (Some((keys, family)), Some(store), Some(counts), Some(v), true) = (&keeping, images.as_deref(), counts, P::linked_version(&all), shared && interfaces.len() == programs.len()) {
        let slots = keys.iter().zip(&interfaces).map(|(k, f)| kept_link::Slot { key: *k, interface: f.clone() }).collect();
        kept_link::record(store, family, v, &kept_link::Record { counts, slots, requests: first_askers, sets, library_sets, bound: hiding::BOUND, hide, policies: policies.clone() });
        kept = Some(TailKept(*family));
    }
    Ok((all, Tail::Many { hide, policies, kept }))
}

/// `combine_linked_as` as a splice into the combine's last link (decision 0373): the programs whose image changed
/// lowered and read as the merge reads each (`own_part`, the request, `undone_on_request`, the header's kind sets,
/// globals and policy), and where each one's interface is its slot's, its rules written in its slot's place and every
/// other program's copied from the last link (`Part::spliced`). `None` where nothing is kept for the run's shape, or a
/// program's interface changed: the programs are then linked whole, and the link kept again.
#[allow(clippy::too_many_arguments)]
pub fn combine_spliced<P: link::Part>(
    store: &ContentStore,
    family: &[u8; 32],
    keys: &[[u8; 32]],
    programs: &[&str],
    texts: &mut [Option<crate::code_rule_dsl::logic::rr::Text>],
    scope_of: &[Option<usize>],
    scoped: &[crate::code_rule_dsl::logic::rr::Scoped],
    library: &crate::code_rule_dsl::logic::rr::Library,
    modular: &[Option<crate::code_rule_dsl::logic::rr::Modules>],
    extra: &added::Added,
    per_program: bool,
) -> Result<Option<(P::Linked, Tail)>> {
    let Some((last, mut rec, sources)) = kept_link::lookup(store, family) else { return Ok(None) };
    // Kept as the programs' images are, in the same store: its lease renewed where a run reads it (decision 0250).
    crate::code_rule_dsl::logic::rr::renew_images(store, &sources);
    // The header's rules, the library's, then each program's.
    if rec.slots.len() != programs.len() || rec.counts.len() != programs.len() + 2 {
        return Ok(None);
    }
    let changed: Vec<usize> = (0..programs.len()).filter(|&i| rec.slots[i].key != keys[i]).collect();
    let _span = crate::code_rule_dsl::logic::profile::span(|| format!("the changed programs spliced into the combine's last link ({} of {}; decision 0373)", changed.len(), programs.len()));
    let core = crate::code_rule_dsl::logic::rr::core_names();
    let mut parts: Vec<(usize, P::Rules)> = Vec::new();
    for &i in &changed {
        let (Some(read), Some(k), Some(modules)) = (texts[i].take(), scope_of[i], modular[i].as_ref()) else { return Ok(None) };
        let mut part = P::lowered_in(read, programs[i], library, &scoped[k], Some(store), None, extra)?;
        if hiding::BOUND != rec.bound {
            return Ok(None);
        }
        let guests = guests::REQUESTS.iter().any(|r| part.has_head(r)) || reads_guests(part.rules());
        for name in &modules.declared {
            if let Some(file) = modules.files.iter().find(|(_, _, names)| names.contains(name) && !core.contains(name)) {
                return Err(Error::Plan(format!("program #{i} declares {name}, which {} it includes declares", file.0.display())));
            }
        }
        let (_, cone) = own_part(&mut part, modules, core)?;
        let (mut request, mut asked) = (None, None);
        if !cone.is_empty() {
            let (unit, key) = request_unit(&part, &cone, core, i, per_program);
            // A request no earlier program asks first would be numbered, and copied, otherwise.
            let Some(&(k, first)) = rec.requests.get(&key).filter(|(_, first)| *first <= i) else { return Ok(None) };
            if first != i {
                drop_relations(&mut part, &unit)?;
            }
            asked = Some((key, k, first == i));
            request = Some((k, unit));
        }
        part.undo_on_request()?;
        let prog = part.head();
        let scope = prog.doc_scopes();
        let (named, library_names) = front::globals(&prog.settings());
        let globs = prog.text_globs();
        let kinds_of = prog.kindsets();
        // From the library's sets, as the link names them (`combine_linked_as`).
        let mut set_of: BTreeMap<String, String> = rec.library_sets.clone();
        for (name, kinds) in &kinds_of {
            let Some(shared) = rec.sets.get(kinds) else { return Ok(None) };
            set_of.insert(name.clone(), shared.clone());
        }
        let own = own_names(&part, core);
        let policy = scoping::Policy { prefix: format!("p{i}_"), own: own.iter().map(|r| format!("p{i}_{r}")).collect(), named, library: library_names, scope };
        let core_rules = part.shown(core);
        let names = link::Names { i, own, request, set_of };
        let (header, renamed) = part.into_parts(&names);
        let interface = kept_link::interface(&[&i, &hiding::BOUND, &guests, &asked, &Some(&policy), &globs, &kinds_of, &names.set_of, &names.own, &names.request, &crate::code_rule_dsl::logic::program_image::fields::Shown(P::header_fields(&header)), &core_rules]);
        if interface != rec.slots[i].interface {
            crate::code_rule_dsl::logic::profile::decision("the combine's last link (decision 0373)", || format!("linked again: program #{i}'s interface changed ({})", interface.changed(&rec.slots[i].interface).join(", ")));
            return Ok(None);
        }
        rec.slots[i].key = keys[i];
        parts.push((2 + i, renamed));
    }
    let Some((linked, counts)) = P::spliced(&last, &rec.counts, parts, core)? else { return Ok(None) };
    crate::code_rule_dsl::logic::profile::decision("the combine's last link (decision 0373)", || format!("spliced: {} of {} programs changed, each of the same interface", changed.len(), programs.len()));
    if !changed.is_empty()
        && let Some(v) = P::linked_version(&linked)
    {
        rec.counts = counts;
        kept_link::record(store, family, v, &rec);
    }
    Ok(Some((linked, Tail::Many { hide: rec.hide, policies: rec.policies, kept: Some(TailKept(*family)) })))
}

/// What the converter gives an authored file to read (`logic::convert`, test-only): the scope, the report gate, the
/// options, and the tape's syntax verdict (`tape_syntax_error(d)`: core.rerail's `syntax_error` is the engine's, decision
/// 0141). Here so the runner, which is not test-only, can tell them from unbound core relations.
pub const GIVEN: [&str; 8] = ["in_scope", "out_scope", "report_when_gt", "option_on", "option_item", "option_value", "option_json", "tape_syntax_error"];

/// Fails, naming it, where `prog` reads one of core.rerail's relations that nothing derives and the runner does not bind
/// (`value`, `str_part`, …): the planner would otherwise fail on its missing statistics (`no statistics for
/// value.c0`). A program's own inputs are its business (facts it is given later), so are the facts the converter
/// gives (`GIVEN`: core.rerail's `option_item` among them, empty for an option of no items), and the relations not made
/// yet are empty (`NOT_MADE`, O76).
pub fn bound(v: &crate::code_rule_dsl::logic::program_image::Version) -> std::result::Result<(), String> {
    let derived = |rel: &str| v.rel(rel).is_some_and(|r| v.heads(r).next().is_some()) || v.find(rel).and_then(|a| v.facts(a)).is_some_and(|f| !f.about());
    let cpython = "CPython (core.rerail: the isolated `ast.parse` worker), which the corpus runner does not run";
    // `env` is the run's settings (`tools::with_settings`), and `tool_call` and `tool_input` the tool calls it is given
    // (`corpus::added`, `rr::lower::TOOL_CALLS`): facts when there are any, else empty. The world's relations
    // (`dir_entry`, `file_line`, …, and `loaded`) are answered by the runner (`world`).
    let world = |rel: &str| rel == world::LOADED || crate::code_rule_dsl::logic::rr::WORLD.iter().any(|(r, _)| *r == rel);
    let given = |rel: &str| GIVEN.contains(&rel) || git::INPUTS.contains(&rel) || analysis_history::is_input(rel)
        || matches!(rel, "env" | "source_grammar_suffix" | "source_grammar_registry" | "source_tape_version")
        || crate::code_rule_dsl::logic::rr::lower::TOOL_CALLS.contains(&rel)
        || world(rel) || NOT_MADE.iter().any(|(r, _)| *r == rel);
    // core.rerail's literal parts are made in core.rerail's shape only (`syntax::core_rr`): a program that declares its base
    // relations itself (T's shape, `text_demand::projection_of`: not every relation core.rerail declares is declared) has none.
    let core = crate::code_rule_dsl::facts::core::core_names();
    let t_shape = core.is_empty() || !core.iter().all(|r| v.decl_of(r.as_str()).is_some());
    let parts = |rel: &str| t_shape && rel.starts_with("str_part");
    for rule in v.rules() {
        for a in rule.body().filter_map(|l| l.atom()) {
            let rel = v.rel_name(a.rel());
            if !parts(rel) && (derived(rel) || super::super::super::sql::select::input_columns(rel).is_some() || !crate::code_rule_dsl::logic::rr::is_core(rel) || given(rel)) {
                continue;
            }
            let why = match rel {
                _ if parts(rel) => format!("core.rerail's shape makes it (a program that includes core.rerail); without it, it needs {cpython}"),
                "list_len" => "it counts a pattern's list variable ($$$X), and patterns do not lower".to_owned(),
                _ => "the corpus runner has no producer of it (the relations it binds: candidates/datalog/authored/README.md)".to_owned(),
            };
            return Err(format!("{rel} is read, and nothing derives it or binds it: {why}"));
        }
    }
    Ok(())
}

/// Of a program whose text names its libraries as modules (decision 0089 §5), the part the run's library does not
/// hold, taken out of `prog`: what is left is the program's own. A library relation goes to the library unless the
/// program's own clauses add to it or it reads one they add to, through any relation (the request's cone, §3): the
/// program keeps its own copy of those, as before. A relation lowering made for library rules alone (every reader
/// of it the library's) goes with them. Returns the relations taken out, and the cone's.
pub fn own_part<P: link::Part>(of: &mut P, modules: &crate::code_rule_dsl::logic::rr::Modules, core: &BTreeSet<String>) -> Result<(usize, BTreeSet<String>)> {
    let rules = of.rules();
    let prog = of.head();
    let inputs = prog.input_names();
    let natives = prog.natives();
    let declared = prog.declared_names();
    // An input the program reads (`within`, which lowering keeps where a rule reads it) is a base relation, the run's:
    // its declaration stays with the program.
    let library: BTreeSet<&str> =
        modules.files.iter().flat_map(|(_, _, names)| names).map(String::as_str).filter(|r| !core.contains(*r) && !inputs.iter().any(|i| i == r)).collect();
    let edges: Vec<(String, Vec<String>)> = rules
        .iter()
        .cloned()
        .chain(natives.iter().map(|(rel, n)| (rel.to_string(), n.reads().into_iter().map(str::to_owned).collect())))
        .collect();
    let mut cone: BTreeSet<String> = modules.heads.iter().filter(|h| library.contains(h.as_str())).cloned().collect();
    loop {
        let before = cone.len();
        for (head, body) in &edges {
            if !cone.contains(head) && body.iter().any(|b| cone.contains(b)) {
                cone.insert(head.clone());
            }
        }
        if cone.len() == before {
            break;
        }
    }
    let mut readers: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (head, body) in &edges {
        for b in body {
            readers.entry(b.as_str()).or_default().insert(head.as_str());
        }
    }
    let mut part: BTreeSet<String> = library.iter().filter(|r| !cone.contains(**r)).map(|r| (*r).to_owned()).collect();
    let candidates: BTreeSet<String> = declared
        .iter()
        .map(String::as_str)
        .chain(rules.iter().map(|(head, _)| head.as_str()))
        .chain(natives.keys().map(String::as_str))
        .filter(|r| !library.contains(*r) && !core.contains(*r) && !cone.contains(*r))
        .map(str::to_owned)
        .collect();
    loop {
        let before = part.len();
        for c in &candidates {
            if !part.contains(c) && readers.get(c.as_str()).is_some_and(|rs| !rs.is_empty() && rs.iter().all(|h| part.contains(*h))) {
                part.insert(c.clone());
            }
        }
        if part.len() == before {
            break;
        }
    }
    let taken = part.iter().filter(|r| declared.contains(*r) || rules.iter().any(|(head, _)| head == *r)).count();
    let cones = cone.into_iter().filter(|r| library.contains(r.as_str())).collect();
    drop_relations(of, &part)?;
    Ok((taken, cones))
}

/// `relations` taken out of `part`: their rules, declarations, rows, natives, lattices and nullary marks.
pub fn drop_relations<P: link::Part>(part: &mut P, relations: &BTreeSet<String>) -> Result<()> {
    use crate::code_rule_dsl::logic::program_facts::Field;
    part.drop_heads(&|head| relations.contains(head));
    part.edit_head(&mut |prog| {
        for r in prog.declared_names() {
            if relations.contains(&r) {
                prog.undeclare(&r);
            }
        }
        for r in prog.fact_names() {
            if relations.contains(&r) {
                prog.drop_facts(&r);
            }
        }
        let natives = prog.natives();
        if natives.keys().any(|r| relations.contains(r)) {
            let kept: BTreeMap<_, _> = natives.into_iter().filter(|(r, _)| !relations.contains(r)).collect();
            prog.set_field(Field::Natives(&kept));
        }
        let lattice = prog.lattice();
        if lattice.keys().any(|r| relations.contains(r)) {
            let kept: BTreeMap<_, _> = lattice.into_iter().filter(|(r, _)| !relations.contains(r)).collect();
            prog.set_field(Field::Lattice(&kept));
        }
        let nullary = prog.nullary();
        if nullary.iter().any(|r| relations.contains(r)) {
            let kept: BTreeSet<String> = nullary.into_iter().filter(|r| !relations.contains(r)).collect();
            prog.set_field(Field::Nullary(&kept));
        }
    })
}

/// A program's relations that take its prefix (`p<i>_`) when programs are combined: what it declares, holds facts of
/// or derives, other than the base inputs, the shared outputs, `core.rerail`'s (decision 0141), the earlier phases'
/// exports and the guests' shared relations. A repository file a policy reads (`.input r(IO=…)`) is its own.
pub fn own_names<P: link::Part>(part: &P, core: &BTreeSet<String>) -> BTreeSet<String> {
    let prog = part.head();
    let heads: Vec<String> = part.rules().into_iter().map(|(head, _)| head).collect();
    // `violation`, and each finding's relation (`rr::is_diagnostic`, named by its policy). R9's relations too: each
    // program's findings' nodes by clause (named by the clause), one rule refusing them and the placement it reads,
    // the same rules in every program, so a pack holds one of each, not one a program.
    let shared_outputs = |r: &str| {
        use crate::code_rule_dsl::logic::rr::{FOUND_NODE, FOUND_THROUGH, PLACED_AT, PLACED_LINE, UNPLACED};
        r == "violation" || [UNPLACED, FOUND_NODE, FOUND_THROUGH, PLACED_AT, PLACED_LINE].contains(&r) || crate::code_rule_dsl::logic::rr::is_diagnostic(r)
    };
    let file_inputs = file_inputs(prog);
    let inputs = prog.input_names();
    let (declared, facts) = (prog.declared_names(), prog.fact_names());
    declared
        .iter()
        .map(String::as_str)
        .chain(facts.iter().map(String::as_str))
        .chain(heads.iter().map(String::as_str))
        .filter(|r| (!inputs.iter().any(|i| i == r) || file_inputs.contains(*r)) && !shared_outputs(r) && !core.contains(*r) && !phases::is_phased(r) && !guests::SHARED.contains(r))
        .map(str::to_owned)
        .collect()
}

/// The relations a policy reads from repository files (`.input r(IO=…)`): each is the policy's own.
pub fn file_inputs(prog: &dyn crate::code_rule_dsl::logic::program_image::fields::ProgramFields) -> BTreeSet<String> {
    prog.settings().iter().filter(|(key, _)| key == "input").filter_map(|(_, text)| text.split_once('(').map(|(rel, _)| rel.to_owned())).collect()
}

/// A program's request (decision 0089 §3): the library relations of its cone (`cone`) and every relation of its own
/// they read, through rules and natives; and the request's key, the digest of what those relations are. Two programs
/// with one key ask the same, so they read one copy. A request that reads one of the program's own relations it does
/// not hold, or a repository file, is the program's alone (its index is in the key), as is every request when
/// `alone`.
pub fn request_unit<P: link::Part>(part: &P, cone: &BTreeSet<String>, core: &BTreeSet<String>, i: usize, alone: bool) -> (BTreeSet<String>, [u8; 32]) {
    use sha2::Digest as _;
    let own = own_names(part, core);
    let prog = part.head();
    let rules = part.rules();
    let files = file_inputs(prog);
    let (natives, lattice, nullary) = (prog.natives(), prog.lattice(), prog.nullary());
    let mut unit: BTreeSet<String> = cone.iter().filter(|r| own.contains(*r)).cloned().collect();
    let mut work: Vec<String> = unit.iter().cloned().collect();
    let mut outside = false;
    while let Some(rel) = work.pop() {
        let mut reads: Vec<String> = rules.iter().filter(|(head, _)| *head == rel).flat_map(|(_, reads)| reads.iter().cloned()).collect();
        if let Some(native) = natives.get(&rel) {
            reads.extend(native.reads().into_iter().map(str::to_owned));
        }
        for r in reads {
            if files.contains(&r) {
                outside = true;
            }
            if own.contains(&r) && unit.insert(r.clone()) {
                work.push(r);
            }
        }
    }
    let mut hash = sha2::Sha256::new();
    if alone || outside {
        hash.update(format!("program {i}\n"));
    }
    let mut rules: Vec<String> = part.shown(&unit);
    rules.sort();
    for rule in rules {
        hash.update(rule);
        hash.update("\n");
    }
    for rel in &unit {
        hash.update(format!(
            "{rel} {:?} {:?} {:?} {:?} {}\n",
            prog.columns_of(rel),
            prog.states_facts(rel).then(|| prog.fact_rows(rel)),
            natives.get(rel),
            lattice.get(rel),
            nullary.contains(rel)
        ));
    }
    (unit, hash.finalize().into())
}

#[cfg(any(test, feature = "test-hooks"))]
thread_local! {
    /// A test's own store, so that another test's collection in the shared default store does not evict its rows.
    pub static TEST_TAPE_CACHE: std::cell::RefCell<Option<std::path::PathBuf>> = const { std::cell::RefCell::new(None) };
    /// The thread's next memoised run of a program (not of a phase it reads) stops with an error once its first
    /// entries are published (and written), as a run killed then would: what it leaves is a stopped run's.
    pub static STOP_AFTER_ENTRIES: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The store the corpus runner keeps tapes in: `RERAIL_TAPE_CACHE`, else one in the temporary
/// directory; `RERAIL_TAPE_CACHE=off` keeps none. A named store another process kept past this one's wait is an error
/// that names that process (`owner::named`).
pub fn tape_cache() -> std::result::Result<Option<ContentStore>, crate::content_store::StoreError> {
    match tape_cache_path() {
        Some(path) => ContentStore::open(&path).map(Some),
        None => match std::env::var_os("RERAIL_TAPE_CACHE").filter(|v| v != "off").and_then(|v| owner::refusal(std::path::Path::new(&v))) {
            Some(why) => Err(crate::content_store::StoreError::Invalid(why)),
            None => Ok(None),
        },
    }
}

/// Runs `f` with phase 0's observer (`content_store::observe`), kept beside the tape cache (`<tape cache>.files`), or
/// in memory for the process when there is none. Every path the engine reads goes through it: rule files, corpus
/// files, what resolution rules list, read and load.
pub fn observe<T>(f: impl FnOnce(&mut crate::content_store::observe::Observer) -> T) -> T {
    // The engine's tests check paths by their `stat`: the journal's wait for its cookie, made by every run, is the
    // journal's own tests' to measure (`content_store::observe`). A test of the engine that wants it turns it on.
    #[cfg(any(test, feature = "test-hooks"))]
    {
        static OFF: std::sync::Once = std::sync::Once::new();
        OFF.call_once(|| {
            // Explicit workload caches use production observer semantics. Ordinary unit
            // tests keep stat-based isolation; RERAIL_JOURNAL still controls benchmarks.
            if std::env::var_os("RERAIL_TAPE_CACHE").is_none() {
                crate::content_store::observe::set_journal(false);
            }
        });
    }
    let store = tape_cache_path().map(|p| crate::content_store::observe::store_beside(&p));
    // Each test thread its own observer, unless the test set its own tape cache: another test's run flushes a shared
    // one's journal, after which it answers for this test's edits until the next flush (decision 0105).
    #[cfg(any(test, feature = "test-hooks"))]
    let store = store.map(|p| {
        // An explicit workload cache keeps production observer semantics; a test's own store is its alone.
        if TEST_TAPE_CACHE.with(|t| t.borrow().is_some())
            || std::env::var_os("RERAIL_TAPE_CACHE").is_some()
            || tape_cache_path().is_some_and(|own| test_store::is_own(&own))
        {
            return p;
        }
        let thread: String = format!("{:?}", std::thread::current().id()).chars().filter(char::is_ascii_digit).collect();
        std::path::PathBuf::from(format!("{}.thread{thread}", p.display()))
    });
    crate::content_store::observe::with(store.as_deref(), f)
}

/// The store this caller's runs use instead of the machine's while one is bound (`use_store`): testing mode's
/// (decision 0130), whose runs must not read what other runs left, nor wait behind their writes. In the run's
/// context (decision 0442), so its runs' jobs use it too.
#[derive(Clone, Default)]
pub struct UseStore(Option<std::path::PathBuf>);

#[derive(Clone)]
pub struct StoreSelection {
    path: Option<PathBuf>,
    ownership: Option<(PathBuf, crate::content_store::analysis_scope::AnalysisOwner)>,
    git_reads: Arc<crate::git_workspace::snapshot::InventoryReads>,
}

/// The store the caller selected for its runs (`ThreadStore::select`), in the run's context (decision 0442): its
/// runs' jobs read it too.
#[derive(Clone, Default)]
pub struct SelectedStore(Option<StoreSelection>);

pub struct ThreadStore {
    _bound: crate::code_rule_dsl::logic::profile::Bound,
}

impl ThreadStore {
    pub fn select(path: &Path, ownership: Option<(PathBuf, crate::content_store::analysis_scope::AnalysisOwner)>) -> Self {
        let path = (path != Path::new(":memory:")).then(|| path.to_owned());
        Self { _bound: crate::code_rule_dsl::logic::profile::bind(SelectedStore(Some(StoreSelection { path, ownership, git_reads: Arc::default() }))) }
    }

    fn with<R>(f: impl FnOnce(Option<&StoreSelection>) -> R) -> R {
        crate::code_rule_dsl::logic::profile::with_bound(|s: &SelectedStore| f(s.0.as_ref()))
    }

    pub fn owner(root: &Path) -> Option<crate::content_store::analysis_scope::AnalysisOwner> {
        Self::with(|store| {
            let (expected, owner) = store?.ownership.as_ref()?;
            (expected == root).then(|| owner.clone())
        })
    }

    pub fn ephemeral() -> bool {
        Self::with(|store| store.is_some_and(|store| store.path.is_none()))
    }

    pub fn git_reads() -> Option<Arc<crate::git_workspace::snapshot::InventoryReads>> {
        Self::with(|store| store.map(|selected| Arc::clone(&selected.git_reads)))
    }
}

/// Run with an explicitly selected cache on this thread.
pub fn with_tape_cache_path<T>(path: &Path, ownership: Option<(PathBuf, crate::content_store::analysis_scope::AnalysisOwner)>, run: impl FnOnce() -> T) -> T {
    let _store = ThreadStore::select(path, ownership);
    run()
}

/// While the guard lives, every run of this thread (without `RERAIL_TAPE_CACHE`) uses the store at `path`.
pub fn use_store(path: std::path::PathBuf) -> UsedStore {
    UsedStore { _store: crate::code_rule_dsl::logic::profile::bind(UseStore(Some(path))) }
}

/// `use_store`'s guard: the store before it comes back when it drops.
pub struct UsedStore {
    _store: crate::code_rule_dsl::logic::profile::Bound,
}

/// Whether the caller names the store its runs use (`RERAIL_TAPE_CACHE`, `use_store`), so a test's runs use that
/// one and not their own copy (`test_store::carried`).
#[cfg(any(test, feature = "test-hooks"))]
pub fn names_its_store() -> bool {
    std::env::var_os("RERAIL_TAPE_CACHE").is_some() || crate::code_rule_dsl::logic::profile::bound::<UseStore>().0.is_some()
}

/// The path `tape_cache` opens. `None`: `RERAIL_TAPE_CACHE=off`, or the store it names belongs to another process past
/// this one's wait (`owner::refusal`, said once as it was refused), and the process runs without a store; `tape_cache`,
/// the run's opener, fails with that error.
pub fn tape_cache_path() -> Option<std::path::PathBuf> {
    if let Some(path) = ThreadStore::with(|store| store.map(|store| store.path.clone())) {
        return path;
    }
    #[cfg(any(test, feature = "test-hooks"))]
    if let Some(path) = TEST_TAPE_CACHE.with(|p| p.borrow().clone()).or_else(test_store::handed) {
        return Some(path);
    }
    if std::env::var_os("RERAIL_TAPE_CACHE").is_none()
        && let Some(path) = crate::code_rule_dsl::logic::profile::bound::<UseStore>().0
    {
        return Some(path);
    }
    match std::env::var("RERAIL_TAPE_CACHE") {
        Ok(v) if v == "off" => None,
        // The user's store, this process's once no other owns it (`owner::named`): none while another keeps it.
        Ok(v) => {
            let path = std::path::PathBuf::from(v);
            owner::named(&path).is_ok().then_some(path)
        }
        // A test's own store, a copy of the template (`test_store`); the process's only off a test's thread.
        #[cfg(any(test, feature = "test-hooks"))]
        Err(_) => Some(test_store::own().unwrap_or_else(|| test_tape_cache().clone())),
        #[cfg(not(any(test, feature = "test-hooks")))]
        Err(_) => Some(default_store().clone()),
    }
}

/// The store a run uses when none is named: the project's for this build (`build_store`), the project being the
/// working directory. Every stored key carries the engine's build, so another build's entries are of no use to a run;
/// and another project's runs would leave their atoms and plans in it, and hold its writer. Of that store's slots, the
/// first no other process owns (`owner::slot`): runs at once from one directory each have their own.
#[cfg(not(any(test, feature = "test-hooks")))]
pub fn default_store() -> &'static std::path::PathBuf {
    static PATH: std::sync::LazyLock<std::path::PathBuf> = std::sync::LazyLock::new(|| {
        let cwd = std::env::current_dir().ok().and_then(|d| dunce::canonicalize(&d).ok()).unwrap_or_default();
        owner::slot(&build_store("rerail-stores", Some(&cwd.to_string_lossy())))
    });
    &PATH
}

/// A store of this engine build's in `$TMPDIR/<dir>/`: `<build>.sqlite`, or `<build>-<key>.sqlite` for one `key` (a
/// project) of several. Another build's files there, unused for a day, are removed (with their `-wal`, `-shm` and
/// lock files, which start with the same name), unless a live process owns their store (`owner::remove_unowned`).
pub fn build_store(dir: &str, key: Option<&str>) -> std::path::PathBuf {
    use std::hash::{Hash, Hasher};
    let dir = std::env::temp_dir().join(dir);
    let _ = crate::content_store::prepare_cache_directory(&dir);
    let build = crate::code_rule_dsl::build_ids::engine();
    let build = &build[..build.len().min(16)];
    let day = std::time::Duration::from_secs(24 * 3600);
    let old: Vec<String> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > day))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| !name.starts_with(build))
        .collect();
    owner::remove_unowned(&dir, &old);
    match key {
        // The hasher's keys are fixed: within a build, a project's name is the same in every process.
        Some(key) => {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            key.hash(&mut h);
            dir.join(format!("{build}-{:016x}.sqlite", h.finish()))
        }
        None => dir.join(format!("{build}.sqlite")),
    }
}

/// The process's store, for what asks off a test's thread (a test's is its own, `test_store`): the machine's is
/// written by every other run on it, so a test's writes could wait out its busy deadline behind them. Removed when
/// the process exits; one a killed process left is removed by the next test process to start.
#[cfg(any(test, feature = "test-hooks"))]
pub fn test_tape_cache() -> &'static std::path::PathBuf {
    static PATH: std::sync::LazyLock<std::path::PathBuf> = std::sync::LazyLock::new(|| {
        const PREFIX: &str = "rerail-tape-cache-test-";
        let dir = std::env::temp_dir();
        // Another process's, when it is gone.
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(pid) = name.strip_prefix(PREFIX).and_then(|rest| rest.split('.').next()).and_then(|p| p.parse::<i32>().ok()) else { continue };
            if crate::content_store::process_gone(pid) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
        extern "C" fn remove() {
            let own = format!("{PREFIX}{}.", std::process::id());
            for entry in std::fs::read_dir(std::env::temp_dir()).into_iter().flatten().flatten() {
                if entry.file_name().to_string_lossy().starts_with(&own) {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        // The C runtime's, on every platform (libc's crate has it on Unix only).
        unsafe extern "C" {
            fn atexit(f: extern "C" fn()) -> std::ffi::c_int;
        }
        // SAFETY: `remove` is a plain function that runs once, at exit.
        unsafe { atexit(remove) };
        dir.join(format!("{PREFIX}{}.sqlite", std::process::id()))
    });
    &PATH
}

/// Inputs specific to an embedded fact phase. Its own manifest retains the compiled program,
/// including when another evaluation calls this phase before it publishes its own manifest.
#[derive(Clone, Copy, Default)]
pub struct FactPhase<'a> {
    pub program_entry: Option<&'a combined::Embedded>,
    _run: std::marker::PhantomData<&'a ()>,
}

/// Runs under way here, in the run's context: a run started inside another (a phase, the constants, phase 0) is
/// nested, and so is a run in a job of one (decision 0442: the context goes with the job).
#[derive(Clone, Copy, Default)]
pub struct RunsUnderWay(usize);

/// Counts a run under way here until dropped.
pub struct UnderWay {
    _count: crate::code_rule_dsl::logic::profile::Bound,
}

impl UnderWay {
    pub fn enter() -> UnderWay {
        UnderWay { _count: crate::code_rule_dsl::logic::profile::bind(RunsUnderWay(runs_under_way() + 1)) }
    }
}

/// Runs under way here (`RunsUnderWay`).
pub fn runs_under_way() -> usize {
    crate::code_rule_dsl::logic::profile::bound::<RunsUnderWay>().0
}

/// `programs` with their messages masked (`rr::message_masked`), as the keys read them.
pub fn masked<'a>(programs: &[&'a str]) -> Vec<std::borrow::Cow<'a, str>> {
    programs.iter().map(|t| crate::code_rule_dsl::logic::rr::message_masked(t)).collect()
}

/// A corpus's files by language: Python's, Rust's and `.rerail` rule files, and the text-only documents.
#[derive(Clone)]
pub struct CorpusFiles {
    /// Each dialect's files, by its label (the language registry's).
    pub by_label: BTreeMap<&'static str, Vec<String>>,
    /// Files of no grammar: after `scoped`, those a `.scope text` glob reads as text (`ProgramFields::text_globs`),
    /// with the files of a grammar it matches (a `.md` file is Markdown's, and text to a run of another grammar).
    pub text: Vec<String>,
    /// Whether the program reads the files that hold its language's programs (`.embedded on`, decision 0470 step 2);
    /// set by `scoped`. Off by default.
    pub embedded: bool,
    /// The root the paths are under: a place embedding's host is read only where its text names a place.
    pub root: std::path::PathBuf,
}

/// Whether `path`'s dialect embeds `label`'s (decision 0470 step 2): its file holds programs a run of `label` reads
/// under `.embedded on`.
pub fn embeds_label(path: &str, label: &str) -> bool {
    use crate::code_rule_dsl::language;
    language::label_of_path(path)
        .filter(|own| *own != label)
        .and_then(language::dialect_by_label)
        .is_some_and(|d| d.embeds().iter().any(|&(_, e)| language::label(e) == Some(label)))
}

/// Whether the program says `.embedded on`: its run reads the files that hold its language's programs.
pub fn embedded_on(prog: &(impl ProgramFields + ?Sized)) -> bool {
    prog.setting_values("embedded").iter().any(|v| v.trim().trim_matches('"') == "on")
}

/// Whether a host file of a place embedding holds a program of `label` (decision 0470 §4): its text names a place's
/// last segment. An embedding of whole elements (an HTML file's scripts) holds one wherever its file has them.
pub fn names_a_place(root: &Path, path: &str, label: &str) -> bool {
    use crate::code_rule_dsl::language;
    let Some(dialect) = language::label_of_path(path).and_then(language::dialect_by_label) else { return false };
    let places: Vec<&str> = dialect.embed_places().iter().filter(|(_, e, ..)| language::label(*e) == Some(label)).map(|(_, _, callee, _)| callee.rsplit('.').next().unwrap_or(callee)).collect();
    let whole = dialect.embeds().iter().any(|&(via, e)| language::label(e) == Some(label) && !dialect.embed_places().iter().any(|(v, ..)| *v == via));
    if whole {
        return true;
    }
    crate::git_workspace::snapshot::read(root.join(path)).is_ok_and(|text| places.iter().any(|p| memchr::memmem::find(&text, p.as_bytes()).is_some()))
}

impl CorpusFiles {
    /// The run's documents: the language's files, then the text ones, in path order; under `.embedded on`, also the
    /// files that hold its programs (decision 0470 step 2).
    /// A file of `label`'s grammar a `.scope text` glob also matches is parsed, not read as text as well.
    pub fn of(mut self, label: &str) -> Vec<String> {
        let mut files = self.by_label.remove(label).unwrap_or_default();
        if self.embedded {
            let root = self.root.clone();
            for paths in self.by_label.values() {
                files.extend(paths.iter().filter(|p| embeds_label(p, label) && names_a_place(&root, p, label)).cloned());
            }
        }
        files.extend(self.text);
        files.sort();
        files.dedup();
        files
    }

    /// How many files of `label` there are.
    pub fn count(&self, label: &str) -> usize {
        self.by_label.get(label).map_or(0, Vec::len)
    }

    /// How many files of any grammar there are.
    pub fn parsed(&self) -> usize {
        self.by_label.values().map(Vec::len).sum()
    }
}

/// The documents JSON policies select, each its `scope`'s `include` (every file where it has none) less its `exclude`;
/// `None` where a policy has no `scope` (it reads every file) or there is no policy.
pub fn json_scopes(policies: &[serde_json::Value]) -> Option<Vec<(Vec<String>, Vec<String>)>> {
    let globs = |v: Option<&serde_json::Value>| -> Vec<String> { v.and_then(|v| v.as_array()).into_iter().flatten().filter_map(|g| g.as_str().map(str::to_owned)).collect() };
    if policies.is_empty() {
        return None;
    }
    policies.iter().map(|p| p.get("scope").map(|s| (globs(s.get("include")), globs(s.get("exclude"))))).collect()
}

/// The files the program's policies select (`ProgramFields::doc_scopes`, their `.scope`): a document outside
/// every policy's scope is not parsed, and gives no facts.
/// A scope entry taken from a policy option (`.scope dirs option("…")`) reads the option's values, from the run's
/// `options`. A file of no grammar is read, as text only, when a `.scope text` glob matches it and a scope selects it.
pub fn scoped(mut found: CorpusFiles, prog: &(impl ProgramFields + ?Sized), options: &serde_json::Value) -> CorpusFiles {
    found.embedded = embedded_on(prog);
    if let Some(paths) = options.get("sourcePaths").and_then(serde_json::Value::as_array) {
        let paths: Vec<&Path> = paths.iter().filter_map(serde_json::Value::as_str).map(Path::new).collect();
        let allowed = |path: &String| paths.iter().any(|prefix| Path::new(path).starts_with(prefix));
        found.by_label.values_mut().for_each(|files| files.retain(allowed));
        found.text.retain(allowed);
    }
    let globs = prog.text_globs();
    let text_glob = |p: &String| globs.iter().any(|g| crate::code_rule_dsl::logic::strings::test("glob", p, g));
    found.text.retain(text_glob);
    // A glob names files of a grammar too (`"**/*.md"`): a run of another grammar reads them as text (`CorpusFiles::of`).
    // A hidden file (`.hidden.md`) is no text document, as `corpus_files` has it.
    let hidden = |p: &String| p.rsplit('/').next().is_some_and(|name| name.starts_with('.'));
    let of_grammars: Vec<String> = found.by_label.values().flatten().filter(|p| text_glob(p) && !hidden(p)).cloned().collect();
    found.text.extend(of_grammars);
    found.text.sort();
    // A program of JSON policies (`.policy {…}`) and no `.scope`: their `scope`s (decision 0107).
    let doc_scopes = prog.doc_scopes().or_else(|| json_scopes(&prog.policies()));
    if doc_scopes.is_none() {
        return found;
    }
    let scopes = front::resolve_scopes(&doc_scopes, options);
    let before = found.parsed();
    for files in found.by_label.values_mut().chain([&mut found.text]) {
        files.retain(|p| front::in_doc_scopes(&scopes, p));
    }
    crate::code_rule_dsl::logic::profile::decision("documents", || {
        format!("{} of {before} files, by the policies' `.scope`; {} read as text (`.scope text`)", found.parsed(), found.text.len())
    });
    found
}

/// A digest as lowercase hex.
pub fn hex(digest: &[u8]) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// A reader's refusal of one of a run's programs, named by the program's `.policy` (a run reads texts, not files).
pub fn in_policy(text: &str, e: crate::code_rule_dsl::logic::front::Error) -> crate::code_rule_dsl::logic::front::Error {
    use crate::code_rule_dsl::logic::front::Error;
    let name = text.lines().find_map(|l| l.trim().strip_prefix(".policy ").and_then(|r| r.split('"').nth(1)).map(str::to_owned));
    match (e, name) {
        (Error::Syntax(m), Some(n)) => Error::Syntax(format!("policy {n}: {m}")),
        (e, _) => e,
    }
}

/// A measurement made once per mix (Q88): the chunk that made it (its place in the filing order), the rows it was
/// made over, and the measurement.
#[derive(Clone)]
pub struct Once<T>(pub usize, pub usize, pub T);

/// A text's version with the library's cone it asks for (decision 0089 §3), as `rr::Text::lowered_in` makes it on
/// the tree (decision 0281's path, step 7b): its answers on request undone (`rr::lower::undone_on_request`), the cone
/// copied from the library's version (`rr::with_cone_version`), and the attributes on request made again over both
/// (`on_request::SourcesOnRequest`, decision 0174), its cone's readers asking too. No tree is decoded.
pub fn cone_on_version(
    own: &Arc<crate::code_rule_dsl::logic::program_image::Version>,
    library: &Arc<crate::code_rule_dsl::logic::program_image::Version>,
    asked: &BTreeSet<String>,
    declared: &BTreeSet<String>,
) -> std::result::Result<Arc<crate::code_rule_dsl::logic::program_image::Version>, String> {
    let undone = crate::code_rule_dsl::logic::rr::lower::undone_on_request(own)?;
    let coned = crate::code_rule_dsl::logic::rr::with_cone_version(&undone, library, asked, declared)?;
    let mut p = crate::code_rule_dsl::logic::program_image::manager::Pipeline::new("a text's cone", coned);
    for round in crate::code_rule_dsl::logic::rr::lower::ON_REQUEST {
        p.program(&mut preparation::on_request::SourcesOnRequest { round });
    }
    p.current()?;
    for d in p.decisions() {
        crate::code_rule_dsl::logic::profile::decision(&d.what, || d.detail.clone());
    }
    Ok(p.finish()?.0)
}

#[allow(clippy::too_many_arguments)]
pub fn run_with_options(
    programs: &[&str],
    root: &Path,
    t_facts: Option<&Path>,
    out: &Path,
    modes: &str,
    binding: Binding,
    chunk_nodes: usize,
    typing: &Typing,
    options: &serde_json::Value,
) -> Result<Report> {
    // The run's options as it is first given them: a `toolCalls` option not of its shape is an error here (`added`).
    added::check_tool_calls(options).map_err(Error::Plan)?;
    run_handed(programs, None, None, root, t_facts, out, modes, binding, chunk_nodes, typing, options, None, Default::default())
}

pub fn install_git_inventory() {
    crate::git_workspace::snapshot::install_inventory_reader(git::inventory);
    crate::git_workspace::snapshot::install_source_checker(git::check_source_digests);
}

pub fn usage() -> (f64, f64, f64) {
    // wasm32: the one thread's time as user CPU, and no resident-set counter.
    #[cfg(target_family = "wasm")]
    {
        (crate::code_rule_dsl::prof::single_thread_cpu().as_secs_f64(), 0.0, 0.0)
    }
    // Windows: the process's user and kernel CPU, and its peak working set.
    #[cfg(windows)]
    {
        let (user, kernel) = crate::code_rule_dsl::prof::windows_cpu(false);
        (user.as_secs_f64(), kernel.as_secs_f64(), crate::code_rule_dsl::prof::windows_peak_working_set() as f64 / 1e6)
    }
    #[cfg(unix)]
    {
        let mut u: libc::rusage = unsafe { std::mem::zeroed() };
        unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut u) };
        let s = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
        // macOS reports ru_maxrss in bytes, Linux in kilobytes.
        let rss = if cfg!(target_os = "macos") { u.ru_maxrss as f64 / 1e6 } else { u.ru_maxrss as f64 / 1e3 };
        (s(u.ru_utime), s(u.ru_stime), rss)
    }
}

/// The root a manifest names: the fact root of a snapshot, else the canonical path.
pub fn manifest_root(root: &Path) -> String {
    crate::git_workspace::snapshot::fact_root(root)
        .or_else(|| root.canonical().ok()).unwrap_or_else(|| root.to_path_buf()).to_string_lossy().into_owned()
}

/// `run_read` inside another run that hands it the source texts it read (`sources`, decision 0300): phase 1's.
#[allow(clippy::too_many_arguments)]
pub fn run_read_handing(
    read: &Arc<crate::code_rule_dsl::logic::program_image::Version>,
    sources: Option<&texts::Sources>,
    root: &Path,
    out: &Path,
    modes: &str,
    binding: Binding,
    chunk_nodes: usize,
    typing: &Typing,
    options: &serde_json::Value,
    image: Option<&mut Vec<u8>>,
) -> Result<Report> {
    added::check_tool_calls(options).map_err(Error::Plan)?;
    let key = read_key(read)?;
    run_handed(&[key.as_str()], Some(read), sources, root, None, out, modes, binding, chunk_nodes, typing, options, image, Default::default())
}

pub fn run_fact_image(source: &str, root: &Path, options: &serde_json::Value) -> Result<Vec<u8>> {
    run_fact_image_planned(source, root, options).map(|(image, _)| image)
}

/// `run_fact_image`, with the memo plans its run (and the phases it ran) recorded manifests of: what a later run that
/// reuses its image in memory (a Git view's inventory) owns again for its own scope (`own_recorded_manifests`).
pub fn run_fact_image_planned(source: &str, root: &Path, options: &serde_json::Value) -> Result<(Vec<u8>, Vec<[u8; 32]>)> {
    let store = if cfg!(feature = "tape-cache") { phases::reading_store() } else { None };
    let _reading = store.as_ref().map(|store| store.hold_collection()).transpose()?;
    let (program, program_entry) = combined::embedded_program(source, store.as_deref())?;
    let program_entry = program_entry.as_ref();
    let key = read_key(&program)?;
    let mut image = Vec::new();
    let report = run_handed(&[key.as_str()], Some(&program), None, root, None, Path::new(""), "program",
        Binding::Cpu, 1 << 18, &Typing::default(), options, Some(&mut image), FactPhase { program_entry, _run: std::marker::PhantomData })?;
    let mut plans = report.plans.clone();
    plans.extend(report.phase_runs.iter().flat_map(|r| r.report.plans.iter().copied()));
    plans.sort();
    plans.dedup();
    Ok((image, plans))
}

/// A handed program's key, in the place of a text: its version's digest (`choices::key`, decision 0173 §7), named so
/// that it reads as no program. Every key a run makes of its texts is made of this.
pub fn read_key(read: &Arc<crate::code_rule_dsl::logic::program_image::Version>) -> Result<String> {
    let version = super::super::body::Placed::whole(Arc::clone(read));
    Ok(format!("rerail.read-program {}", crate::content_store::observe::hex(&super::super::choices::key(&version))))
}

pub fn corpus_language(language: Option<&str>, typing: &Typing, found: &CorpusFiles) -> &'static str {
    match language {
        Some(name) if let Some(label) = crate::code_rule_dsl::language::label_named(name) => label,
        _ if typing.rust.unwrap_or(found.count("rust") > found.count("python")) => "rust",
        _ => "python",
    }
}

/// The files Git sees under `root`, relative, and how many of them Git's status changed (staged, untracked,
/// deleted): `None` outside a work tree, or where Git sees none under it.
///
/// Decision 0105: the list is kept as the observer's record (`git-files:<root>`), with what Git read for it: the
/// root, the `.git` file and the ignore files above the root, the work tree's `HEAD` and index, and the refs,
/// config and info shared with other work trees. Where the journal names a change in none of those, the list is
/// as the status found it, and the status is not read again (a status stats every file in the index).
pub fn git_files(root: &Path) -> Option<(Vec<String>, usize)> {
    use std::path::PathBuf;
    let real = observe(|o| o.canonical(root))?;
    let key = format!("git-files:{}", real.display());
    // What the record read (`reuse::encode_lists`): the journal's cursors before the status, the roots they are of,
    // and the paths read; and what it found: the overlays' count (eight bytes), then the files.
    let kept = observe(|o| {
        let (read, files) = o.run_record(&key)?;
        let [cursors, watched, read]: [Vec<String>; 3] = reuse::decode_lists(&read)?.try_into().ok()?;
        let watched: Vec<PathBuf> = watched.into_iter().map(PathBuf::from).collect();
        let read: Vec<PathBuf> = read.into_iter().map(PathBuf::from).collect();
        let named = o.changed_since(&key, &watched, &cursors)?;
        // The repository's own directories, of which only what `read` names counts (not its objects).
        let git_dirs: Vec<PathBuf> = watched.iter().skip(2).cloned().collect();
        let overlays = u64::from_le_bytes(files.get(..8)?.try_into().ok()?) as usize;
        let [under]: [Vec<String>; 1] = reuse::decode_lists(&files[8..])?.try_into().ok()?;
        Some((read, git_dirs, named, overlays, under))
    });
    if let Some((read, git_dirs, named, overlays, under)) = kept {
        // A change named in what Git reads besides the files (its index, refs, ignore files), or at a directory above
        // one, may change the list. A file named under the root leaves it as it was when it is still a file the list
        // holds, or still absent and not in the list; a directory named, or one the list has files under, may not.
        let listed: std::collections::BTreeSet<&str> = under.iter().map(String::as_str).collect();
        let holds = named.iter().all(|n| {
            if read.iter().any(|p| (n.starts_with(p) && *p != real) || (p.starts_with(n) && *p != *n)) {
                return false;
            }
            if git_dirs.iter().any(|d| n.starts_with(d)) {
                return true;
            }
            let Ok(relative) = n.strip_prefix(&real) else { return true };
            let relative = relative.to_string_lossy();
            let inside = format!("{relative}/");
            if n.is_dir() || listed.range(inside.as_str()..).next().is_some_and(|f| f.starts_with(inside.as_str())) {
                return false;
            }
            listed.contains(relative.as_ref()) == n.is_file()
        });
        if holds {
            crate::code_rule_dsl::logic::profile::decision("phase 0: Git's view of the corpus (decision 0105)", || {
                format!("{} files as the last status found them: of {} paths the journal names, none changes the list", under.len(), named.len())
            });
            return (!under.is_empty()).then_some((under, overlays));
        }
    }
    // Watched before the status is read: a change while it reads is named to the next run.
    let places = crate::git_workspace::locations(&real);
    let cursors = places.as_ref().and_then(|(work, own, common)| observe(|o| o.mark(&[real.clone(), work.clone(), own.clone(), common.clone()])));
    // Kept when Git sees no file under the root too (a corpus in an ignored directory): unkept, every run read the
    // work tree's whole status again to find none (decision 0285).
    let found = observe(|o| o.git(root)).map(|g| {
        // With `/`, as Git spells the work tree's files.
        let prefix = real.strip_prefix(&g.worktree_root).map(slashed).unwrap_or_default();
        let under: Vec<String> = g
            .worktree_files()
            .into_iter()
            .filter_map(|file| if prefix.is_empty() { Some(file) } else { file.strip_prefix(prefix.as_str()).and_then(|f| f.strip_prefix('/')).map(str::to_owned) })
            .collect();
        (under, g.overlays.len())
    });
    if let (Some((work, own, common)), Some(cursors), Some((under, overlays))) = (&places, cursors, &found) {
        let mut read: Vec<PathBuf> = vec![real.clone(), work.join(".git"), own.join("HEAD"), own.join("index")];
        read.extend(real.ancestors().filter(|d| d.starts_with(work) && *d != real.as_path()).map(|d| d.join(".gitignore")));
        read.extend(["HEAD", "refs", "packed-refs", "config", "info"].iter().map(|f| common.join(f)));
        let watched: Vec<String> = [&real, work, own, common].iter().map(|p| p.to_string_lossy().into_owned()).collect();
        let read: Vec<String> = read.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        let record = reuse::encode_lists(&[&cursors[..], &watched[..], &read[..]]);
        let mut files = (*overlays as u64).to_le_bytes().to_vec();
        files.extend(reuse::encode_lists(&[&under[..]]));
        observe(|o| o.put_run_record(&key, &record, &files));
    }
    found.filter(|(under, _)| !under.is_empty())
}

/// Whether the corpus lists files under a directory of this name: every one but those the host's walk never enters
/// (`git_workspace::ignored_directories`), hidden ones (`.github`) included.
pub fn listed_directory(name: &str) -> bool {
    !crate::git_workspace::ignored_directory(name)
}

/// Whether `root` is a site directory (`site-packages`): it holds an installed distribution's `*.dist-info`, which
/// `typing::deps::Installed::scan` reads. The host's ignored directories are a repository's; a site directory's
/// packages are what Python imports, whatever their names (`vendor`, `build`, `coverage`), so none is skipped there.
pub fn site_directory(root: &Path) -> bool {
    observe(|o| o.listing(root)).is_some_and(|listing| listing.iter().any(|(name, is_dir)| *is_dir && name.ends_with(".dist-info")))
}

/// The files under `root` the corpus lists, relative, each with its grammar's label where the listing knows it. In a
/// Git work tree, the files Git sees under the root (HEAD's, less deleted, plus staged and untracked ones it does not
/// ignore), with no walk: from the inventory (`git_workspace::snapshot::scan_files`, with each file's grammar), else
/// Git's listing (`git_files`). Outside one, or where Git cannot read it, or where it sees no file under the root (one
/// it ignores, such as fetched fixtures), the walk. None lists a file under a directory `listed_directory` refuses, but
/// in a site directory (`site_directory`).
pub fn corpus_listing(root: &Path) -> Result<Vec<(String, Option<&'static str>)>> {
    let enters: fn(&str) -> bool = if site_directory(root) { |_| true } else { listed_directory };
    let entered = |relative: &str| relative.rsplit_once('/').is_none_or(|(dirs, _)| dirs.split('/').all(enters));
    install_git_inventory();
    if let Some(files) = crate::git_workspace::snapshot::scan_files(root, ThreadStore::git_reads().as_deref()) {
        use crate::git_workspace::snapshot::InventoryError;
        let files = files.map_err(|error| match error {
            InventoryError::Source(reason) => Error::Plan(reason),
            InventoryError::Cache(error) => Error::Cache(error),
        })?;
        return Ok(files.into_iter().filter(|(relative, _)| entered(relative)).map(|(relative, grammar)| (relative, grammar.as_deref().and_then(crate::code_rule_dsl::language::label_named))).collect());
    }
    let paths = match git_files(root) {
        Some((under, overlays)) => {
            let listed: Vec<String> = under.into_iter().filter(|p| entered(p)).collect();
            crate::code_rule_dsl::logic::profile::decision("phase 0: corpus files", || format!("{} files from Git ({overlays} overlays: changed, staged or untracked), no walk", listed.len()));
            listed
        }
        None => walked(root, enters)?,
    };
    // The language registry's suffixes (`languages/<id>.json`): Python's include stubs (`.pyi`).
    Ok(paths.into_iter().map(|p| {
        let label = crate::code_rule_dsl::language::label_of_path(&p);
        (p, label)
    }).collect())
}

/// The corpus's files under `root` (`corpus_listing`), relative and in path order: each grammar's, by the language
/// registry's suffixes (`languages/<id>.json`: Python's include stubs, `.pyi`), and the others as text.
pub fn corpus_files(root: &Path) -> Result<CorpusFiles> {
    let (mut by_label, mut text): (BTreeMap<&'static str, Vec<String>>, Vec<String>) = (BTreeMap::new(), Vec::new());
    for (relative, grammar) in corpus_listing(root)? {
        match grammar {
            Some(label) => by_label.entry(label).or_default().push(relative),
            // A hidden file (`.env`, `.hidden.md`) is no text document, as in W.
            None if !relative.rsplit('/').next().is_some_and(|name| name.starts_with('.')) => text.push(relative),
            None => {}
        }
    }
    by_label.values_mut().for_each(|files| files.sort());
    text.sort();
    // `RERAIL_SAMPLE_EVERY=n`: every n-th file only, to iterate on a rule's cost quickly (its findings are a sample's).
    if let Some(n) = std::env::var("RERAIL_SAMPLE_EVERY").ok().and_then(|n| n.parse::<usize>().ok()).filter(|n| *n > 1) {
        // Documents read as text too: a rule that reads every text document (a check over the repository's docs)
        // would otherwise read all of them in every sample.
        for files in by_label.values_mut().chain([&mut text]) {
            *files = files.iter().step_by(n).cloned().collect();
        }
    }
    Ok(CorpusFiles { by_label, text, embedded: false, root: root.to_path_buf() })
}

/// The walk, into the directories `enters` allows: phase 0's observer lists each directory, again only when its stat
/// changed (t23). The run reads its files by their canonical paths too (`Observer::canonical`): a root reached through
/// a symlink (macOS's `/var` is `/private/var`) has its real part read before the waves as well, where no store is
/// asked.
pub fn walked(root: &Path, enters: fn(&str) -> bool) -> Result<Vec<String>> {
    observe(|o| {
        o.preload(root);
        if let Ok(real) = dunce::canonicalize(root)
            && real != root
        {
            o.preload(&real);
        }
    });
    let (mut pending, mut out) = (vec![root.to_path_buf()], Vec::new());
    while let Some(dir) = pending.pop() {
        let listing = observe(|o| o.listing(&dir)).ok_or_else(|| Error::Plan(format!("{}: not a readable directory", dir.display())))?;
        for (name, is_dir) in listing.iter() {
            let path = dir.join(name);
            if *is_dir {
                pending.extend(enters(name).then_some(path));
            } else if path.is_file() {
                // A document is named by its path in the repository, with `/` on Windows too: scopes' globs and
                // findings read it.
                out.push(slashed(path.strip_prefix(root).map_err(|e| Error::Plan(e.to_string()))?));
            }
        }
    }
    Ok(out)
}

/// A path under a root as a document's path is spelled on every OS (and a canonical one as phase 1 hands it on), Git's
/// way: its parts joined with `/` (`slashed_by`, by the platform's separator).
pub fn slashed(relative: &Path) -> String {
    slashed_by(&relative.to_string_lossy(), std::path::MAIN_SEPARATOR)
}

/// A relative path whose parts `separator` divides (the platform's: `\` on Windows), with `/` between them, as every
/// reader of the corpus's paths takes them (scope globs, artifact digests, Git's listing). Elsewhere a `\` is a
/// character of a name, and is kept.
pub fn slashed_by(relative: &str, separator: char) -> String {
    relative.split(separator).collect::<Vec<_>>().join("/")
}

/// The files under `root` the corpus lists (`corpus_listing`), relative.
pub fn corpus_paths(root: &Path) -> Result<Vec<String>> {
    Ok(corpus_listing(root)?.into_iter().map(|(p, _)| p).collect())
}

/// The programs `programs` (`.rerail` text) as one program. One program is parsed as it is.
/// Of several, each one's own relations (the ones it derives or declares beyond its inputs) take a prefix, `p<i>_`,
/// so programs that name relations alike (a converter's `m650`, `exists_0`) do not meet. The base inputs and the
/// `violation` output are shared, and kind sets are named by their kinds (`ks<j>`), so programs asking for one set
/// share its tape pass. The simplifier then merges relations that are equal across programs (decision 0038).
pub fn combine(programs: &[&str]) -> Result<Arc<crate::code_rule_dsl::logic::program_image::Version>> {
    let graphs: Vec<Option<[u8; 32]>> = programs.iter().map(|t| crate::code_rule_dsl::logic::rr::graph_digest(t).ok().flatten()).collect();
    let masked = masked(programs);
    combine_version(programs, &masked.iter().map(|t| t.as_ref()).collect::<Vec<_>>(), &graphs, &added::Added::default(), None).map(|(program, _)| program)
}


// Its own tests, which read `link`'s parts (this crate's); `base_tests` is the engine's: it compiles rule documents.
#[cfg(test)]
mod tests {
    /// Decision 0431 R9's relations are a pack's, not a program's (`own_names`): each program's findings' nodes by
    /// clause and the one rule refusing them are the same relations in every program, so a pack holds one of each.
    /// Each program's own, they were 310 relations and as many strata in one pack of corpus0's policies.
    #[test]
    fn a_packs_programs_share_their_unplaced_findings_relations() {
        use super::super::link::Part as _;
        let text = r#"
.policy "t:shared-unplaced" revision 1
.decl e(n: node)
.input e
.violation Hit(at: node) "a hit"
Hit(n) :- e(n).
"#;
        let v = crate::code_rule_dsl::logic::rr::parse_version(text).unwrap();
        let heads: Vec<String> = v.rules().map(|r| v.rel_name(r.head().rel()).to_owned()).collect();
        assert!(heads.iter().any(|h| h == crate::code_rule_dsl::logic::rr::FOUND_NODE), "the finding's node by its clause: {heads:?}");
        let own = super::own_names(&super::super::link::Versioned::of_version(v).unwrap(), &crate::code_rule_dsl::logic::rr::core_names());
        for shared in [crate::code_rule_dsl::logic::rr::FOUND_NODE, crate::code_rule_dsl::logic::rr::FOUND_THROUGH, crate::code_rule_dsl::logic::rr::UNPLACED, crate::code_rule_dsl::logic::rr::PLACED_AT, crate::code_rule_dsl::logic::rr::PLACED_LINE] {
            assert!(!own.contains(shared), "{shared} is the pack's: {own:?}");
        }
        assert!(own.contains("Hit"), "the finding's relation is the program's own: {own:?}");
    }
}
