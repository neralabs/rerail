//! A program's `ast` matchers (decision 0106) lowered for the transducer and run as one wave over a batch's tapes
//! (decision 0107): the engine's `AstWave`, which facts runs where `with_matchers` binds.

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
    /// The metavariables the atom captures, in its relation's column order after `n` (decision 0107).
    pub(crate) captures: Vec<String>,
    pub(in crate::code_rule_dsl) captor: Arc<crate::code_rule_dsl::waves::policies::Captor>,
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
        batch: &Batch,
        tapes: &[WaveTape],
        binding: crate::code_rule_dsl::typing::file_local::Binding,
        atoms: &crate::content_store::atoms::Atoms,
    ) -> StoreResult<Vec<(String, Arc<dyn Source>)>> {
        matched(batch, tapes, self, binding, atoms)
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
type WaveOutput = (Vec<Vec<BTreeSet<u32>>>, crate::code_rule_dsl::transducer::packed::WaveRun, f64);
type WaveFound = Result<WaveOutput, String>;

/// The cores' interpreter: seconds a tape node and pass (one thread), and the GPU's. Priors, replaced by the
/// executor's learned rates once it has measured the form. Measured over the data layer (M5, under load): the cores
/// 1.1e-8 to 2.4e-8, the GPU's interpreter 2.5e-8 to 2.5e-7 (one wave of large files 5.6e-5 a node): no faster a
/// unit than one core, on one lane against the cores' many.
const WAVE_CPU_PER_NODE_PASS: f64 = 2e-8;
const WAVE_GPU_PER_NODE_PASS: f64 = 2.5e-8;

/// The profile's topic for a wave the GPU lane ran on the CPU, its Metal run having failed (`MatcherWave::run`).
#[cfg(target_os = "macos")]
pub const METAL_FELL_BACK: &str = "ast matcher wave fell back to the CPU (its Metal run failed)";

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
        if lane == rerail_accel::Lane::Gpu {
            match self.on(MatcherBackend::MetalInterpreter) {
                // The CPU's wave instead, named in the profile's warnings: a Metal path that fails (a kernel that does
                // not compile, a run over its bounds) is never silent.
                Ok(found) => {
                    // The wave's GPU form is the Metal interpreter: no generated kernel (decision 0202).
                    rerail_accel::forms::missing("metal-jit-form", "ast matcher wave", "ast matcher wave", "the wave runs the Metal interpreter: no generated MSL", 1);
                    return Ok(Arc::new(Ok(found) as WaveFound));
                }
                Err(e) => {
                    let reason: String = e.lines().next().unwrap_or("").chars().take(160).collect();
                    rerail_accel::forms::missing("metal-form", "ast matcher wave", "ast matcher wave", &reason, 1);
                    crate::code_rule_dsl::logic::profile::decision(METAL_FELL_BACK, || reason.clone())
                }
            }
        }
        let _ = lane;
        // The wave's CPU form is the interpreter: no Cranelift form of its roots (decision 0202).
        rerail_accel::forms::missing("cpu-jit-form", "ast matcher wave", "ast matcher wave", "the wave runs the CPU interpreter: no Cranelift form", 1);
        Ok(Arc::new(self.on(MatcherBackend::Cpu)))
    }
}

/// The tapes a wave runs over, as the batch holds them.
struct Ran<'a> {
    tapes: &'a [WaveTape],
    /// Each tape's document in the batch, by key (guests of one text share their facts); `None` outside it.
    at: Vec<Option<&'a Projected>>,
    /// The tapes whose document the batch holds, in the wave's order: the others take no part.
    held: Vec<usize>,
    atoms: &'a crate::content_store::atoms::Atoms,
}

