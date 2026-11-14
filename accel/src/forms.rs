//! The kernels' forms (decision 0191): which work ran without the form that would run it faster, how often, and
//! why. Each entry is one (kind, key) seen by this process:
//!
//! - **Interpreted** (`interpreted`): a plan ran on its backend's interpreter while its compiled form was not
//!   ready (`cpu-jit`, `metal-jit`: decision 0012's swap at the plan's next batch; `msl-pipeline`: a kernel run
//!   in another form while its pipeline compiles). Its compile, when it finishes, is recorded (`compiled`) with
//!   its seconds.
//! - **Missing** (`missing`): work that has no form on a lane at all (`metal-form`: a program Metal refuses; a
//!   wave whose Metal run failed; `ir-form`: native work with no program of the IR). The reason is kept.
//!
//! The record is taken by the cost statistics' writer (`drain`) into `costs.sqlite` (`kernel_forms`, per
//! machine, summed over runs): the list of forms to build, ranked by the batches and seconds spent without them.

use std::collections::HashMap;
use std::sync::Mutex;

/// What happened to one (kind, key) in this process since the last `drain`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Form {
    pub kind: &'static str,
    /// The work's identity: a plan's digest, a pipeline's key, a kernel's name.
    pub key: String,
    /// A name to read (the kernel, the program).
    pub name: String,
    /// Batches run without the form.
    pub interpreted: u64,
    /// Compiles finished, their seconds, and failures.
    pub compiled: u64,
    pub compile_seconds: f64,
    pub failed: u64,
    /// Why there is no form (the last reason given).
    pub missing: Option<String>,
}

static FORMS: Mutex<Option<HashMap<(&'static str, String), Form>>> = Mutex::new(None);

fn with(kind: &'static str, key: &str, name: &str, f: impl FnOnce(&mut Form)) {
    let mut forms = FORMS.lock().unwrap_or_else(|p| p.into_inner());
    let e = forms.get_or_insert_with(HashMap::new).entry((kind, key.to_owned())).or_insert_with(|| Form { kind, key: key.to_owned(), name: name.to_owned(), ..Default::default() });
    if e.name.is_empty() {
        e.name = name.to_owned();
    }
    f(e);
}

/// `batches` of `key` ran without its `kind` of form.
pub fn interpreted(kind: &'static str, key: &str, name: &str, batches: u64) {
    with(kind, key, name, |e| e.interpreted += batches);
}

/// `key`'s `kind` of form was compiled in `seconds` (`ok`), or failed.
pub fn compiled(kind: &'static str, key: &str, name: &str, seconds: f64, ok: bool) {
    with(kind, key, name, |e| {
        if ok {
            e.compiled += 1;
            e.compile_seconds += seconds;
        } else {
            e.failed += 1;
        }
    });
}

/// `key` has no `kind` of form, for `reason`; `batches` ran without it.
pub fn missing(kind: &'static str, key: &str, name: &str, reason: &str, batches: u64) {
    with(kind, key, name, |e| {
        e.interpreted += batches;
        e.missing = Some(reason.to_owned());
    });
}

/// The record since the last drain, emptied.
pub fn drain() -> Vec<Form> {
    let mut forms = FORMS.lock().unwrap_or_else(|p| p.into_inner());
    forms.take().map(|m| m.into_values().collect()).unwrap_or_default()
}

/// The record since the last drain, kept (a profile's view).
pub fn peek() -> Vec<Form> {
    let forms = FORMS.lock().unwrap_or_else(|p| p.into_inner());
    forms.as_ref().map(|m| m.values().cloned().collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forms_sum_and_drain() {
        interpreted("test-jit", "k1", "a plan", 2);
        interpreted("test-jit", "k1", "", 3);
        compiled("test-jit", "k1", "", 0.5, true);
        missing("test-metal", "wave", "the wave", "its Metal run failed", 1);
        let mut d: Vec<Form> = drain().into_iter().filter(|f| f.kind.starts_with("test-")).collect();
        d.sort_by(|a, b| a.kind.cmp(b.kind));
        assert_eq!(d.len(), 2);
        assert_eq!((d[0].interpreted, d[0].compiled, d[0].name.as_str()), (5, 1, "a plan"));
        assert_eq!(d[1].missing.as_deref(), Some("its Metal run failed"));
        assert!(peek().iter().all(|f| !f.kind.starts_with("test-")));
    }
}
