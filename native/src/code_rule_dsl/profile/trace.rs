//! Opt-in spans for bottom-up investigations. Separate from aggregate CPU accounting.
use super::thread_cpu_ns;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::io::Write;
use std::sync::LazyLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

static ENABLED: LazyLock<bool> = LazyLock::new(|| std::env::var_os("RERAIL_PROFILE").is_some());
pub(crate) fn enabled() -> bool {
    *ENABLED
}

#[derive(Clone, Default)]
struct Attribution {
    rule: String,
    clause: String,
    path: Option<String>,
}

thread_local! {
    static CONTEXT: RefCell<Attribution> = RefCell::new(Attribution::default());
}

pub(crate) fn emit(prefix: &str, value: Value) {
    // Profiling must never panic when a consumer closes stderr or uses a nonblocking pipe.
    let _ = emit_to(&mut std::io::stderr().lock(), prefix, value);
}

fn emit_to(writer: &mut impl Write, prefix: &str, value: Value) -> std::io::Result<()> {
    let line = format!("{prefix} {value}\n");
    writer.write_all(line.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn trace_line_is_serialized_before_entering_the_shared_output_stream() {
        #[derive(Default)]
        struct Output {
            calls: usize,
            bytes: Vec<u8>,
        }
        impl Write for Output {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.calls += 1;
                self.bytes.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut output = Output::default();
        emit_to(
            &mut output,
            "RERAIL_TRACE",
            json!({"phase":"producer-wave", "detail":{"files":[1,2,3],"message":"capture"}}),
        )
        .unwrap();
        assert_eq!(
            output.calls, 1,
            "formatting fragments allow other stderr writers to split JSON records"
        );
        let line = std::str::from_utf8(&output.bytes).unwrap();
        assert_eq!(line.lines().count(), 1);
        assert!(serde_json::from_str::<Value>(line.strip_prefix("RERAIL_TRACE ").unwrap()).is_ok());
    }
}

pub(crate) struct Context(Option<Attribution>);
impl Context {
    pub(crate) fn enter(rule: &str, clause: &str, path: Option<&str>) -> Self {
        if !*ENABLED {
            return Self(None);
        }
        Self(Some(CONTEXT.with(|context| {
            context.replace(Attribution {
                rule: rule.to_owned(),
                clause: clause.to_owned(),
                path: path.map(str::to_owned),
            })
        })))
    }
}
impl Drop for Context {
    fn drop(&mut self) {
        if let Some(previous) = self.0.take() {
            CONTEXT.with(|context| context.replace(previous));
        }
    }
}

pub(crate) struct Span(Option<ActiveSpan>);
struct ActiveSpan {
    context: Attribution,
    phase: &'static str,
    name: String,
    started: Instant,
    unix_us: u128,
    cpu_ns: Option<u128>,
    detail: Value,
    memory: Option<super::memory::Sample>,
}
impl Span {
    pub(crate) fn start(phase: &'static str, name: &str) -> Self {
        Self(if *ENABLED {
            Some(ActiveSpan {
                context: CONTEXT.with(|context| context.borrow().clone()),
                phase,
                name: name.to_owned(),
                started: Instant::now(),
                unix_us: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_micros(),
                cpu_ns: thread_cpu_ns(),
                detail: Value::Null,
                memory: super::memory::sample(phase),
            })
        } else {
            None
        })
    }
    pub(crate) fn detail(&mut self, value: impl FnOnce() -> Value) {
        if let Some(span) = &mut self.0 {
            span.detail = value();
        }
    }
}
impl Drop for Span {
    fn drop(&mut self) {
        if let Some(span) = &self.0 {
            emit(
                "RERAIL_TRACE",
                json!({
                    "version": 1, "rule": span.context.rule, "clause": span.context.clause,
                    "path": span.context.path, "phase": span.phase, "name": span.name,
                    "startUnixUs": span.unix_us, "wallNs": span.started.elapsed().as_nanos(),
                    "cpuNs": span.cpu_ns.zip(thread_cpu_ns()).map(|(start, end)| end.saturating_sub(start)),
                    "thread": format!("{:?}", std::thread::current().id()), "detail": span.detail,
                    "memory":span.memory.map(|before|json!({"before":before,"after":super::memory::sample(span.phase)})),
                }),
            );
        }
    }
}
