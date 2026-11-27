//! A program file's text with its includes read in, and the run's settings (`rerail.env`, `--env`) as facts of a
//! program: what a run of the logic reads before it plans (the command line's tools re-export them).

/// A program file's `.rerail` text, with what it `#include`s from the library (`docs/rule-syntax/lib`) read in.
pub fn program_text(path: &std::path::Path) -> Result<String, String> {
    program_texts(&[path]).map(|mut texts| texts.remove(0))
}

/// [`program_text`] for each of `paths`: phase 0 resolves all their `#include`s in one run of `bootstrap.rerail`
/// (`plan::corpus::phase0`), not one run each.
pub fn program_texts(paths: &[&std::path::Path]) -> Result<Vec<String>, String> {
    program_texts_read(paths, paths.iter().map(|_| None).collect())
}

/// A rule file's text, read as the run observes its reads (`program_texts` reads each so).
pub fn read_program_file(path: &std::path::Path) -> Result<String, String> {
    let (bytes, _) = crate::code_rule_dsl::logic::plan::corpus::observe(|o| o.read(path)).ok_or_else(|| format!("{}: cannot read the file", path.display()))?;
    String::from_utf8(bytes).map_err(|error| format!("{}: {error}", path.display()))
}

/// `program_texts` of `paths`, each one's file text given where its reader read it already (`read_program_file`):
/// a directory's files are read to find the libraries others include, and each was read a second time here.
pub fn program_texts_read(paths: &[&std::path::Path], given: Vec<Option<String>>) -> Result<Vec<String>, String> {
    Ok(program_texts_parsed(paths, given)?.0)
}

/// A text's parse, made where its text was read (`rr::text_seed_read`): the run's read of it takes it.
pub type Parsed = Option<std::sync::Arc<crate::code_rule_dsl::logic::rr::from_tape::RrFile>>;

/// `program_texts_read`, and each text's parse where its read made one (a text that includes something), for the run
/// (`corpus::run_with_parsed`): the run's read, its masking and its templates each parsed every text again.
pub fn program_texts_parsed(paths: &[&std::path::Path], given: Vec<Option<String>>) -> Result<(Vec<String>, Vec<Parsed>), String> {
    use crate::code_rule_dsl::logic::plan::corpus::phase0;
    let read = read_program_file;
    // A JSON policy (decision 0175) is its translation: a text whose `#include`s are library files, read as a text's.
    let json: Vec<bool> = paths.iter().map(|p| crate::code_rule_dsl::logic::json_rules::is_policy_path(p)).collect();
    let translator = json.iter().any(|j| *j).then(crate::code_rule_dsl::logic::json_rules::Translator::new);
    let texts: Vec<String> = paths
        .iter()
        .zip(&json)
        .zip(given)
        .map(|((p, j), given)| {
            let text = match given {
                Some(text) => text,
                None => read(p)?,
            };
            if !j {
                return Ok(text);
            }
            let id = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            translator.as_ref().expect("a translator").translate_text(&text, &id).map_err(|e| format!("{}: {e}", p.display()))
        })
        .collect::<Result<_, String>>()?;
    // A policy (`.policy`, `.language`) is read as `run-rr` reads it: its language's base relations are declared by
    // reading the file (O111), which a policy's text does not do itself.
    let policy = |text: &str| text.lines().map(str::trim_start).any(|l| l.starts_with(".policy") || l.starts_with(".language"));
    let includes = |text: &str| text.lines().any(|l| l.trim_start().starts_with("#include"));
    // Each including file's canonical path, found once: its seed's, and the text's below (a walk of the path each).
    let mut seeds = Vec::new();
    for ((path, text), j) in paths.iter().zip(&texts).zip(&json) {
        if includes(text) && !j {
            seeds.push(phase0::Seed::File(dunce::canonicalize(path).map_err(|e| format!("{}: {e}", path.display()))?));
        }
    }
    let mut canonicals = seeds.iter().map(|s| match s {
        phase0::Seed::File(p) => p.clone(),
        phase0::Seed::Text { .. } => unreachable!("a file's seed"),
    });
    let graphs = phase0::graphs(&seeds).map_err(|e| e.to_string())?;
    crate::code_rule_dsl::logic::profile::timeline_mark("the program files' graphs");
    let mut graphs = graphs.into_iter();
    // The library and the phase programs' directories, found once for every text: each was a search for the checkout
    // (`source_tree::root`, 1.9 M instructions), 0.6 G of a 317-file pack's texts when asked per text.
    let dirs = phase0::Dirs::here();
    let mut out = Vec::with_capacity(texts.len());
    let mut parsed: Vec<Parsed> = Vec::with_capacity(texts.len());
    for ((_, text), j) in paths.iter().zip(texts).zip(json) {
        if j || !includes(&text) {
            out.push(if policy(&text) && !includes(&text) { format!("#include \"core.rerail\"\n{text}") } else { text });
            parsed.push(None);
            continue;
        }
        let graph = graphs.next().expect("a graph per seed");
        // The program as modules (decision 0089): the library files it includes stay `#include`s, read once by a
        // run however many of its programs include them; a file beside it is named from the library, a module too
        // (`module_text`). A policy reads `core.rerail`.
        let canonical = canonicals.next().expect("a canonical path per seed");
        let library = dirs.as_ref().map(|d| d.lib.as_path()).ok_or_else(|| "the rule library does not canonicalize".to_owned())?;
        let own = module_text(&canonical, &text, &graph, library)?;
        let own = if policy(&text) && !includes(&own) { format!("#include \"core.rerail\"\n{own}") } else { own };
        // The text's own graph is the file's, cut to what the text includes: kept, so no second run derives it.
        let (seed, file) = crate::code_rule_dsl::logic::rr::text_seed_read(&own).unwrap_or((None, None));
        if let (Some(seed), Some(dirs)) = (seed, &dirs) {
            phase0::keep_text(&seed, &graph, dirs);
        }
        out.push(own);
        parsed.push(file);
    }
    Ok((out, parsed))
}

