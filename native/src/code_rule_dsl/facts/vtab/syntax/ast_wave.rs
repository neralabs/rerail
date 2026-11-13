//! A program's `ast` matchers (decision 0106) lowered for the transducer and run as one wave over a batch's tapes
//! (decision 0107b): the engine's `AstWave`, which facts runs where `with_matchers` binds.

use super::*;
use std::collections::HashMap;
use std::sync::Mutex;

/// The process's lowered matchers, by their text and their base policy's: lowered once.
type Lowered = Result<(Arc<crate::code_rule_dsl::transducer::packed::LoweredMatcher>, Vec<crate::code_rule_dsl::matcher::DocumentGate>, Arc<crate::code_rule_dsl::waves::policies::Captor>), String>;
static LOWERED: std::sync::LazyLock<Mutex<HashMap<(String, String), Lowered>>> = std::sync::LazyLock::new(Default::default);

/// The engine's compiler of `ast` matchers (`waves::policies::lower_matcher`), registered at load.
pub(crate) struct Compiler;

impl AstCompiler for Compiler {
    /// Each relation's matcher lowered for the transducer, over the program's first `.policy` (its utils). A matcher
    /// that does not lower fails the run, naming it.
    fn compile(&self, matchers: &[(String, String, Vec<String>)], policies: &[serde_json::Value]) -> Result<Matchers, String> {
        compiled(matchers, policies)
    }
}

fn compiled(matchers: &[(String, String, Vec<String>)], policies: &[serde_json::Value]) -> Result<crate::code_rule_dsl::facts::vtab::syntax::Matchers, String> {
    let base = policies.first();
    let base_text = base.map(|b| b.to_string()).unwrap_or_default();
    matchers
        .iter()
        .map(|(rel, text, captures)| {
            let key = (text.clone(), base_text.clone());
            let got = {
                let mut kept = LOWERED.lock().unwrap_or_else(|p| p.into_inner());
                kept.entry(key)
                    .or_insert_with(|| crate::code_rule_dsl::waves::policies::lower_matcher(text, base).map(|(m, g, c)| (Arc::new(m), g, Arc::new(c))))
                    .clone()
            };
            let (lowered, gates, captor) = got.map_err(|why| format!("{rel}: the matcher {text} does not lower: {why}"))?;
            Ok(Matcher { rel: rel.clone(), lowered, gates, captures: captures.clone(), captor })
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
    /// The metavariables the atom captures, in its relation's column order after `n` (decision 0107b).
    pub(crate) captures: Vec<String>,
    pub(in crate::code_rule_dsl) captor: Arc<crate::code_rule_dsl::waves::policies::Captor>,
}

/// A program's matchers (decision 0106) and the wave they run as: every matcher's labels and roots in one run per
/// chunk (decision 0107b).
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
        batch: &Batch,
        tapes: &[(Arc<Facts>, crate::code_rule_dsl::tape::TapeDoc, String)],
        binding: crate::code_rule_dsl::typing::file_local::Binding,
        atoms: &crate::content_store::atoms::Atoms,
    ) -> StoreResult<Vec<(String, Arc<dyn Source>)>> {
        matched(batch, tapes, self, binding, atoms)
    }
}

/// A chunk's matcher wave as one launch of the executor (decision 0107b), as every other dispatch: priced by its tape
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

/// The cores' interpreter: seconds a tape node and pass (one thread), and the GPU's. Priors, replaced by the
/// executor's learned rates once it has measured the form.
const WAVE_CPU_PER_NODE_PASS: f64 = 2e-8;
const WAVE_GPU_PER_NODE_PASS: f64 = 1e-9;

impl MatcherWave {
    fn on(&self, backend: crate::code_rule_dsl::transducer::packed::MatcherBackend) -> WaveFound {
        let started = std::time::Instant::now();
        let (found, run) = self.wave.roots_batch(backend, &self.tapes, &self.active).map_err(|e| format!("{e:?}"))?;
        Ok((found, run, started.elapsed().as_secs_f64()))
    }
}

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

    fn run(&self, lane: rerail_accel::Lane) -> Result<rerail_accel::kernel::ProgramOutput, String> {
        use crate::code_rule_dsl::transducer::packed::MatcherBackend;
        #[cfg(target_os = "macos")]
        if lane == rerail_accel::Lane::Gpu
            && let Ok(found) = self.on(MatcherBackend::MetalInterpreter)
        {
            return Ok(Arc::new(Ok(found) as WaveFound));
        }
        let _ = lane;
        Ok(Arc::new(self.on(MatcherBackend::Cpu)))
    }
}

