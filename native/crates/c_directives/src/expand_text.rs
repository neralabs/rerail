/// The program's text for C (its `; C++ only` lines left out) or C++, from `expand.rir`'s.
/// Shared with the build script (`build.rs`, the `generated` feature), so both read one program.
pub fn text_of(rir: &str, cpp: bool) -> String {
    rir.lines().filter(|l| cpp || !l.trim_end().ends_with("; C++ only")).map(|l| format!("{l}\n")).collect()
}
