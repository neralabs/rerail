//! A program's `ast` matchers (decision 0106) lowered for the transducer and run as one wave over a batch's tapes
//! (decision 0107): the engine's `AstWave`, which facts runs where `with_matchers` binds.

use rerail_blocks::vector::Vector;
use super::*;
use std::collections::HashMap;
use std::sync::Mutex;

/// The process's lowered matchers, by their text and their base policy's: lowered once.
type Lowered = Result<(Arc<crate::code_rule_dsl::transducer::packed::LoweredMatcher>, Vec<crate::code_rule_dsl::matcher::DocumentGate>, Arc<crate::code_rule_dsl::waves::policies::Captor>), String>;
#[derive(Default)]
struct LoweredWaves(Mutex<HashMap<(String, String), Lowered>>);

/// The engine's compiler of `ast` matchers (`waves::policies::lower_matcher`), registered at load.
pub(crate) struct Compiler;

impl AstCompiler for Compiler {
    /// Each relation's matcher lowered for the transducer, over the program's first `.policy` (its utils). A matcher
    /// that does not lower fails the run, naming it.
    fn compile(&self, matchers: &[(String, String, Vec<String>)], demand: &[Option<rerail_blocks::vector::Vector<usize>>], policies: &[serde_json::Value]) -> Result<Matchers, String> {
        compiled(matchers, demand, policies)
    }
}

fn compiled(matchers: &[(String, String, Vec<String>)], demand: &[Option<rerail_blocks::vector::Vector<usize>>], policies: &[serde_json::Value]) -> Result<crate::code_rule_dsl::facts::vtab::syntax::Matchers, String> {
    let base = policies.first();
    let base_text = base.map(|b| b.to_string()).unwrap_or_default();
    matchers
        .iter()
        .enumerate()
        .map(|(k, (rel, text, captures))| {
            let key = (text.clone(), base_text.clone());
            let got = {
                let held_ = crate::code_rule_dsl::logic::profile::shared::<LoweredWaves>();
                let mut kept = held_.0.lock().unwrap_or_else(|p| p.into_inner());
                kept.entry(key)
                    .or_insert_with(|| crate::code_rule_dsl::waves::policies::lower_matcher(text, base).map(|(m, g, c)| (Arc::new(m), g, Arc::new(c))))
                    .clone()
            };
            let (lowered, gates, captor) = got.map_err(|why| format!("{rel}: the matcher {text} does not lower: {why}"))?;
            let direct = field_captures(text, captures);
            Ok(Matcher { rel: rel.clone(), lowered, gates, captures: captures.clone(), captor, demanded_by: demand.get(k).cloned().flatten(), direct })
        })
        .collect::<Result<Vec<_>, String>>()
        .and_then(Wave::new)
        .map(|wave| Matchers::new(Arc::new(wave)))
}


/// A program's `ast` matcher (decision 0106): the relation of its matched nodes, `rr_ast_<k>(n)`, its matcher lowered
/// for the transducer, and the conditions on a document's path and text it runs under.
pub(crate) struct Matcher {
    pub(crate) rel: String,
    pub(in crate::code_rule_dsl) lowered: Arc<crate::code_rule_dsl::transducer::packed::LoweredMatcher>,
    pub(in crate::code_rule_dsl) gates: Vec<crate::code_rule_dsl::matcher::DocumentGate>,
    /// The metavariables the atom captures, in its relation's column order after `n` (decision 0107).
    pub(crate) captures: Vec<String>,
    pub(in crate::code_rule_dsl) captor: Arc<crate::code_rule_dsl::waves::policies::Captor>,
    /// The matchers (by place in the wave) whose roots hold every node this one's rows are read at
    /// (`matchers::demanded_by_roots`, decision 0459): its rows are made only there. `None`: at every root.
    pub(crate) demanded_by: Option<rerail_blocks::vector::Vector<usize>>,
    /// Where every capture is a field child of the root, `{"has": {"field": F, "kind": K, "pattern": "$C", "stopBy":
    /// "neighbor"}}` in the rule's top `all` (as `Absorb` writes them, decision 0106 §4): each capture's field and
    /// kind. Its rows are then read from the tape at each root the transducer matched, not by W's matcher there.
    pub(crate) direct: Option<Vec<(String, String)>>,
}

