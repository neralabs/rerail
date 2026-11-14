//! The transducer's device, behind an interface: matcher programs, label passes and root walks run on it when one
//! is registered, and on the CPU otherwise. The engine registers its devices as it loads (`crate::install_devices`);
//! on macOS that is Metal (`rerail-device-metal`, `code_rule_dsl::device_metal`). Nothing here names a device.
//!
//! The backends stay data (`ExecutionBackend`, `Backend::MetalInterpreter`, `RanOn`): a backend with no device to
//! run it is refused (`Error::UnsupportedBackend`), or falls back to the CPU where it did before the split.
use std::collections::BTreeSet;
use std::sync::{Arc, OnceLock};

use super::{BatchOutcome, Error, ExecutionBackend, LoweredMatcher, LoweredWave, PackedExecution, Program};
use crate::code_rule_dsl::tape::{TapeDoc, TapeSource};

/// Which of the device's program engines runs a batch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Engine {
    /// One interpreter kernel for every program.
    Interpreter,
    /// The batch's programs compiled (cached by their words).
    Jit,
}

/// What the transducer runs on a device.
pub trait MatcherDevice: Send + Sync {
    /// The batch's programs on `engine`, from `anchors` (per file, per program), at most `result_limits` results each.
    fn execute_packed(&self, engine: Engine, programs: &[Program], files: &[TapeDoc], anchors: &[Vec<Vec<u32>>], result_limits: Option<&[u32]>) -> Result<PackedExecution, Error>;
    /// The batch's outcomes on `backend` (one of the device's: `MetalInterpreter`, `MetalJit` or `Metal`).
    fn execute_outcomes(&self, backend: ExecutionBackend, programs: &[Program], files: &[TapeDoc], anchors: &[Vec<Vec<u32>>]) -> Result<BatchOutcome, Error>;
    /// The production programs compiled, so the first batch need not wait for them.
    fn warm(&self, programs: &[Program]) -> Result<(), Error>;
    /// Whether the batch's compiled programs are ready.
    fn jit_ready(&self, programs: &[Program]) -> Result<bool, Error>;
    /// The interpreter built (once per process).
    fn compile_interpreter(&self) -> Result<(), Error>;
    /// The sweep and the batch's programs compiled: the background compilation of `MetalBackground`.
    fn compile_background(&self, programs: &[Program]) -> Result<(), Error>;
    /// The production corpus in one grid, sharing its files' anchors.
    fn execute_shared_corpus(&self, programs: &[Program], files: &[TapeDoc], anchors: &[Vec<Arc<[u32]>>], result_limits: &[u32]) -> Result<PackedExecution, Error>;
    /// [`execute_shared_corpus`](Self::execute_shared_corpus) by pages of restored tapes, each page to `consumer`.
    fn execute_shared_corpus_pages(&self, programs: &[Program], files: &[TapeSource], anchors: &[Vec<Arc<[u32]>>], result_limits: &[u32], consumer: &mut PageConsumer<'_>) -> Result<(), Error>;
    /// A matcher's roots over the batch, every pass in one command buffer, pattern leaves included.
    fn matcher_roots(&self, matcher: &LoweredMatcher, tapes: &[TapeDoc]) -> Result<(Vec<BTreeSet<u32>>, Outcome), Error>;
    /// Each clause's matches of the batch from `roots` (per clause, per tape), every pass and root in one command
    /// buffer.
    fn wave_roots(&self, wave: &LoweredWave, tapes: &[TapeDoc], roots: &[Vec<Vec<u32>>]) -> Result<(Vec<Vec<BTreeSet<u32>>>, Outcome), Error>;
    /// Every clause's labels over the batch, as views of its tapes the root programs read, with `roots`' records.
    fn wave_labelled(&self, wave: &LoweredWave, tapes: &[TapeDoc], active: &[Vec<bool>], roots: Option<&RootJobs<'_>>) -> Result<(Vec<TapeDoc>, Outcome), Error>;
    /// Starts compiling the label passes' pipelines.
    fn warm_labels(&self);
    /// Drops the label passes' pipelines (their memory) once the documents they ran over are gone.
    fn drop_label_pipelines(&self);
    /// Span prefilters' ends (`regex_block`): bit `i` of a file's words, a match can end after its byte `i`. None:
    /// the device could not run them (the cores do).
    fn span_ends(&self, natives: &[&rerail_compiler::block_native::BlockNative], batch: &[super::block::BatchFile<'_>]) -> Option<Vec<Vec<u64>>>;
    /// This thread's program invocations and host events since the last reset (tests count them).
    fn execution_metrics(&self) -> (usize, usize);
    /// Resets this thread's `execution_metrics`.
    fn reset_execution_metrics(&self);
}

static MATCHER: OnceLock<&'static dyn MatcherDevice> = OnceLock::new();

/// Registers the transducer's device; the first registration holds.
pub fn register(device: &'static dyn MatcherDevice) {
    let _ = MATCHER.set(device);
}

/// The registered device, if any.
pub fn matcher() -> Option<&'static dyn MatcherDevice> {
    MATCHER.get().copied()
}

/// The registered device, or `UnsupportedBackend`.
pub(in crate::code_rule_dsl) fn required() -> Result<&'static dyn MatcherDevice, Error> {
    matcher().ok_or(Error::UnsupportedBackend)
}

/// Root programs and, per program, per file, their starts (an anchor a task).
pub struct RootJobs<'a> {
    pub programs: Vec<&'a Program>,
    pub starts: Vec<Vec<Vec<u32>>>,
}

/// Per root program: per file its (task, record)s in task then walk order, or `None` where the
/// program did not run on the device (not a micro walk of at most 32 frames, a walk that
/// overflowed, or records past the buffer); the host walks those.
pub type RootsFound = Vec<Option<Vec<Vec<(u32, crate::code_rule_dsl::transducer::packed::RootRecord)>>>>;

#[derive(Debug, Default)]
pub struct Outcome {
    /// Every label's words after the passes.
    pub labels: Vec<u32>,
    pub words: usize,
    pub command_buffers: usize,
    pub dispatches: usize,
    /// Why the host waited before the end: workspace growth, a recursive group that needed more
    /// rounds than were encoded (the whole clause again), the upload budget.
    pub waits: [usize; 3],
    /// Rounds encoded per recursive group.
    pub rounds: usize,
    /// Whether a task outgrew `NARROW`'s states, so the batch ran again with `WIDE`'s.
    pub widened: bool,
    /// Whether a micro walk outgrew its frames, so every program ran on the interpreter.
    pub interpreted: bool,
    /// The plan's root records (`Plan::roots`); empty without roots.
    pub records: RootsFound,
    /// The command buffers' seconds on the GPU (`GPUStartTime` to `GPUEndTime`), summed.
    pub device: f64,
}

pub type PageConsumer<'a> = dyn FnMut(PackedExecution, usize) -> Result<(), Error> + 'a;
