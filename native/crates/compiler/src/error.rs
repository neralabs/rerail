//! The transducer's errors.

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidOpcode(usize),
    InvalidMode(usize),
    InvalidOperand(usize),
    InvalidTarget(usize),
    NonProgressingCycle(usize),
    InvalidGreedyRegion(usize),
    WorkLimit,
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
    BackendLaunchLimit,
    /// A label run's micro walk outgrew its frames, captures or steps on the device: the run
    /// again with deep stacks, then interpreted. Not a size refusal: the batch fits; its walks'
    /// pools do not.
    WalkFrames,
    BackendFailure,
    /// A generated function with a call or an unstated loop (`native_cpu_shape`): refused.
    RefusedShape(String),
}

/// The executor's GPU lane reports its failures as messages; to the engine they are a backend failure, as the other
/// device paths' (`metal_join`, `metal_rows`).
impl From<String> for Error {
    fn from(_: String) -> Error {
        Error::BackendFailure
    }
}