/// A matcher's captures as its root's field children ([`Matcher::direct`]), where its text has them so: every
/// capture bound once, by a `has` of a field and a kind and nothing more, in the rule's top `all`; the matcher with
/// no `transform`, `constraints` or `utils`.
fn field_captures(text: &str, captures: &[String]) -> Option<Vec<(String, String)>> {
    if captures.is_empty() || crate::code_rule_dsl::logic::profile::var("RERAIL_AST_DIRECT_CAPTURES").is_ok_and(|v| v == "off") {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let top = v.as_object()?;
    if top.keys().any(|k| !matches!(k.as_str(), "language" | "rule" | "syntax")) {
        return None;
    }
    let all = top.get("rule")?.as_object().filter(|r| r.len() == 1)?.get("all")?.as_array()?;
    captures
        .iter()
        .map(|c| {
            let name = format!("${c}");
            let binding = |part: &serde_json::Value| -> Option<(String, String)> {
                let has = part.as_object().filter(|p| p.len() == 1)?.get("has")?.as_object()?;
                if has.keys().any(|k| !matches!(k.as_str(), "field" | "kind" | "pattern" | "stopBy")) || has.get("pattern")?.as_str()? != name || has.get("stopBy")?.as_str()? != "neighbor" {
                    return None;
                }
                Some((has.get("field")?.as_str()?.to_owned(), has.get("kind")?.as_str()?.to_owned()))
            };
            let bound: Vec<(String, String)> = all.iter().filter_map(binding).collect();
            // Bound once, and named nowhere else in the rule.
            (bound.len() == 1 && text.matches(name.as_str()).count() == 1).then(|| bound.into_iter().next()).flatten()
        })
        .collect()
}

/// A [`Matcher::direct`] matcher's rows at the transducer's `roots` of the tape: the root and each capture's first
/// child in its field, of its kind. A node the projection leaves out has no row, as `captured_rows`'.
fn field_captured_rows(fields: &[(String, String)], tape: &crate::code_rule_dsl::tape::TapeDoc, p: &Projected, roots: &BTreeSet<u32>, out: &mut Relation) -> u64 {
    let grammar = tape.grammar();
    let Some(codes) = fields.iter().map(|(f, _)| grammar.field_code(f)).collect::<Option<Vec<u16>>>() else { return 0 };
    let global = |core: u32| {
        let local = p.local_of_node(core);
        (local != NONE).then(|| p.global(local))
    };
    let mut rows = 0;
    let mut row = Vec::with_capacity(fields.len() + 1);
    for &root in roots {
        row.clear();
        let Some(r) = global(root) else { continue };
        row.push(r);
        for (code, (_, kind)) in codes.iter().zip(fields) {
            let child = tape.transducer_children(root).find(|&c| tape.transducer_field(c) == *code && grammar.kind_name(tape.transducer_kind(c)) == Some(kind.as_str()));
            match child.and_then(global) {
                Some(g) => row.push(g),
                None => break,
            }
        }
        if row.len() == fields.len() + 1 {
            out.push(&row);
            rows += 1;
        }
    }
    rows
}

/// A program's matchers (decision 0106) and the wave they run as: every matcher's labels and roots in one run per
/// chunk (decision 0107).
pub(crate) struct Wave {
    pub(crate) list: Vec<Matcher>,
    pub(in crate::code_rule_dsl) wave: Option<Arc<crate::code_rule_dsl::transducer::packed::LoweredWave>>,
}

impl Wave {
    /// `list` as one wave (labels shared by matchers are one pass).
    pub(in crate::code_rule_dsl) fn new(list: Vec<Matcher>) -> Result<Self, String> {
        let wave = if list.is_empty() {
            None
        } else {
            let clauses: Vec<&crate::code_rule_dsl::transducer::packed::LoweredMatcher> = list.iter().map(|m| m.lowered.as_ref()).collect();
            Some(Arc::new(crate::code_rule_dsl::transducer::packed::LoweredWave::new(&clauses).map_err(|e| format!("the ast matchers as one wave: {e:?}"))?))
        };
        Ok(Self { list, wave })
    }
}

impl AstWave for Wave {
    fn len(&self) -> usize {
        self.list.len()
    }
    fn matched(
        &self,
        tapes: &[(Arc<Facts>, crate::code_rule_dsl::tape::TapeDoc, String)],
        at: &[Option<&Projected>],
        binding: crate::code_rule_dsl::typing::file_local::Binding,
        atoms: &dyn crate::content_store::AtomTable,
    ) -> StoreResult<Vec<(String, Relation)>> {
        matched(tapes, at, self, binding, atoms)
    }
    /// A capture's column holds nodes of the matched document (`captured_rows`' `global`), unless the atom's rule
    /// `transform`s it into text (a symbol).
    fn node_columns(&self) -> Vec<(String, Vec<usize>)> {
        self.list
            .iter()
            .map(|m| {
                let transformed = |name: &str| m.captor.grammar.transform.as_ref().is_some_and(|t| t.contains(name));
                (m.rel.clone(), m.captures.iter().enumerate().filter(|(_, c)| !transformed(c)).map(|(i, _)| i + 1).collect())
            })
            .collect()
    }
}

/// A chunk's matcher wave as one launch of the executor (decision 0107), as every other dispatch: priced by its tape
/// nodes and passes, placed on the cores or the GPU, run on that lane's worker and scored. Its GPU form is the wave's
/// passes and roots in one command buffer; its cores' form the interpreter, which the GPU's runs instead where the
/// device refuses the batch.
struct MatcherWave {
    wave: Arc<crate::code_rule_dsl::transducer::packed::LoweredWave>,
    tapes: Vec<crate::code_rule_dsl::tape::TapeDoc>,
    /// Per matcher, per tape: whether the matcher runs there (its gates hold).
    active: Vec<Vec<bool>>,
    gpu: bool,
}

/// What a wave's launch returns: per matcher, per tape, the matched roots; how it ran; its wall seconds on the lane.
type WaveFound = Result<(Vec<Vec<BTreeSet<u32>>>, crate::code_rule_dsl::transducer::packed::WaveRun, f64), String>;

/// The cores' interpreter: seconds a tape node and pass (one thread), and the GPU's: priors, until a run has measured
/// its own launches (`price::observe`: a line of a launch's fixed cost and a unit's). Measured 2026-11-17 on s1000
/// (M5, format 16, the structured MSL, the machine's load 40 to 110): the GPU 6.2 ms a launch + 1.5e-8 s a unit of
/// wall, 4.3 ms + 3.7e-9 s of host CPU; the cores 7 launches of 7.1e6 units in 0.77 s, 1.1e-7 s a unit under a load
/// of about 50 on 18 threads, so about 4e-8 unloaded (the price scales the cores by their load itself). The
/// GPU's was 2.5e-8 and its host CPU nothing; the cores' 2e-8 (2026-11-16, before format 16 and the structured MSL).
const WAVE_CPU_PER_NODE_PASS: f64 = 4e-8;
const WAVE_GPU_PER_NODE_PASS: f64 = 1.5e-8;
const WAVE_GPU_HOST_PER_NODE_PASS: f64 = 4e-9;

impl MatcherWave {
    fn on(&self, backend: crate::code_rule_dsl::transducer::packed::MatcherBackend) -> WaveFound {
        use crate::code_rule_dsl::transducer::packed::MatcherBackend;
        let started = std::time::Instant::now();
        // On the cores, parts of the tapes side by side: a tape's labels and roots read that tape alone, so each part's
        // roots are the whole's at its tapes (a corpus batch's wave over every file ran on one core, 8 s on t50's
        // product run). Each part's limited roots are placed back at their tapes.
        let grain = self.tapes.len().div_ceil(crate::code_rule_dsl::logic::profile::cores_threads() * 4).max(WAVE_PART_TAPES);
        if !matches!(backend, MatcherBackend::Cpu) || self.tapes.len() <= grain {
            let (found, run) = self.wave.roots_batch(backend, &self.tapes, &self.active).map_err(|e| format!("{e:?}"))?;
            return Ok((found, run, started.elapsed().as_secs_f64()));
        }
        use rayon::prelude::*;
        let starts: Vec<usize> = (0..self.tapes.len()).step_by(grain).collect();
        let parts: Vec<Result<_, String>> = crate::code_rule_dsl::logic::profile::on_cores(|| {
            starts
                .par_iter()
                .map(crate::code_rule_dsl::logic::profile::carried("ast matcher wave part (pool)", |&at: &usize| {
                    let end = (at + grain).min(self.tapes.len());
                    let active: Vec<Vec<bool>> = self.active.iter().map(|a| a[at..end].to_vec()).collect();
                    self.wave.roots_batch(backend, &self.tapes[at..end], &active).map_err(|e| format!("{e:?}"))
                }))
                .collect()
        });
        let mut found: Vec<Vec<std::collections::BTreeSet<u32>>> = vec![Vec::with_capacity(self.tapes.len()); self.active.len()];
        let mut run = crate::code_rule_dsl::transducer::packed::WaveRun::default();
        for (&at, part) in starts.iter().zip(parts) {
            let (part_found, part_run) = part?;
            for (all, theirs) in found.iter_mut().zip(part_found) {
                all.extend(theirs);
            }
            run.limited.extend(part_run.limited.into_iter().map(|(c, t)| (c, t + at)));
            run.roots_cpu.resize(part_run.roots_cpu.len().max(run.roots_cpu.len()), 0);
            for (all, theirs) in run.roots_cpu.iter_mut().zip(&part_run.roots_cpu) {
                *all += theirs;
            }
        }
        Ok((found, run, started.elapsed().as_secs_f64()))
    }
}

/// The fewest tapes a part of a CPU wave has: a part labels its tapes as one pass, whose setup a few would not carry
/// (parts of 16 tapes took a chunk's wave from one pass to several, and the waves' CPU from 12 s to 24 s on t50's
/// product run). A chunk's wave is one part; a corpus batch's, of every file, is split.
const WAVE_PART_TAPES: usize = 256;

impl rerail_accel::kernel::Program for MatcherWave {
    fn name(&self) -> &'static str {
        "ast matcher wave"
    }

    fn units(&self) -> f64 {
        let passes = self.wave.passes().max(1) as f64;
        self.tapes.iter().map(|t| t.transducer_len() as f64).sum::<f64>() * passes
    }

    fn per_unit(&self, lane: rerail_accel::Lane) -> Option<f64> {
        match lane {
            rerail_accel::Lane::Cpu => Some(WAVE_CPU_PER_NODE_PASS),
            rerail_accel::Lane::Gpu if self.gpu => Some(WAVE_GPU_PER_NODE_PASS),
            _ => None,
        }
    }

    fn host_per_unit(&self, lane: rerail_accel::Lane) -> Option<f64> {
        match lane {
            rerail_accel::Lane::Gpu => self.per_unit(lane).map(|_| WAVE_GPU_HOST_PER_NODE_PASS),
            _ => self.per_unit(lane),
        }
    }

    fn run(&self, lane: rerail_accel::Lane) -> Result<rerail_accel::kernel::ProgramOutput, String> {
        use crate::code_rule_dsl::transducer::packed::MatcherBackend;
        #[cfg(any(target_os = "macos", target_os = "linux", windows))]
        if lane == rerail_accel::Lane::Gpu {
            // A Metal run that fails stops the run (decision 0382 §6): no CPU wave in its place. What does not fit is
            // split, and work past its budget replanned, inside the run (0334).
            let found = self.on(MatcherBackend::MetalInterpreter).unwrap_or_else(|e| panic!("rerail: the ast matcher wave failed on Metal: {e} ({} tapes; the device: {})", self.tapes.len(), rerail_accel::gpu::shared::describe()));
            // The wave's GPU form is the Metal interpreter: no generated kernel (decision 0202).
            rerail_accel::forms::missing("metal-jit-form", "ast matcher wave", "ast matcher wave", "the wave runs the Metal interpreter: no generated MSL", 1);
            return Ok(Arc::new(Ok(found) as WaveFound));
        }
        let _ = lane;
        // The wave's CPU form is the interpreter: no Cranelift form of its roots (decision 0202).
        rerail_accel::forms::missing("cpu-jit-form", "ast matcher wave", "ast matcher wave", "the wave runs the CPU interpreter: no Cranelift form", 1);
        Ok(Arc::new(self.on(MatcherBackend::Cpu)))
    }
}