/// Each matcher's relation over the documents' tapes: the chunk's matcher wave as one launch ([`MatcherWave`]), its
/// roots mapped from tape preorder (a core id) to the batch's node ids. A document with no tape (a text-only file), a
/// guest in another grammar or one a matcher's gates exclude has no rows of that matcher (`syntax::matcher_tapes`).
fn matched(
    batch: &Batch,
    tapes: &[WaveTape],
    matchers: &Wave,
    binding: crate::code_rule_dsl::typing::file_local::Binding,
    atoms: &crate::content_store::atoms::Atoms,
) -> StoreResult<Vec<(String, Arc<dyn Source>)>> {
    let _span = crate::code_rule_dsl::logic::profile::span(|| "ast matchers on the transducer".into());
    let Some(wave) = &matchers.wave else { return Ok(Vec::new()) };
    let metal = matches!(binding, crate::code_rule_dsl::typing::file_local::Binding::Metal);
    let at: Vec<Option<&Projected>> = tapes.iter().map(|(key, ..)| batch.docs.iter().find(|p| p.key == *key)).collect();
    let held: Vec<usize> = (0..tapes.len()).filter(|&i| at[i].is_some()).collect();
    let ran = Ran { tapes, at, held, atoms };
    let active = active(matchers, &ran);
    // Each read at its path, which a `filePath` test inside a matcher tests.
    let docs: Vec<crate::code_rule_dsl::tape::TapeDoc> = ran.held.iter().map(|&i| tapes[i].2.with_path(&tapes[i].3)).collect();
    let nodes: u64 = docs.iter().map(|d| d.transducer_len() as u64).sum();
    let (found, limited) = launch(wave, docs, active, metal, nodes, matchers.len())?;
    crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Node, nodes);
    matchers
        .list
        .iter()
        .zip(found)
        .enumerate()
        .map(|(c, (m, found))| -> StoreResult<(String, Arc<dyn Source>)> {
            // The tapes where this matcher's program outran the work limit: there it gave every candidate root.
            let over: Vec<usize> = limited.iter().filter(|&&(at, _)| at == c).map(|&(_, t)| t).collect();
            Ok((m.rel.clone(), Arc::new(relation(m, &found, &over, &ran)?) as Arc<dyn Source>))
        })
        .collect()
}

/// Per matcher, per held tape: whether the matcher runs there.
fn active(matchers: &Wave, ran: &Ran) -> Vec<Vec<bool>> {
    let errors: Vec<crate::code_rule_dsl::soundness::SyntaxErrors> = ran.held.iter().map(|&i| crate::code_rule_dsl::soundness::SyntaxErrors::of_doc(&ran.tapes[i].2)).collect();
    matchers.list.iter().map(|m| ran.held.iter().zip(&errors).map(|(&i, errors)| runs_on(m, &ran.tapes[i], errors)).collect()).collect()
}

/// Whether matcher `m` runs on `tape`: its gates hold and W admits the document, its `syntax` (valid or invalid) and
/// text literals.
fn runs_on(m: &Matcher, tape: &WaveTape, errors: &crate::code_rule_dsl::soundness::SyntaxErrors) -> bool {
    let source = tape.1.source.as_str();
    m.gates.iter().all(|g| g.holds(&tape.3, source)) && m.captor.grammar.admits_source(source, errors, &mut rustc_hash::FxHashMap::default())
}

type RootsAndLimits = (Vec<Vec<BTreeSet<u32>>>, Vec<(usize, usize)>);

/// The wave's roots per matcher, per tape of `docs`, and the (matcher, tape) pairs over the work limit; no roots
/// where no matcher runs.
fn launch(
    wave: &Arc<crate::code_rule_dsl::transducer::packed::LoweredWave>,
    docs: Vec<crate::code_rule_dsl::tape::TapeDoc>,
    active: Vec<Vec<bool>>,
    metal: bool,
    nodes: u64,
    matchers: usize,
) -> StoreResult<RootsAndLimits> {
    use crate::code_rule_dsl::logic::profile;
    if docs.is_empty() || active.iter().all(|a| !a.iter().any(|&on| on)) {
        return Ok((vec![vec![BTreeSet::new(); docs.len()]; matchers], Vec::new()));
    }
    let program = MatcherWave { wave: Arc::clone(wave), tapes: docs, active, gpu: cfg!(target_os = "macos") && metal };
    let (found, run, wall) = run_wave(program, metal)?;
    if run.metal {
        profile::device("ast matcher wave", run.command_buffers as u64, run.device, wall, nodes);
    }
    profile::decision("ast matcher wave lane", || (if run.metal { "metal" } else { "cpu" }).to_owned());
    if !run.limited.is_empty() {
        profile::decision("ast matcher roots over the work limit (confirmed by W)", || format!("{} matcher-files", run.limited.len()));
    }
    Ok((found, run.limited))
}

