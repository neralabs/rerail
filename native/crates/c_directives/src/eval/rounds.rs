//! P1's rounds (decision 0467 §1): the device decides, the host loads.
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
use rerail_metal::pp_expand::{Layout, NO_PARENT, NOT_FOUND, PpNode, Request, relex_options};
use rerail_metal::slabs;

/// A unit of `files` (each a source and whether it is C++), as the evaluator reads it: each file's
/// format 16 tape and its source placed in the slab residency, where the device reads them
/// (decision 0460 §1). A document already resident (by its digest) is not placed again.
pub fn place(files: &[(&[u8], bool)]) -> Unit {
    use sha2::{Digest as _, Sha256};
    let tapes: Vec<_> = files.iter().map(|&(s, cpp)| rerail_c_pp::tape::image_16(s, crate::pp::options(cpp))).collect();
    let digests: Vec<[u8; 32]> = files.iter().zip(&tapes).map(|(&(s, _), t)| Sha256::new().chain_update(t.as_bytes()).chain_update(s).finalize().into()).collect();
    let misses = slabs::misses(&digests).unwrap();
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
    slabs::insert(&digests, parts).unwrap();
    Unit::of(slabs::resident(&digests).unwrap()).unwrap()
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

/// The unit whose main file is `main`, in rounds of `eval` under `cfg` and `io` (C++ where
/// `cpp`). `if_values`: each `#if`'s and `#elif`'s value by its slot (the test input of 0448,
/// until the device evaluates them, 0467 step 5).
pub fn run<T>(cfg: &Config, io: &mut dyn Files, main: &str, cpp: bool, if_values: &[(u32, u32)], mut eval: impl FnMut(&Unit, &Layout) -> Result<Round<T>, rerail_metal::Error>) -> Result<Rounds<T>, Refused> {
    let src = io.read(main).ok_or(Refused::NoMain)?;
    let (pre, builtins) = rerail_c_pp::stream::parallel::predefined(cfg);
    let mut loaded = Loaded { files: vec![pre, src], paths: vec!["<built-in>".into(), main.to_string()], found_by: vec![NOT_FOUND, NOT_FOUND] };
    let mut table: Vec<(u32, u64, u32)> = Vec::new();
    let o = crate::pp::options(cpp);
    for round in 1..=cfg.max_depth + 2 {
        let unit = place(&loaded.files.iter().map(|s| (s.as_slice(), cpp)).collect::<Vec<_>>());
        let lay = Layout {
            nodes: vec![PpNode { file: 0, parent: NO_PARENT, pdir: 0, depth: 0, unit: 0 }, PpNode { file: 1, parent: NO_PARENT, pdir: 1, depth: 0, unit: 0 }],
            table: table.clone(),
            found_by: loaded.found_by.clone(),
            max_depth: cfg.max_depth,
            if_values: if_values.iter().map(|&(s, v)| (0, s, v)).collect(),
            builtins: builtins.iter().map(|n| name_key(n)).collect(),
            relex_options: relex_options(o.cpp, o.colons, o.digit_separators, o.raw_strings),
            file_literals: loaded.paths.iter().chain([&cfg.date, &cfg.time, &cfg.timestamp]).map(|p| literal(p)).collect(),
        };
        let (requests, flags, answer) = eval(&unit, &lay).map_err(|e| Refused::Device(format!("{e:?}")))?;
        if flags != 0 {
            return Err(Refused::P1(flags));
        }
        if requests.is_empty() {
            return Ok(Rounds { unit, lay, loaded, rounds: round, answer });
        }
        // The host's part: each request answered, by the search and a read.
        for q in requests {
            let (s, e) = (q.start as usize, (q.start + q.len) as usize);
            let name = loaded.files[q.file as usize][s..e].to_vec();
            let includer = loaded.paths[q.file as usize].clone();
            let next_after = q.next.then(|| Some(loaded.found_by[q.file as usize]).filter(|&k| k != NOT_FOUND));
            let file = match io.find(&name, q.angle, &includer, next_after) {
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
            table.push((0, q.key, file));
        }
    }
    Err(Refused::Rounds)
}