/// Each matcher's relation over the documents' tapes: the chunk's matcher wave as one launch ([`MatcherWave`]), its
/// roots mapped from tape preorder (a core id) to the batch's node ids. A document with no tape (a guest, a text-only
/// file) or one a matcher's gates exclude has no rows of that matcher.
fn matched(
    tapes: &[(Arc<Facts>, crate::code_rule_dsl::tape::TapeDoc, String)],
    at: &[Option<&Projected>],
    matchers: &Wave,
    binding: crate::code_rule_dsl::typing::file_local::Binding,
    atoms: &dyn crate::content_store::AtomTable,
) -> StoreResult<Vec<(String, Relation)>> {
    use crate::code_rule_dsl::logic::profile;
    let _span = profile::span(|| "ast matchers on the transducer".into());
    let Some(wave) = &matchers.wave else { return Ok(Vec::new()) };
    let metal = matches!(binding, crate::code_rule_dsl::typing::file_local::Binding::Metal);
    // Each tape's document in the batch (`at`, the caller's); the tapes outside it take no part.
    let held: Vector<usize> = (0..tapes.len()).filter(|&i| at[i].is_some()).collect();
    // A matcher runs where its gates hold and W admits the file: its `syntax` (valid or invalid) and text literals.
    let gates = profile::span(|| "ast matcher gates".into());
    let errors: Vec<crate::code_rule_dsl::soundness::SyntaxErrors> = held.iter().map(|&i| crate::code_rule_dsl::soundness::SyntaxErrors::of_doc(&tapes[i].1)).collect();
    let active: Vec<Vec<bool>> = matchers
        .list
        .iter()
        .map(|m| {
            held.iter()
                .zip(&errors)
                .map(|(&i, errors)| {
                    let source = tapes[i].0.source.as_str();
                    m.gates.iter().all(|g| g.holds(&tapes[i].2, source)) && m.captor.grammar.admits_source(source, errors, &mut rustc_hash::FxHashMap::default())
                })
                .collect()
        })
        .collect();
    // Each read at its path, which a `filePath` test inside a matcher tests.
    let docs: Vec<crate::code_rule_dsl::tape::TapeDoc> = held.iter().map(|&i| tapes[i].1.with_path(&tapes[i].2)).collect();
    let nodes: u64 = docs.iter().map(|d| d.transducer_len() as u64).sum();
    drop(gates);
    let run_span = profile::span(|| "ast matcher wave run".into());
    let (found, limited) = if docs.is_empty() || active.iter().all(|a| !a.iter().any(|&on| on)) {
        (vec![vec![BTreeSet::new(); docs.len()]; matchers.len()], Vec::new())
    } else {
        // The GPU's form only once its generated kernels are at hand (decision 0395): till then, the cores'.
        // The batch's one tape format, the label kernels' constant (`DocTable::format`: 16, for 16 and 16z).
        let format = || 16;
        let gpu = cfg!(any(target_os = "macos", target_os = "linux", windows)) && metal && wave.metal_ready(format());
        let program = MatcherWave { wave: Arc::clone(wave), tapes: docs, active, gpu };
        let kernel = rerail_accel::Kernel::Program(rerail_accel::kernel::ProgramKernel(Arc::new(program)));
        let ex = crate::code_rule_dsl::logic::lanes::executor_for(metal);
        let (mut done, _) = crate::code_rule_dsl::logic::lanes::prof::run_on(&ex, "ast matcher wave", rerail_accel::Batch(vec![rerail_accel::Launch::new(kernel)])).map_err(StoreError::Invalid)?;
        let done = done.remove(0);
        let rerail_accel::kernel::Output::Program(r) = done.out else {
            return Err(StoreError::Invalid("ast matcher wave: not a program's result".into()));
        };
        let r = r.0.downcast::<WaveFound>().map_err(|_| StoreError::Invalid("ast matcher wave: a result of another type".into()))?;
        let (found, run, wall) = r.as_ref().clone().map_err(|e| StoreError::Invalid(format!("ast matcher wave: {e}")))?;
        if run.metal {
            profile::device("ast matcher wave", run.command_buffers as u64, run.device, wall, nodes);
        }
        profile::decision("ast matcher wave lane", || (if run.metal { "metal" } else { "cpu" }).to_owned());
        if crate::code_rule_dsl::prof::on() {
            for (m, ns) in matchers.list.iter().zip(&run.roots_cpu) {
                crate::code_rule_dsl::prof::count_named(format!("wave.roots_us.{}", m.rel), (*ns / 1000) as usize);
            }
            for (m, per) in matchers.list.iter().zip(&found) {
                crate::code_rule_dsl::prof::count_named(format!("wave.roots_n.{}", m.rel), per.iter().map(BTreeSet::len).sum());
            }
        }
        if !run.limited.is_empty() {
            profile::decision("ast matcher roots over the work limit (confirmed by W)", || format!("{} matcher-files", run.limited.len()));
        }
        (found, run.limited)
    };
    profile::work(profile::Unit::Node, nodes);
    drop(run_span);
    let _rows_span = profile::span(|| "ast matcher rows".into());
    // A matcher read only where other matchers' roots are (decision 0459): its roots there alone, before any capture.
    let mut demanded: Vec<Option<Vec<BTreeSet<u32>>>> = matchers
        .list
        .iter()
        .map(|m| {
            let by = m.demanded_by.as_ref()?;
            Some((0..held.len()).map(|t| by.iter().flat_map(|&b| found[b].get(t)).flatten().copied().collect::<BTreeSet<u32>>()).collect())
        })
        .collect();
    let found: Vec<Vec<BTreeSet<u32>>> = found
        .into_iter()
        .zip(&mut demanded)
        .map(|(roots, demand)| match demand.take() {
            Some(demand) => {
                let kept: Vec<BTreeSet<u32>> = roots.iter().zip(&demand).map(|(roots, demand)| roots.intersection(demand).copied().collect()).collect();
                crate::code_rule_dsl::prof::count("ast.demanded_dropped", roots.iter().map(BTreeSet::len).sum::<usize>() - kept.iter().map(BTreeSet::len).sum::<usize>());
                kept
            }
            None => roots,
        })
        .collect();
    let mut out = Vec::new();
    for (c, (m, found)) in matchers.list.iter().zip(found).enumerate() {
        let columns = crate::code_rule_dsl::logic::plan::corpus::ast_columns(&m.rel);
        let mut r = Relation::new(columns);
        let mut rows = 0u64;
        // A tape where this matcher's program outran the work limit gave every candidate root: W confirms them there.
        let over: Vector<usize> = limited.iter().filter(|&&(at, _)| at == c).map(|&(_, t)| t).collect();
        if m.captures.is_empty() && !m.captor.verify && !over.is_empty() {
            let _confirm = profile::span(|| "ast roots over the work limit, confirmed (W's matcher at each root)".into());
            let held_over: Vector<usize> = over.iter().map(|&t| held[t]).collect();
            let found_over: Vec<BTreeSet<u32>> = over.iter().map(|&t| found[t].clone()).collect();
            let (found_rows, budget, _) = captured_rows(m, &held_over, &found_over, at, tapes, atoms, Some(wave.w_key(c)))?;
            if budget > 0 {
                profile::decision("ast captures over a file's visit budget (no rows)", || format!("{}: {budget} roots", m.rel));
            }
            for row in found_rows.chunks_exact(1 + m.captures.len()) {
                r.push(row);
                rows += 1;
            }
        }
        if let (Some(fields), false, true) = (&m.direct, m.captor.verify, over.is_empty()) {
            let _direct = profile::span(|| "ast captures read as field children".into());
            for (&i, found) in held.iter().zip(&found) {
                rows += field_captured_rows(fields, &tapes[i].1, at[i].expect("held"), found, &mut r);
            }
        } else if m.captures.is_empty() && !m.captor.verify {
            for (t, (&i, found)) in held.iter().zip(found).enumerate() {
                if over.contains(&t) {
                    continue;
                }
                let p = at[i].expect("held");
                for core in found {
                    let local = p.local_of_node(core);
                    if local != NONE {
                        r.push(&[p.global(local)]);
                        rows += 1;
                    }
                }
            }
        } else {
            // Captures, or a relaxed matcher's roots to confirm (decision 0107): W's matcher at each root.
            let _captures = profile::span(|| "ast captures and confirmations (W's matcher at each root)".into());
            let roots: usize = found.iter().map(BTreeSet::len).sum();
            let (found_rows, limited, cpu) = captured_rows(m, &held, &found, at, tapes, atoms, Some(wave.w_key(c)))?;
            // What the host's confirmation costs each relation: its roots, the rows kept and the pool's CPU.
            if crate::code_rule_dsl::prof::on() {
                crate::code_rule_dsl::prof::count_named(format!("wave.captures_us.{}", m.rel), (cpu / 1000) as usize);
                profile::decision("ast captures by W at each root: roots, rows, captures, CPU ms", || {
                    format!("{}: {roots} roots, {} rows, {} captures, {:.1} ms", m.rel, found_rows.len() / (1 + m.captures.len()), m.captures.len(), cpu as f64 / 1e6)
                });
            }
            if limited > 0 {
                profile::decision("ast captures over a file's visit budget (no rows)", || format!("{}: {limited} roots", m.rel));
            }
            // A relaxed matcher's cost (decision 0107 §3): the roots W confirmed on the host, and the rows it kept.
            if m.captor.verify {
                crate::code_rule_dsl::prof::count("ast.relaxed_roots", roots);
                crate::code_rule_dsl::prof::count("ast.relaxed_kept", found_rows.len() / (1 + m.captures.len()));
                profile::decision("relaxed matchers: roots W confirmed, rows kept", || format!("{}: {roots} roots, {} kept", m.rel, found_rows.len() / (1 + m.captures.len())));
            }
            for row in found_rows.chunks_exact(1 + m.captures.len()) {
                r.push(row);
                rows += 1;
            }
        }
        crate::code_rule_dsl::prof::count("ast.matched", rows as usize);
        out.push((m.rel.clone(), r));
    }
    Ok(out)
}