/// Each matcher's relation over the documents' tapes: the chunk's matcher wave as one launch ([`MatcherWave`]), its
/// roots mapped from tape preorder (a core id) to the batch's node ids. A document with no tape (a guest, a text-only
/// file) or one a matcher's gates exclude has no rows of that matcher.
fn matched(
    batch: &Batch,
    tapes: &[(Arc<Facts>, crate::code_rule_dsl::tape::TapeDoc, String)],
    matchers: &Wave,
    binding: crate::code_rule_dsl::typing::file_local::Binding,
    atoms: &crate::content_store::atoms::Atoms,
) -> StoreResult<Vec<(String, Arc<dyn Source>)>> {
    use crate::code_rule_dsl::logic::profile;
    let _span = profile::span(|| "ast matchers on the transducer".into());
    let Some(wave) = &matchers.wave else { return Ok(Vec::new()) };
    let metal = matches!(binding, crate::code_rule_dsl::typing::file_local::Binding::Metal);
    // Each tape's document in the batch; the tapes outside it take no part.
    let at: Vec<Option<&Projected>> = tapes.iter().map(|(facts, _, _)| batch.docs.iter().find(|p| Arc::ptr_eq(&p.facts, facts))).collect();
    let held: Vec<usize> = (0..tapes.len()).filter(|&i| at[i].is_some()).collect();
    // A matcher runs where its gates hold and W admits the file: its `syntax` (valid or invalid) and text literals.
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
    let (found, limited) = if docs.is_empty() || active.iter().all(|a| !a.iter().any(|&on| on)) {
        (vec![vec![BTreeSet::new(); docs.len()]; matchers.len()], Vec::new())
    } else {
        let program = MatcherWave { wave: Arc::clone(wave), tapes: docs, active, gpu: cfg!(target_os = "macos") && metal };
        let kernel = rerail_accel::Kernel::Program(rerail_accel::kernel::ProgramKernel(Arc::new(program)));
        let ex = crate::code_rule_dsl::logic::lanes::executor_for(metal);
        let (mut done, _) = crate::code_rule_dsl::logic::lanes::prof::run_on(ex, "ast matcher wave", rerail_accel::Batch(vec![rerail_accel::Launch::new(kernel)])).map_err(StoreError::Invalid)?;
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
        if !run.limited.is_empty() {
            profile::decision("ast matcher roots over the work limit (confirmed by W)", || format!("{} matcher-files", run.limited.len()));
        }
        (found, run.limited)
    };
    profile::work(profile::Unit::Node, nodes);
    let mut out = Vec::new();
    for (c, (m, found)) in matchers.list.iter().zip(found).enumerate() {
        let columns = crate::code_rule_dsl::logic::plan::corpus::ast_columns(&m.rel);
        let mut r = Relation::new(columns);
        let mut rows = 0u64;
        // A tape where this matcher's program outran the work limit gave every candidate root: W confirms them there.
        let over: Vec<usize> = limited.iter().filter(|&&(at, _)| at == c).map(|&(_, t)| t).collect();
        if m.captures.is_empty() && !m.captor.verify && !over.is_empty() {
            let _confirm = profile::span(|| "ast roots over the work limit, confirmed (W's matcher at each root)".into());
            let held_over: Vec<usize> = over.iter().map(|&t| held[t]).collect();
            let found_over: Vec<BTreeSet<u32>> = over.iter().map(|&t| found[t].clone()).collect();
            let (found_rows, budget) = captured_rows(m, &held_over, &found_over, &at, tapes, atoms)?;
            if budget > 0 {
                profile::decision("ast captures over a file's visit budget (no rows)", || format!("{}: {budget} roots", m.rel));
            }
            for row in found_rows {
                r.push(&row);
                rows += 1;
            }
        }
        if m.captures.is_empty() && !m.captor.verify {
            for (t, (&i, found)) in held.iter().zip(found).enumerate() {
                if over.contains(&t) {
                    continue;
                }
                let p = at[i].expect("held");
                for core in found {
                    let local = p.local.get(core as usize).copied().unwrap_or(NONE);
                    if local != NONE {
                        r.push(&[p.global(local)]);
                        rows += 1;
                    }
                }
            }
        } else {
            // Captures, or a relaxed matcher's roots to confirm (decision 0107b): W's matcher at each root.
            let _captures = profile::span(|| "ast captures and confirmations (W's matcher at each root)".into());
            let roots: usize = found.iter().map(BTreeSet::len).sum();
            let (found_rows, limited) = captured_rows(m, &held, &found, &at, tapes, atoms)?;
            if limited > 0 {
                profile::decision("ast captures over a file's visit budget (no rows)", || format!("{}: {limited} roots", m.rel));
            }
            // A relaxed matcher's cost (decision 0107b §3): the roots W confirmed on the host, and the rows it kept.
            if m.captor.verify {
                crate::code_rule_dsl::prof::count("ast.relaxed_roots", roots);
                crate::code_rule_dsl::prof::count("ast.relaxed_kept", found_rows.len());
                profile::decision("relaxed matchers: roots W confirmed, rows kept", || format!("{}: {roots} roots, {} kept", m.rel, found_rows.len()));
            }
            for row in found_rows {
                r.push(&row);
                rows += 1;
            }
        }
        crate::code_rule_dsl::prof::count("ast.matched", rows as usize);
        out.push((m.rel.clone(), Arc::new(r) as Arc<dyn Source>));
    }
    Ok(out)
}

