//! A program file's text with its includes read in, and the run's settings (`rerail.env`, `--env`) as facts of a
//! program: what a run of the logic reads before it plans (the command line's tools re-export them).

/// A program file's `.rr` text, with what it `#include`s from the library (`docs/rule-syntax/lib`) read in.
pub fn program_text(path: &std::path::Path) -> Result<String, String> {
    program_texts(&[path]).map(|mut texts| texts.remove(0))
}

/// [`program_text`] for each of `paths`: phase 0 resolves all their `#include`s in one run of `bootstrap.rr`
/// (`plan::corpus::phase0`), not one run each.
pub fn program_texts(paths: &[&std::path::Path]) -> Result<Vec<String>, String> {
    use crate::code_rule_dsl::logic::plan::corpus::{observe, phase0};
    let read = |path: &std::path::Path| -> Result<String, String> {
        let (bytes, _) = observe(|o| o.read(path)).ok_or_else(|| format!("{}: cannot read the file", path.display()))?;
        String::from_utf8(bytes).map_err(|error| format!("{}: {error}", path.display()))
    };
    let texts: Vec<String> = paths.iter().map(|p| read(p)).collect::<Result<_, _>>()?;
    // A policy (`.policy`, `.language`) is read as `run-rr` reads it: its language's base relations are declared by
    // reading the file (O111), which a policy's text does not do itself.
    let policy = |text: &str| text.lines().map(str::trim_start).any(|l| l.starts_with(".policy") || l.starts_with(".language"));
    let includes = |text: &str| text.lines().any(|l| l.trim_start().starts_with("#include"));
    let mut seeds = Vec::new();
    for (path, text) in paths.iter().zip(&texts) {
        if includes(text) {
            seeds.push(phase0::Seed::File(path.canonicalize().map_err(|e| format!("{}: {e}", path.display()))?));
        }
    }
    let graphs = phase0::graphs(&seeds).map_err(|e| e.to_string())?;
    let mut graphs = graphs.into_iter();
    let mut out = Vec::with_capacity(texts.len());
    for (path, text) in paths.iter().zip(texts) {
        if !includes(&text) {
            out.push(if policy(&text) { format!("#include \"core.rr\"\n{text}") } else { text });
            continue;
        }
        let graph = graphs.next().expect("a graph per seed");
        // The program as modules (decision 0089): the library files it includes stay `#include`s, read once by a
        // run however many of its programs include them; a file beside it is written in. A policy reads `core.rr`.
        let canonical = path.canonicalize().map_err(|e| format!("{}: {e}", path.display()))?;
        let own = module_text(&canonical, &text, &graph, &mut Vec::new(), &read)?;
        let own = if policy(&text) && !includes(&own) { format!("#include \"core.rr\"\n{own}") } else { own };
        // The text's own graph is the file's, cut to what the text includes: kept, so no second run derives it.
        if let Ok(Some(seed)) = crate::code_rule_dsl::logic::rr::text_seed(&own) {
            phase0::keep_text(&seed, &graph);
        }
        out.push(own);
    }
    Ok(out)
}

/// The run's settings (t21 E4): `key=value` lines of `<root>/rerail.env` (blank lines and `#` comments
/// skipped), then each `--env key=value`, a later value for a key replacing an earlier one.
pub fn settings(root: &std::path::Path, flags: &[String]) -> Result<Vec<(String, String)>, String> {
    let file = root.join("rerail.env");
    let text = if file.exists() { std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))? } else { String::new() };
    let lines = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(|l| (l, file.display().to_string()));
    let mut settings: Vec<(String, String)> = Vec::new();
    for (line, from) in lines.chain(flags.iter().map(|f| (f.as_str(), "--env".to_owned()))) {
        let (key, value) = line.split_once('=').ok_or_else(|| format!("{from}: `{line}` is not key=value"))?;
        let (key, value) = (key.trim().to_owned(), value.trim().to_owned());
        settings.retain(|(k, _)| *k != key);
        settings.push((key, value));
    }
    Ok(settings)
}

/// `program` with the run's settings as `env(key, value)` facts, for a program that declares `env`; any other
/// program is returned as it is. The facts are part of the program's text, so the plan, and every memo keyed by
/// it, is per setting.
pub fn with_settings(program: &str, settings: &[(String, String)]) -> String {
    let declares = program.lines().any(|l| {
        let l = l.trim_start();
        l.strip_prefix(".decl").is_some_and(|r| r.trim_start().strip_prefix("env").is_some_and(|r| r.trim_start().starts_with('(')))
    });
    // A text in module form (decision 0089) reads `core.rr`, which declares `env`.
    let declares = declares || program.lines().any(|l| l.trim_start().starts_with("#include"));
    if !declares || settings.is_empty() {
        return program.to_owned();
    }
    let quote = crate::code_rule_dsl::logic::rr::quote;
    let facts: String = settings.iter().map(|(k, v)| format!("env({}, {}).\n", quote(k), quote(v))).collect();
    format!("{program}\n// The run's settings (--env, rerail.env).\n{facts}")
}

/// The process's retired instructions and cycles so far (macOS `proc_pid_rusage`).
pub fn process_counters() -> Option<(u64, u64)> {
    #[cfg(target_os = "macos")]
    {
        // SAFETY: a zeroed `rusage_info_v4` is a valid out-value, and the flavour matches it.
        let mut info = unsafe { std::mem::zeroed::<libc::rusage_info_v4>() };
        if unsafe { libc::proc_pid_rusage(std::process::id() as i32, libc::RUSAGE_INFO_V4, (&raw mut info).cast()) } == 0 {
            return Some((info.ri_instructions, info.ri_cycles));
        }
    }
    None
}

/// `text` (the file `path`'s) with each `#include` phase 0 resolved to a file outside the library written in, each
/// once.
fn module_text(
    path: &std::path::Path,
    text: &str,
    graph: &crate::code_rule_dsl::logic::plan::corpus::phase0::Graph,
    seen: &mut Vec<std::path::PathBuf>,
    read: &dyn Fn(&std::path::Path) -> Result<String, String>,
) -> Result<String, String> {
    let library = crate::code_rule_dsl::logic::rr::library().canonicalize().map_err(|e| e.to_string())?;
    let from = path.to_string_lossy().into_owned();
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let name = line.trim_start().strip_prefix("#include").and_then(|rest| rest.split('"').nth(1));
        match name {
            Some(name) => {
                let found = graph.includes.get(&from).into_iter().flatten().find(|(_, n, _)| n == name).map(|(_, _, p)| p.clone());
                let Some(found) = found else {
                    return Err(format!("{}: #include {name:?}: no such file", path.display()));
                };
                if found.starts_with(&library) {
                    out.push_str(line);
                } else if !seen.contains(&found) {
                    seen.push(found.clone());
                    let inner = read(&found)?;
                    out.push_str(&module_text(&found, &inner, graph, seen, read)?);
                }
            }
            None => out.push_str(line),
        }
        out.push('\n');
    }
    Ok(out)
}
