//! P1's rounds (decision 0501 §1): the device decides, the host loads.
//!
//! A unit starts as its predefines and its main file. Each round is one evaluation on the device
//! (`eval`), which grows the include tree, lays it out and decides its conditionals. An include
//! whose file the unit's file table holds no row for is a request. Between rounds the host
//! answers each request and does nothing else: it searches the configuration's paths, reads the
//! file it finds, places it and its tape in the residency, and adds the table's row (a row that
//! found nothing too). The round with no requests is the unit's answer.

use rerail_c_pp::Config;
use rerail_c_pp::files::Files;
use rerail_metal::pp_eval::{Unit, name_key, placement};
use rerail_metal::pp_expand::{Expanded, Layout, NO_PARENT, NOT_FOUND, Part, PpNode, Request, Sizes, relex_options};
use rerail_metal::slabs;

/// A unit of `files` (each a source and whether it is C++), as the evaluator reads it: each file's
/// format 16 tape and its source placed in the slab residency, where the device reads them
/// (decision 0468 §1). A document already resident (by its digest) is not placed again.
pub fn place(files: &[(&[u8], bool)]) -> Result<Unit, rerail_metal::Error> {
    use sha2::{Digest as _, Sha256};
    let tapes: Vec<_> = files.iter().map(|&(s, cpp)| rerail_c_pp::tape::image_16(s, crate::pp::options(cpp))).collect();
    let digests: Vec<[u8; 32]> = files.iter().zip(&tapes).map(|(&(s, _), t)| Sha256::new().chain_update(t.as_bytes()).chain_update(s).finalize().into()).collect();
    let misses = slabs::misses(&digests)?;
    let mut parts = Vec::new();
    for (k, d) in digests.iter().enumerate() {
        if !misses.contains(d) || parts.iter().any(|p: &slabs::Placement<'_>| p.0 == *d) {
            continue;
        }
        let (tape, src) = (tapes[k].as_bytes(), files[k].0);
        parts.push(placement(tape.len(), src.len(), *d, rerail_c_pp::tape::lang(), move |t: &mut [u8], s: &mut [u8]| {
            // The loader's write of a load (0353 step 4a): the room is the document's one place.
            t.copy_from_slice(tape);
            s.copy_from_slice(src);
        }));
    }
    slabs::insert(&digests, parts)?;
    Unit::of(slabs::resident(&digests)?)
}

/// A string literal of `text`: its `"` and `\` escaped (the serial pass's `string_tok`).
fn literal(text: &str) -> Vec<u8> {
    let mut q = vec![b'"'];
    for &c in text.as_bytes() {
        if c == b'"' || c == b'\\' {
            q.push(b'\\');
        }
        q.push(c);
    }
    q.push(b'"');
    q
}

/// The files a unit's rounds loaded, by the unit's file index: 0 the predefines, 1 the main
/// file, then each as a round's answer found it. Each one's source, path, and the search entry it
/// was found by (`NOT_FOUND`: its includer's directory, an absolute name, a root).
#[derive(Default)]
pub struct Loaded {
    pub files: Vec<Vec<u8>>,
    pub paths: Vec<String>,
    pub found_by: Vec<u32>,
}

/// A unit's rounds: the last round's unit, layout and answer, the files loaded, and how many
/// rounds it took.
pub struct Rounds<T> {
    pub unit: Unit,
    pub lay: Layout,
    pub loaded: Loaded,
    pub rounds: u32,
    pub answer: T,
}

/// What a round on the device gives the host: its requests, P1's refusals (`P1_*`), and its
/// answer (meaningful where there are neither).
pub type Round<T> = (Vec<Request>, u32, T);

/// Why a unit's rounds give no answer.
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    /// The main file is not read.
    NoMain,
    /// P1 refused it (`P1_*`).
    P1(u32),
    /// Past the rounds' bound (the configuration's depth, and two).
    Rounds,
    /// The device's error.
    Device(String),
}

