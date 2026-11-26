//! The error a program's compile and a device's run return: one type, which `rerail_compiler::Error` and
//! `rerail_metal::Error` re-export. In a crate of its own, below both, so that the device's runtime needs no
//! compiler.

//! The transducer's errors.

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidOpcode(usize),
    InvalidMode(usize),
    InvalidOperand(usize),
    InvalidTarget(usize),
    NonProgressingCycle(usize),
    /// A tier-2 word in a phase it may not run in (`stream::phases`): a word that reads the
    /// next token with it taken, or a word other than `TOKEN` reached in both.
    Phase(usize),
    InvalidGreedyRegion(usize),
    WorkLimit,
    /// A task stopped at its budget, below the ceiling (`WorkBudget::Replan`): unfinished, not failed. Its
    /// evaluation is replanned and restarted (decision 0334).
    OverBudget,
    /// A task ran out of a state quota sized from its predicted steps, below the worst case's: unfinished, not
    /// failed. Its evaluation restarts with twice the states (decision 0334 §9).
    StatesOutgrown,
    UnsupportedPattern,
    UnsupportedFeature(&'static str),
    UnknownNodeKind(String),
    UnknownField(String),
    UnknownCapture(String),
    ScmParse { offset: usize, message: String },
    InvalidAnchor,
    UnsupportedBackend,
    BackendCapacity,
    /// A label run's workspace is over Metal's buffer budget (`label_passes::run` halves its lanes'
    /// states; past the fewest it is `BackendCapacity`).
    BackendBudget,
    /// A label run's micro walk outgrew its frames, captures or steps on the device: the run
    /// again with deep stacks, then interpreted. Not a size refusal: the batch fits; its walks'
    /// pools do not.
    WalkFrames,
    BackendFailure,
    /// The system stopped the command before it completed, the work not at fault: macOS's GPU watchdog ends a
    /// command buffer that holds the GPU while the display waits ("Impacting Interactivity"), as several at once on
    /// a Mac with a display do. The run again is the caller's, a bounded number of times.
    BackendPreempted,
    /// A generated function with a call or an unstated loop (`native_cpu_shape`): refused.
    RefusedShape(String),
}

/// The executor's GPU lane reports its failures as messages; to the engine they are a backend failure, as the other
/// device paths' (`metal_join`, `metal_rows`).
impl From<String> for Error {
    fn from(message: String) -> Error {
        // Said where it happens: the error's kind is all its callers keep, and a device failure is never silent.
        eprintln!("rerail: a device path failed: {message}");
        Error::BackendFailure
    }
}