/// `program` launched on the executor (the GPU's lane where `metal`): its roots, how it ran, its wall seconds.
fn run_wave(program: MatcherWave, metal: bool) -> StoreResult<WaveOutput> {
    let kernel = rerail_accel::Kernel::Program(rerail_accel::kernel::ProgramKernel(Arc::new(program)));
    let ex = crate::code_rule_dsl::logic::lanes::executor_for(metal);
    let (mut done, _) = crate::code_rule_dsl::logic::lanes::prof::run_on(ex, "ast matcher wave", rerail_accel::Batch(vec![rerail_accel::Launch::new(kernel)])).map_err(StoreError::Invalid)?;
    let done = done.remove(0);
    let rerail_accel::kernel::Output::Program(r) = done.out else {
        return Err(StoreError::Invalid("ast matcher wave: not a program's result".into()));
    };
    let r = r.0.downcast::<WaveFound>().map_err(|_| StoreError::Invalid("ast matcher wave: a result of another type".into()))?;
    r.as_ref().clone().map_err(|e| StoreError::Invalid(format!("ast matcher wave: {e}")))
}

/// Matcher `m`'s relation from its roots `found` (per held tape): the roots as the batch's nodes, W confirming those
/// of the tapes `over` the work limit; with captures or relaxed, W's matcher's rows at each root.
fn relation(m: &Matcher, found: &[BTreeSet<u32>], over: &[usize], ran: &Ran) -> StoreResult<Relation> {
    let mut r = Relation::new(crate::code_rule_dsl::logic::plan::corpus::ast_columns(&m.rel));
    let rows = if m.captures.is_empty() && !m.captor.verify {
        confirmed_over(m, found, over, ran, &mut r)? + push_roots(&mut r, found, over, ran)
    } else {
        push_captured(m, found, ran, &mut r)?
    };
    crate::code_rule_dsl::prof::count("ast.matched", rows as usize);
    Ok(r)
}

/// The roots of the tapes `over` the work limit, each confirmed by W's matcher, as `m`'s rows: how many.
fn confirmed_over(m: &Matcher, found: &[BTreeSet<u32>], over: &[usize], ran: &Ran, r: &mut Relation) -> StoreResult<u64> {
    use crate::code_rule_dsl::logic::profile;
    if over.is_empty() {
        return Ok(0);
    }
    let _confirm = profile::span(|| "ast roots over the work limit, confirmed (W's matcher at each root)".into());
    let held_over: Vec<usize> = over.iter().map(|&t| ran.held[t]).collect();
    let found_over: Vec<BTreeSet<u32>> = over.iter().map(|&t| found[t].clone()).collect();
    let (found_rows, budget) = captured_rows(m, &held_over, &found_over, ran)?;
    if budget > 0 {
        profile::decision("ast captures over a file's visit budget (no rows)", || format!("{}: {budget} roots", m.rel));
    }
    Ok(push_rows(r, found_rows))
}

/// The roots of the tapes not `over` the work limit, as the batch's nodes: how many (a node the projection leaves
/// out has none).
fn push_roots(r: &mut Relation, found: &[BTreeSet<u32>], over: &[usize], ran: &Ran) -> u64 {
    let mut rows = 0u64;
    for (&i, found) in ran.held.iter().zip(found).enumerate().filter(|(t, _)| !over.contains(t)).map(|(_, x)| x) {
        let p = ran.at[i].expect("held");
        for global in found.iter().filter_map(|&core| global_of(p, core)) {
            r.push(&[global]);
            rows += 1;
        }
    }
    rows
}

/// Captures, or a relaxed matcher's roots to confirm (decision 0107): W's matcher's rows at each root, as `m`'s rows:
/// how many.
fn push_captured(m: &Matcher, found: &[BTreeSet<u32>], ran: &Ran, r: &mut Relation) -> StoreResult<u64> {
    use crate::code_rule_dsl::logic::profile;
    let _captures = profile::span(|| "ast captures and confirmations (W's matcher at each root)".into());
    let roots: usize = found.iter().map(BTreeSet::len).sum();
    let (found_rows, limited) = captured_rows(m, &ran.held, found, ran)?;
    if limited > 0 {
        profile::decision("ast captures over a file's visit budget (no rows)", || format!("{}: {limited} roots", m.rel));
    }
    // A relaxed matcher's cost (decision 0107 §3): the roots W confirmed on the host, and the rows it kept.
    if m.captor.verify {
        crate::code_rule_dsl::prof::count("ast.relaxed_roots", roots);
        crate::code_rule_dsl::prof::count("ast.relaxed_kept", found_rows.len());
        profile::decision("relaxed matchers: roots W confirmed, rows kept", || format!("{}: {roots} roots, {} kept", m.rel, found_rows.len()));
    }
    Ok(push_rows(r, found_rows))
}