/// The `#if` operators of `cfg`'s dialect, by their names' keys (`OPK_*`): the `__has_*` tests
/// (the built-ins'), and in C++ the alternative tokens (`and` is `&&`), as `Pp::if_value` reads
/// them.
fn if_ops(cfg: &Config) -> Vec<(u64, u32)> {
    use rerail_c_lean::T;
    use rerail_c_pp::macros::{BUILTINS, Builtin};
    use rerail_metal::pp_expand::*;
    let mut ops: Vec<(u64, u32)> = BUILTINS
        .iter()
        .filter_map(|&(n, b, _)| {
            let k = match b {
                Builtin::HasInclude { next: false } => OPK_HAS_INCLUDE,
                Builtin::HasInclude { next: true } => OPK_HAS_INCLUDE_NEXT,
                Builtin::HasEmbed => OPK_HAS_EMBED,
                Builtin::Feature if n == "__is_identifier" => OPK_IS_IDENTIFIER,
                Builtin::Feature => OPK_FEATURE,
                _ => return None,
            };
            Some((name_key(n.as_bytes()), k))
        })
        .collect();
    if cfg.opts.cpp {
        for (n, t) in rerail_c_pp::stream::ALT {
            let eo = match t {
                T::AmpAmp => EO_ANDAND,
                T::PipePipe => EO_OROR,
                T::Bang => EO_BANG,
                T::Amp => EO_AND,
                T::Pipe => EO_OR,
                T::Caret => EO_XOR,
                T::Tilde => EO_TILDE,
                T::Ne => EO_NE,
                _ => EO_OTHER,
            };
            ops.push((name_key(n.as_bytes()), OPK_ALT + eo));
        }
    }
    ops
}

/// What a unit's rounds are under one configuration: the `#if` operators, the features, the
/// built-ins' names and the lexer's options.
struct Dialect<'c> {
    cfg: &'c Config,
    ops: Vec<(u64, u32)>,
    features: Vec<(u64, u64)>,
    builtins: Vec<u64>,
    relex: u32,
}

impl<'c> Dialect<'c> {
    fn of(cfg: &'c Config, cpp: bool) -> Dialect<'c> {
        let o = crate::pp::options(cpp);
        let builtins = rerail_c_pp::stream::parallel::predefined(cfg).1.iter().map(|n| name_key(n)).collect();
        let features = cfg.features.iter().map(|((t, op), &v)| (rerail_metal::pp_expand::feature_key(t, op), v as u64)).collect();
        Dialect { cfg, ops: if_ops(cfg), features, builtins, relex: relex_options(o.cpp, o.colons, o.digit_separators, o.raw_strings) }
    }
}

/// One unit's rounds so far: its files loaded, its file table's rows, and the rounds that made
/// requests.
struct Progress {
    loaded: Loaded,
    table: Vec<(u32, u64, u32)>,
    rounds: u32,
}

impl Progress {
    fn start(cfg: &Config, io: &mut dyn Files, main: &str) -> Result<Progress, Refused> {
        let src = io.read(main).ok_or(Refused::NoMain)?;
        let pre = rerail_c_pp::stream::parallel::predefined(cfg).0;
        Ok(Progress { loaded: Loaded { files: vec![pre, src], paths: vec!["<built-in>".into(), main.to_string()], found_by: vec![NOT_FOUND, NOT_FOUND] }, table: Vec::new(), rounds: 0 })
    }

    /// The unit as the next round reads it: its files placed, and its layout.
    fn unit(&self, d: &Dialect<'_>, cpp: bool) -> Result<(Unit, Layout), rerail_metal::Error> {
        let (cfg, loaded) = (d.cfg, &self.loaded);
        let unit = place(&loaded.files.iter().map(|s| (s.as_slice(), cpp)).collect::<Vec<_>>())?;
        let lay = Layout {
            nodes: vec![PpNode { file: 0, parent: NO_PARENT, pdir: 0, depth: 0, unit: 0 }, PpNode { file: 1, parent: NO_PARENT, pdir: 1, depth: 0, unit: 0 }],
            table: self.table.clone(),
            found_by: loaded.found_by.clone(),
            max_depth: cfg.max_depth,
            if_ops: d.ops.clone(),
            features: d.features.clone(),
            true_is_one: cfg.true_is_one,
            builtins: d.builtins.clone(),
            relex_options: d.relex,
            file_literals: loaded.paths.iter().chain([&cfg.date, &cfg.time, &cfg.timestamp]).map(|p| literal(p)).collect(),
            name_literals: loaded.paths.iter().map(|p| literal(p.rsplit('/').next().unwrap_or(""))).collect(),
        };
        Ok((unit, lay))
    }