/// A captured matcher's rows (decision 0107b): at each root the transducer matched, W's matcher of the atom's rule
/// matched again on the host, and each capture's nodes read from its bindings (a multi-capture's every node, so its
/// rows are the product of the captures' nodes). A node the projection leaves out has no row. Tapes in parallel; also
/// how many roots hit their file's visit budget (no rows).
fn captured_rows(
    m: &Matcher,
    held: &[usize],
    found: &[BTreeSet<u32>],
    at: &[Option<&Projected>],
    tapes: &[(Arc<Facts>, crate::code_rule_dsl::tape::TapeDoc, String)],
    atoms: &crate::content_store::atoms::Atoms,
) -> StoreResult<(Vec<Vec<i64>>, usize)> {
    use crate::code_rule_dsl::waves::findings::Capture;
    use rayon::prelude::*;
    let per: Vec<StoreResult<(Vec<Vec<i64>>, usize)>> = held
        .par_iter()
        .zip(found.par_iter())
        .map(|(&i, roots)| {
            let mut rows = Vec::new();
            let mut limited = 0;
            if roots.is_empty() {
                return Ok((rows, limited));
            }
            let p = at[i].expect("held");
            let tape = &tapes[i].1;
            // W's matcher reads the file's path (`filePath`, which a relaxed matcher leaves to it).
            let _file = crate::code_rule_dsl::matcher::FileScopeSlot::install(Vec::new(), None, !tape.syntax_error_ranges().is_empty()).path(&tapes[i].2);
            let root = ast_grep_core::AstGrep::doc(tape.clone());
            let work = m.captor.limits.visits_for(tape.transducer_len());
            let global = |core: u32| -> Option<i64> {
                let local = p.local.get(core as usize).copied().unwrap_or(NONE);
                (local != NONE).then(|| p.global(local))
            };
            for &n in roots {
                let Some(n_global) = global(n) else { continue };
                let names = match crate::code_rule_dsl::waves::findings::captured(&m.captor.grammar, &root, n, &m.captures, work) {
                    Ok(Some(names)) => names,
                    Ok(None) => continue,
                    Err(_) => {
                        limited += 1;
                        continue;
                    }
                };
                let mut product: Vec<Vec<i64>> = vec![vec![n_global]];
                for capture in names {
                    // A transform's text is a symbol, the store's atom, as the program's constants are.
                    let nodes: Vec<i64> = match capture {
                        Capture::Nodes(nodes) => nodes.into_iter().filter_map(global).collect(),
                        Capture::Text(text) => vec![i64::from(atoms.intern(&text)?)],
                    };
                    product = product.into_iter().flat_map(|row| nodes.iter().map(move |&x| {
                        let mut row = row.clone();
                        row.push(x);
                        row
                    })).collect();
                }
                rows.extend(product);
            }
            Ok((rows, limited))
        })
        .collect();
    let per = per.into_iter().collect::<StoreResult<Vec<_>>>()?;
    let limited = per.iter().map(|(_, l)| l).sum();
    Ok((per.into_iter().flat_map(|(rows, _)| rows).collect(), limited))
}