/// `rows` pushed onto `r`: how many.
fn push_rows(r: &mut Relation, rows: Vec<Vec<i64>>) -> u64 {
    let mut pushed = 0u64;
    for row in rows {
        r.push(&row);
        pushed += 1;
    }
    pushed
}

/// The batch's node of core id `core` in `p`; `None` for one the projection leaves out.
fn global_of(p: &Projected, core: u32) -> Option<i64> {
    let local = p.local.get(core as usize).copied().unwrap_or(NONE);
    (local != NONE).then(|| p.global(local))
}

/// A captured matcher's rows (decision 0107): at each root the transducer matched, W's matcher of the atom's rule
/// matched again on the host, and each capture's nodes read from its bindings (a multi-capture's every node, so its
/// rows are the product of the captures' nodes). A node the projection leaves out has no row. Tapes in parallel; also
/// how many roots hit their file's visit budget (no rows).
fn captured_rows(m: &Matcher, held: &[usize], found: &[BTreeSet<u32>], ran: &Ran) -> StoreResult<(Vec<Vec<i64>>, usize)> {
    use rayon::prelude::*;
    let per: Vec<StoreResult<(Vec<Vec<i64>>, usize)>> = held
        .par_iter()
        .zip(found.par_iter())
        .map(crate::code_rule_dsl::logic::profile::carried("ast captures (pool)", |(&i, roots): (&usize, &BTreeSet<u32>)| tape_captures(m, i, roots, ran)))
        .collect();
    let per = per.into_iter().collect::<StoreResult<Vec<_>>>()?;
    let limited = per.iter().map(|(_, l)| l).sum();
    Ok((per.into_iter().flat_map(|(rows, _)| rows).collect(), limited))
}

/// `m`'s captured rows at `roots` of tape `i`, and how many roots hit the file's visit budget (no rows).
fn tape_captures(m: &Matcher, i: usize, roots: &BTreeSet<u32>, ran: &Ran) -> StoreResult<(Vec<Vec<i64>>, usize)> {
    let mut rows = Vec::new();
    let mut limited = 0;
    if roots.is_empty() {
        return Ok((rows, limited));
    }
    let p = ran.at[i].expect("held");
    let tape = &ran.tapes[i].2;
    // W's matcher reads the file's path (`filePath`, which a relaxed matcher leaves to it).
    let _file = crate::code_rule_dsl::matcher::FileScopeSlot::install(Vec::new(), None, !tape.syntax_error_ranges().is_empty()).path(&ran.tapes[i].3);
    let root = ast_grep_core::AstGrep::doc(tape.clone());
    let work = m.captor.limits.visits_for(tape.transducer_len());
    for &n in roots {
        let Some(n_global) = global_of(p, n) else { continue };
        match crate::code_rule_dsl::waves::findings::captured(&m.captor.grammar, &root, n, &m.captures, work) {
            Ok(Some(names)) => rows.extend(capture_product(n_global, names, p, ran.atoms)?),
            Ok(None) => {}
            Err(_) => limited += 1,
        }
    }
    Ok((rows, limited))
}

/// Root `n_global`'s rows: the product of its captures' nodes, a transform's text a symbol (the store's atom, as the
/// program's constants are).
fn capture_product(n_global: i64, names: Vec<crate::code_rule_dsl::waves::findings::Capture>, p: &Projected, atoms: &crate::content_store::atoms::Atoms) -> StoreResult<Vec<Vec<i64>>> {
    use crate::code_rule_dsl::waves::findings::Capture;
    let mut product: Vec<Vec<i64>> = vec![vec![n_global]];
    for capture in names {
        let nodes: Vec<i64> = match capture {
            Capture::Nodes(nodes) => nodes.into_iter().filter_map(|core| global_of(p, core)).collect(),
            Capture::Text(text) => vec![i64::from(atoms.intern(&text)?)],
        };
        product = product.into_iter().flat_map(|row| nodes.iter().map(move |&x| {
            let mut row = row.clone();
            row.push(x);
            row
        })).collect();
    }
    Ok(product)
}