/// The run's settings (t21 E4): `key=value` lines of `<root>/rerail.env` (blank lines and `#` comments
/// skipped), then each `--env key=value`, a later value for a key replacing an earlier one.
pub fn settings(root: &std::path::Path, flags: &[String]) -> Result<Vec<(String, String)>, String> {
    settings_with_file(root, flags, true)
}

/// `settings`, for what keys a run before it runs (its frame's key, its program's summary): in a Git snapshot whose
/// `rerail.env` cannot be read (a commit whose tree is not in the repository), the flags alone and why, as the run
/// itself reads none where its program reads only Git's metadata (`corpus::run_observed`). Elsewhere, `settings`.
pub fn settings_as_read(root: &std::path::Path, flags: &[String]) -> Result<(Vec<(String, String)>, Option<String>), String> {
    match settings(root, flags) {
        Ok(settings) => Ok((settings, None)),
        Err(why) if crate::git_workspace::snapshot::contains(root) => Ok((settings_with_file(root, flags, false)?, Some(why))),
        Err(why) => Err(why),
    }
}

pub fn settings_with_file(root: &std::path::Path, flags: &[String], read_file: bool) -> Result<Vec<(String, String)>, String> {
    let file = root.join("rerail.env");
    let text = if read_file {
        match crate::git_workspace::snapshot::read_to_string(&file) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(format!("{}: {e}", file.display())),
        }
    } else {
        String::new()
    };
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

/// `text` (the file `path`'s) with each `#include` phase 0 resolved to a file outside the library named by its path
/// from the library (`../../packs/django/rules/lib/django.rerail`), which phase 0 resolves for a text as for a file:
/// the file is a module of the program's library (decision 0639 §1), planned, lowered and kept once however many
/// programs include it. (It was written into each program's text, which lowered it again with every program: a
/// pack's own library, Django's `lib/django.rerail`, was no module.)
fn module_text(path: &std::path::Path, text: &str, graph: &crate::code_rule_dsl::logic::plan::corpus::phase0::Graph, library: &std::path::Path) -> Result<String, String> {
    let from = path.to_string_lossy().into_owned();
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        // As phase 0's graph names it (`rr::include_switched`: the reference graph's file under RERAIL_REFERENCES).
        let name = line.trim_start().strip_prefix("#include").and_then(|rest| rest.split('"').nth(1)).map(super::rr::include_switched);
        match name {
            Some(name) => {
                let found = graph.includes.get(&from).into_iter().flatten().find(|(_, n, _)| *n == name).map(|(_, _, p)| p.clone());
                let Some(found) = found else {
                    return Err(format!("{}: #include {name:?}: no such file", path.display()));
                };
                if found.starts_with(library) {
                    out.push_str(line);
                } else {
                    out.push_str(&format!("#include \"{}\"", from_library(&found, library)));
                }
            }
            None => out.push_str(line),
        }
        out.push('\n');
    }
    Ok(out)
}

/// File `path`'s name from the library directory `library` (both canonical): `..` up to their common directory,
/// then down to the file, with `/` between, as a text's `#include` names it on every host.
fn from_library(path: &std::path::Path, library: &std::path::Path) -> String {
    let (p, l): (Vec<_>, Vec<_>) = (path.components().collect(), library.components().collect());
    let common = p.iter().zip(&l).take_while(|(a, b)| a == b).count();
    let up = std::iter::repeat_n("..".to_owned(), l.len() - common);
    let down = p[common..].iter().map(|c| c.as_os_str().to_string_lossy().into_owned());
    up.chain(down).collect::<Vec<_>>().join("/")
}