    /// The host's part of a round: each request answered, by the search and a read, as a row of
    /// the file table (a row that found nothing too).
    fn answer(&mut self, io: &mut dyn Files, requests: impl IntoIterator<Item = Request>) {
        let loaded = &mut self.loaded;
        for q in requests {
            let includer = loaded.paths[q.file as usize].clone();
            let next_after = q.next.then(|| Some(loaded.found_by[q.file as usize]).filter(|&k| k != NOT_FOUND));
            let file = match io.find(&q.name, q.angle, &includer, next_after) {
                Some((path, found_by)) => match loaded.paths.iter().position(|p| *p == path) {
                    Some(k) => k as u32,
                    None => match io.read(&path) {
                        Some(bytes) => {
                            loaded.files.push(bytes);
                            loaded.paths.push(path);
                            loaded.found_by.push(found_by.unwrap_or(NOT_FOUND));
                            loaded.files.len() as u32 - 1
                        }
                        None => NOT_FOUND,
                    },
                },
                None => NOT_FOUND,
            };
            self.table.push((0, q.key, file));
        }
        self.rounds += 1;
    }
}

/// The unit whose main file is `main`, in rounds of `eval` under `cfg` and `io` (C++ where
/// `cpp`).
pub fn run<T>(cfg: &Config, io: &mut dyn Files, main: &str, cpp: bool, mut eval: impl FnMut(&Unit, &Layout) -> Result<Round<T>, rerail_metal::Error>) -> Result<Rounds<T>, Refused> {
    let d = Dialect::of(cfg, cpp);
    let mut p = Progress::start(cfg, io, main)?;
    while p.rounds < cfg.max_depth + 2 {
        let (unit, lay) = p.unit(&d, cpp).map_err(|e| Refused::Device(format!("{e:?}")))?;
        let (requests, flags, answer) = eval(&unit, &lay).map_err(|e| Refused::Device(format!("{e:?}")))?;
        if flags != 0 {
            return Err(Refused::P1(flags));
        }
        if requests.is_empty() {
            return Ok(Rounds { unit, lay, loaded: p.loaded, rounds: p.rounds + 1, answer });
        }
        p.answer(io, requests);
    }
    Err(Refused::Rounds)
}

/// A unit its rounds settled: its last round's layout, its files, its rounds, and its expansion
/// (the last round's run: decision 0503 §7). It holds none of the residency: its documents may
/// be retired once the batch's run is done.
pub struct Settled {
    pub lay: Layout,
    pub loaded: Loaded,
    pub rounds: u32,
    pub part: Part,
}

/// The units `units` (each its files and its main file), of one configuration `cfg` (C++ where
/// `cpp`), in rounds run as batches (decision 0503 §7): each round one run over the batch's
/// units, the units that request answered between rounds, and the round with no requests every
/// unit's expansion. A batch is split in halves where its documents are in more than one slab,
/// where the budget refuses its arena, or where the device refuses it or P1 does: a refusal is
/// then a unit's alone. `eval` is the device's run of a batch from the rooms it is given, which
/// it leaves at the rooms it settled at.
pub fn run_many(cfg: &Config, cpp: bool, units: &mut [(Box<dyn Files + '_>, String)], mut eval: impl FnMut(&Unit, &Layout, &mut Sizes) -> Result<Expanded, rerail_metal::Error>) -> Vec<Result<Settled, Refused>> {
    let d = Dialect::of(cfg, cpp);
    let mut out: Vec<Option<Result<Settled, Refused>>> = (0..units.len()).map(|_| None).collect();
    let mut live = Vec::new();
    for (k, (files, main)) in units.iter_mut().enumerate() {
        match Progress::start(cfg, files.as_mut(), main) {
            Ok(p) => live.push((k, p)),
            Err(e) => out[k] = Some(Err(e)),
        }
    }
    batch_rounds(&d, cpp, units, live, &mut out, &mut eval);
    out.into_iter().map(|r| r.expect("every unit answered or refused")).collect()
}

/// What `run_many` gives back: each unit settled or refused, by its index.
type Answers = [Option<Result<Settled, Refused>>];

/// A batch's run from the rooms it is given (`run_many`'s `eval`).
type Eval<'e> = dyn FnMut(&Unit, &Layout, &mut Sizes) -> Result<Expanded, rerail_metal::Error> + 'e;

/// `run_many`'s rounds for the units `live` (each its index in `units` and its rounds so far).
fn batch_rounds(d: &Dialect<'_>, cpp: bool, units: &mut [(Box<dyn Files + '_>, String)], mut live: Vec<(usize, Progress)>, out: &mut Answers, eval: &mut Eval<'_>) {
    let mut sizes: Option<Sizes> = None;
    loop {
        live.retain_mut(|(k, p)| {
            let past = p.rounds >= d.cfg.max_depth + 2;
            if past {
                out[*k] = Some(Err(Refused::Rounds));
            }
            !past
        });
        if live.is_empty() {
            return;
        }
        // The round's units are placed only while it runs: the halves of a split run with none
        // of the batch's documents held.
        match batch_round(d, cpp, units, &mut live, out, eval, &mut sizes) {
            Step::Again => {}
            Step::Done => return,
            Step::Split => return split(d, cpp, units, live, out, eval),
        }
    }
}

/// What a batch's round leaves: another round, every unit answered, or the batch to split.
enum Step {
    Again,
    Done,
    Split,
}

/// One round of the batch `live`, from the rooms `sizes` (the batch's last round's).
fn batch_round(d: &Dialect<'_>, cpp: bool, units: &mut [(Box<dyn Files + '_>, String)], live: &mut Vec<(usize, Progress)>, out: &mut Answers, eval: &mut Eval<'_>, sizes: &mut Option<Sizes>) -> Step {
    use rerail_metal::Error;
    let many = live.len() > 1;
    let placed: Result<Vec<(Unit, Layout)>, Error> = live.iter().map(|(_, p)| p.unit(d, cpp)).collect();
    let placed = match placed {
        Ok(placed) => placed,
        Err(_) if many => return Step::Split,
        Err(e) => {
            out[live[0].0] = Some(Err(Refused::Device(format!("{e:?}"))));
            return Step::Done;
        }
    };
    let joined = if many {
        let batch = Unit::batch(&placed.iter().map(|(u, _)| u).collect::<Vec<_>>());
        let lay = Layout::join(&placed.iter().map(|(u, l)| (l, u.len())).collect::<Vec<_>>());
        match batch.zip(lay) {
            Some(j) => Some(j),
            None => return Step::Split,
        }
    } else {
        None
    };
    let (unit, lay) = joined.as_ref().map_or((&placed[0].0, &placed[0].1), |(u, l)| (u, l));
    let mut s = sizes.map_or_else(|| Sizes::first(unit, lay), |s| s.fit(unit, lay));
    let x = eval(unit, lay, &mut s);
    *sizes = Some(s);
    let x = match x {
        Ok(x) if many && (x.refused != 0 || x.p1_flags != 0) => return Step::Split,
        Err(Error::BackendCapacity) if many => return Step::Split,
        Err(e) => {
            for (k, _) in live.drain(..) {
                out[k] = Some(Err(Refused::Device(format!("{e:?}"))));
            }
            return Step::Done;
        }
        Ok(x) if x.p1_flags != 0 => {
            out[live[0].0] = Some(Err(Refused::P1(x.p1_flags)));
            return Step::Done;
        }
        Ok(x) => x,
    };
    if !x.requests.is_empty() {
        // The host's part: each unit's requests answered. A unit with none runs again with the
        // batch, as it was.
        let mut requests = x.requests;
        for (u, (k, p)) in live.iter_mut().enumerate() {
            let mine: Vec<Request> = requests.extract_if(.., |q| q.unit == u as u32).collect();
            if !mine.is_empty() {
                p.answer(units[*k].0.as_mut(), mine);
            }
        }
        return Step::Again;
    }
    let x = std::rc::Rc::new(x);
    let mut file = 0;
    for (u, ((k, p), (unit, lay))) in live.drain(..).zip(placed).enumerate() {
        let out_range = if many { x.unit_out[u] as usize..x.unit_out[u + 1] as usize } else { 0..x.out.len() };
        let part = Part { x: std::rc::Rc::clone(&x), files: file..file + unit.len(), out: out_range };
        file += unit.len();
        out[k] = Some(Ok(Settled { lay, loaded: p.loaded, rounds: p.rounds + 1, part }));
    }
    Step::Done
}

/// `batch_rounds` of each half of `live`, from where its rounds are.
fn split(d: &Dialect<'_>, cpp: bool, units: &mut [(Box<dyn Files + '_>, String)], mut live: Vec<(usize, Progress)>, out: &mut Answers, eval: &mut Eval<'_>) {
    let b = live.split_off(live.len() / 2);
    batch_rounds(d, cpp, units, live, out, eval);
    batch_rounds(d, cpp, units, b, out, eval);
}
