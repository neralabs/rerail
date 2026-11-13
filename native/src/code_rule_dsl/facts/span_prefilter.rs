//! A span pattern's prefilter as the facts read it: where a match can end over a source, marked by a program the
//! transducer compiles (its block programs, on the cores or the GPU). The facts define what they ask of it; the
//! transducer registers what compiles and launches one at load (`install_span_prefilter`). Without one, no pattern
//! is prefiltered and every span is the regex's.

use std::any::Any;
use std::sync::{Arc, OnceLock};

/// Where a batch of span programs ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RanOn {
    Metal,
    Cpu,
}

/// A compiled pattern's prefilter.
pub trait SpanEnds: Send + Sync + 'static {
    /// Bit `i` of the words: a match can end after byte `i` of `bytes`.
    fn ends(&self, bytes: &[u8]) -> Vec<u64>;
    /// The prefilter as its own type (the launcher reads it).
    fn into_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync>;
}

/// Compiles a pattern over sources (`lines` false) or over each line (`lines` true); None: the host's.
pub type Compile = fn(&str, bool) -> Option<Arc<dyn SpanEnds>>;
/// Ends over each of `sources` (`files`: (source, the pattern's index in the prefilters)), placed by price on the
/// cores or, when `device` and the executor has one, the GPU.
pub type Launch = fn(&rerail_accel::Executor, &[Arc<dyn SpanEnds>], &[Arc<String>], &[(usize, usize)], bool) -> Result<(Vec<Vec<u64>>, RanOn), String>;

static SPAN_PREFILTER: OnceLock<(Compile, Launch)> = OnceLock::new();

/// Registers what compiles and launches span prefilters (once, at load).
pub fn install_span_prefilter(compile: Compile, launch: Launch) {
    let _ = SPAN_PREFILTER.set((compile, launch));
}

/// `re`'s prefilter, when one is registered and the pattern compiles.
pub fn compile(re: &str, lines: bool) -> Option<Arc<dyn SpanEnds>> {
    SPAN_PREFILTER.get().and_then(|(c, _)| c(re, lines))
}

/// The prefilters' ends over the sources (see `Launch`); none registered, none compiled, so none to launch.
pub fn launch(ex: &rerail_accel::Executor, filters: &[Arc<dyn SpanEnds>], sources: &[Arc<String>], files: &[(usize, usize)], device: bool) -> Result<(Vec<Vec<u64>>, RanOn), String> {
    match SPAN_PREFILTER.get() {
        Some((_, l)) => l(ex, filters, sources, files, device),
        None if files.is_empty() => Ok((Vec::new(), RanOn::Cpu)),
        None => Err("span prefilter: none registered".into()),
    }
}

/// Whether a match can end in `(a, b]`, a span's exclusive end, given `ends`.
pub fn any_in(ends: &[u64], a: usize, b: usize) -> bool {
    // Bits a..b (a match ending at e is bit e - 1).
    let (mut i, end) = (a, b);
    while i < end {
        let (w, bit) = (i / 64, i % 64);
        let take = (64 - bit).min(end - i);
        let mask = if take == 64 { u64::MAX } else { ((1u64 << take) - 1) << bit };
        if ends.get(w).is_some_and(|x| x & mask != 0) {
            return true;
        }
        i += take;
    }
    false
}
