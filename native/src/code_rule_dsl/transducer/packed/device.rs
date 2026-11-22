//! The transducer's device, behind an interface: matcher programs, label passes and root walks run on it when one
//! is registered, and on the CPU otherwise. The engine registers its devices as it loads (`crate::install_devices`);
//! on macOS that is Metal (`rerail-device-metal`, `code_rule_dsl::device_metal`). Nothing here names a device.
//!
//! The backends stay data (`ExecutionBackend`, `Backend::MetalInterpreter`, `RanOn`): a backend with no device to
//! run it is refused (`Error::UnsupportedBackend`, decision 0382). The product's own choice of backend, where no device
//! is registered, is the CPU's (`execute_production_corpus`): no device is not an error.
use std::collections::BTreeSet;
use std::sync::OnceLock;

use super::{BatchOutcome, Error, ExecutionBackend, PackedExecution, Program};
use crate::code_rule_dsl::tape::TapeDoc;

/// Which of the device's program engines runs a batch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Engine {
    /// One interpreter kernel for every program.
    Interpreter,
    /// The batch's programs compiled (cached by their words).
    Jit,
}

/// Where a wave's clause root programs start on the device.
#[derive(Clone, Copy)]
pub enum ClauseRoots<'r> {
    /// Per clause, per tape, the listed nodes (document-local, ascending).
    Listed(&'r [Vec<Vec<u32>>]),
    /// Per clause, per tape, whether the clause runs there: its roots are then its domain's nodes, a rule the device
    /// reads at each node's code (decision 0293 §10: the host lists none).
    Rule(&'r [Vec<bool>]),
}

/// A kernel the device compiles ahead of its first use: its name, and the call that asks for it as its use does
/// (`rerail_gpu::kernels::Ahead`, which the engine makes of it).
pub type KernelAhead = (String, Box<dyn Fn() -> Result<(), String> + Send + Sync>);

/// A step over slots: a slot is the space's nodes, a bit each.
#[derive(Clone, Copy, Debug)]
pub enum SetStep {
    Clear { out: u32 },
    /// Every node of the space.
    Fill { out: u32 },
    Copy { out: u32, a: u32 },
    Or { out: u32, a: u32, b: u32 },
    And { out: u32, a: u32, b: u32 },
    AndNot { out: u32, a: u32, b: u32 },
    /// `out`'s nodes kept in files where `test` has a node.
    Within { out: u32, test: u32 },
}

/// What the transducer runs on a device. What it runs of a matcher or a wave (W's, lowered) is the wave device's
/// (`compile_matcher::WaveDevice`, rerail-transducer's), which the device registers beside this.
pub trait MatcherDevice: Send + Sync {
    /// The device's fixed kernels (the same for every rule set), to compile ahead of their first use: the engine's
    /// `kernels compile`, and its warm-up as a run starts.
    fn kernels_ahead(&self) -> Vec<KernelAhead> {
        Vec::new()
    }
    /// The batch's programs on `engine`, from `anchors` (per file, per program), at most `result_limits` results each.
    /// The batch with each task held to `budget` (decision 0334).
    fn execute_packed(&self, engine: Engine, programs: &[Program], files: &[TapeDoc], anchors: &[Vec<Vec<u32>>], result_limits: Option<&[u32]>, budget: super::Budget) -> Result<PackedExecution, Error>;
    /// The batch's outcomes on `backend` (one of the device's: `MetalInterpreter` or `MetalJit`).
    fn execute_outcomes(&self, backend: ExecutionBackend, programs: &[Program], files: &[TapeDoc], anchors: &[Vec<Vec<u32>>]) -> Result<BatchOutcome, Error>;
    /// The production programs compiled, so the first batch need not wait for them.
    fn warm(&self, programs: &[Program]) -> Result<(), Error>;
    /// Whether the batch's compiled programs are ready.
    fn jit_ready(&self, programs: &[Program]) -> Result<bool, Error>;
    /// The interpreter built (once per process).
    fn compile_interpreter(&self) -> Result<(), Error>;
    /// The sweep and the batch's programs compiled: the background compilation of `MetalBackground`.
    fn compile_background(&self, programs: &[Program]) -> Result<(), Error>;
    /// Which of the kernels' tape views a batch of `format` reads (0 or 1): the sources differ by it alone.
    fn pattern_variant(&self, format: u32) -> usize;
    /// Whether the kernels of `sources` are at hand without a compile (decision 0395); each missing one is
    /// recorded, and compiled as the mode says.
    fn patterns_ready(&self, sources: &[String], format: u32) -> bool;
    /// Starts compiling the label passes' pipelines.
    fn warm_labels(&self);
    /// Drops the label passes' pipelines (their memory) once the documents they ran over are gone.
    fn drop_label_pipelines(&self);
    /// Span prefilters' ends (`regex_block`): bit `i` of a file's words, a match can end after its byte `i`.
    /// `Ok(None)`: a program form the device lacks, recorded in 0202's `metal-form` list (the cores run them). Any
    /// other failure is the run's error (decision 0382: no fallback on an error).
    fn span_ends(&self, natives: &[&rerail_compiler::block_native::BlockNative], batch: &[super::block::BatchFile<'_>]) -> Result<Option<Vec<Vec<u64>>>, Error>;
    /// A node-set program (the prefilter's sets, `SetStep`s over `slots` slots of a space of `len` nodes in `files`,
    /// each file its first node and count), with `leaves` written in their slots: the words of each of `outputs`.
    fn set_steps(&self, files: &[(u32, u32)], len: u32, slots: usize, leaves: &[(usize, &[u64])], steps: &[SetStep], outputs: &[usize]) -> Result<Vec<Vec<u64>>, Error>;
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
    /// How many times the run went again with twice the step budget after a task ran over it (decision 0334).
    pub replanned: usize,
    /// The (label, file) whose pass went to W's matcher (decision 0334): over its budget where W is predicted
    /// cheaper, or at the ceiling. Its label there is as far as it got; the clause that owns it, and every clause that
    /// reads it, are W's on that file. A label past the plan's (`labels + c`) is clause `c`'s roots.
    pub unfinished: Vec<(u32, usize)>,
}