/// A captured matcher's rows (decision 0107): at each root the transducer matched, W's matcher of the atom's rule
/// matched again on the host, and each capture's nodes read from its bindings (a multi-capture's every node, so its
/// rows are the product of the captures' nodes). A node the projection leaves out has no row. Tapes in parallel; also
/// how many roots hit their file's visit budget (no rows). The rows flat, each `1 + m.captures.len()` wide: one vector a
/// tape, not one a row.
fn captured_rows(
    m: &Matcher,
    held: &[usize],
    found: &[BTreeSet<u32>],
    at: &[Option<&Projected>],
    tapes: &[(Arc<Facts>, crate::code_rule_dsl::tape::TapeDoc, String)],
    atoms: &dyn crate::content_store::AtomTable,
    w_key: Option<String>,
) -> StoreResult<(Vector<i64>, usize, u128)> {
    use crate::code_rule_dsl::transducer::packed::evaluation_costs as costs;
    use crate::code_rule_dsl::waves::findings::Capture;
    use rayon::prelude::*;
    // W's steps at the roots and their subtrees' nodes, for its rate in the cost store (decision 0334): what the
    // wave weighs a clause over its budget against. And each tape's CPU, for the profile's captures (t45).
    let measure = w_key.is_some();
    // A tape's rows, its roots past the budget, W's steps and nodes, and its CPU (ns).
    type TapeRun = (Vector<i64>, usize, u64, u64, u128);
    let width = 1 + m.captures.len();
    let per: Vec<StoreResult<TapeRun>> = crate::code_rule_dsl::logic::profile::on_cores(|| {
        held
            .par_iter()
            .zip(found.par_iter())
            .map(crate::code_rule_dsl::logic::profile::carried("ast captures (pool)", |(&i, roots): (&usize, &BTreeSet<u32>)| {
                let mut rows = Vector::new();
                let (mut limited, mut steps, mut nodes) = (0, 0_u64, 0_u64);
                if roots.is_empty() {
                    return Ok((rows, limited, steps, nodes, 0));
                }
                let started = rerail_prof::thread_cpu();
                let p = at[i].expect("held");
                let tape = &tapes[i].1;
                // W's matcher reads the file's path (`filePath`, which a relaxed matcher leaves to it).
                let _file = crate::code_rule_dsl::matcher::FileScopeSlot::install(Vec::new(), None, tape.has_syntax_error()).path(&tapes[i].2);
                let root = ast_grep_core::AstGrep::doc(tape.clone());
                let work = m.captor.limits.visits_for(tape.transducer_len());
                let global = |core: u32| -> Option<i64> {
                    let local = p.local_of_node(core);
                    (local != NONE).then(|| p.global(local))
                };
                // A root's rows as captures are added (`k` wide), its next, and a capture's nodes: reused across roots.
                let (mut product, mut next, mut captured): (Vec<i64>, Vec<i64>, Vec<i64>) = Default::default();
                for &n in roots {
                    let Some(n_global) = global(n) else { continue };
                    let (names, used) = crate::code_rule_dsl::waves::findings::captured_counted(&m.captor.grammar, &root, n, &m.captures, work);
                    if measure {
                        steps += used as u64;
                        nodes += u64::from(tape.transducer_descendants(n).end.saturating_sub(n).max(1));
                    }
                    let names = match names {
                        Ok(Some(names)) => names,
                        Ok(None) => continue,
                        Err(_) => {
                            limited += 1;
                            continue;
                        }
                    };
                    product.clear();
                    product.push(n_global);
                    let mut k = 1;
                    for capture in names {
                        // A transform's text is a symbol, the store's atom, as the program's constants are.
                        captured.clear();
                        match capture {
                            Capture::Nodes(found) => captured.extend(found.into_iter().filter_map(global)),
                            Capture::Text(text) => captured.push(i64::from(atoms.intern(&text)?)),
                        }
                        next.clear();
                        for row in product.chunks_exact(k) {
                            for &x in &captured {
                                next.extend_from_slice(row);
                                next.push(x);
                            }
                        }
                        std::mem::swap(&mut product, &mut next);
                        k += 1;
                    }
                    debug_assert_eq!(k, width, "{}: a root's captures, not the matcher's", m.rel);
                    rows.extend_from_slice(&product);
                }
                let cpu = (rerail_prof::thread_cpu() - started).as_nanos();
                Ok((rows, limited, steps, nodes, cpu))
            }))
            .collect()
    });
    let per = per.into_iter().collect::<StoreResult<Vec<_>>>()?;
    let limited = per.iter().map(|(_, l, ..)| l).sum();
    let cpu = per.iter().map(|(.., cpu)| cpu).sum();
    if let Some(key) = w_key.filter(|_| measure) {
        let (steps, nodes) = per.iter().fold((0, 0), |(s, n), (_, _, steps, nodes, _)| (s + steps, n + nodes));
        if steps > 0 {
            costs::observe([costs::Observation { program: key, steps, size: nodes, predicted: 0, over_budget: false }]);
        }
    }
    let mut all = Vector::with_capacity(per.iter().map(|(rows, ..)| rows.len()).sum());
    for (rows, ..) in &per {
        all.extend_from_slice(rows);
    }
    Ok((all, limited, cpu))
}

