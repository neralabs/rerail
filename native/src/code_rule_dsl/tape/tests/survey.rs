//! The engine's own parsers against tree-sitter, over any body of source: Git repositories read in
//! place (`git ls-tree`, `git cat-file --batch`: no checkout, nothing written), directories, or
//! single files. Each file whose dialect has a parser of the engine's own (`tape::tree_parser`, a
//! lean parser or simdjson) is read by it and by tree-sitter, and the documents are compared: the
//! host's tape, and each document derived from it. A Markdown file's inline texts and a Vue file's
//! program are checked again in their own dialects, as texts of their own.
//!
//! Every parser has one behavior: tree-sitter's tape, or a refusal. So each file is one of:
//!
//! | Outcome | Meaning |
//! |---|---|
//! | `same` | the parser read it, and its documents are tree-sitter's |
//! | `refused` | the parser left a text tree-sitter reads without an error to tree-sitter |
//! | `refused-error` | the parser refused a text with a syntax error: as it must |
//! | `differ` | **a finding**: the parser read it, and its documents are not tree-sitter's |
//! | `accepted-error` | **a finding**: the parser read a text tree-sitter reads with an error |
//! | `panic` | **a finding**: the parser panicked |
//! | `failed` | **a finding**: the parser stopped with an error of its own (`Some(Err)`) |
//!
//! A finding's source, and a refused clean text's, is kept in the output directory (`cases/<dialect>/<outcome>/<digest>.<ext>`,
//! a few of each), so a regression test can be written from it. The corpus itself is not kept.

use super::*;
use std::collections::{BTreeMap, HashSet};
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Outcome {
    Same,
    Refused,
    RefusedError,
    Differ,
    AcceptedError,
    Panic,
    Failed,
}

impl Outcome {
    fn name(self) -> &'static str {
        match self {
            Outcome::Same => "same",
            Outcome::Refused => "refused",
            Outcome::RefusedError => "refused-error",
            Outcome::Differ => "differ",
            Outcome::AcceptedError => "accepted-error",
            Outcome::Panic => "panic",
            Outcome::Failed => "failed",
        }
    }

    fn is_finding(self) -> bool {
        matches!(self, Outcome::Differ | Outcome::AcceptedError | Outcome::Panic | Outcome::Failed)
    }
}

/// A dialect's grammar and tree-sitter parser, made once.
struct Reader {
    grammar: Arc<Grammar>,
    parser: tree_sitter::Parser,
}

/// The survey's state: what it has read, what it found, where it writes.
struct Survey {
    out: PathBuf,
    readers: BTreeMap<&'static str, Reader>,
    /// Each text's digest, so a file vendored into many repositories is read once.
    seen: HashSet<u64>,
    /// Files and bytes by dialect and outcome.
    counts: BTreeMap<(&'static str, Outcome), (u64, u64)>,
    /// Cases kept by dialect and outcome.
    kept: BTreeMap<(&'static str, Outcome), usize>,
    keep: usize,
    findings: std::fs::File,
    gaps: std::fs::File,
    /// `current.txt`, rewritten in place for each file (no file made per file).
    progress: std::fs::File,
    /// Places not read: files a parser did not stop on in an earlier run.
    skip: HashSet<String>,
    /// The part of each repository read (`RERAIL_SURVEY_PART`).
    part: Option<(usize, usize)>,
    files: u64,
}

/// The first lines where two tapes' listings differ, with a little context.
fn first_difference(ours: &str, theirs: &str) -> String {
    let (a, b): (Vec<_>, Vec<_>) = (ours.lines().collect(), theirs.lines().collect());
    let at = a.iter().zip(&b).position(|(x, y)| x != y).unwrap_or(a.len().min(b.len()));
    let from = at.saturating_sub(2);
    let mut out = String::new();
    for line in a.iter().skip(from).take(5) {
        out.push_str(&format!("  ours   {line}\n"));
    }
    for line in b.iter().skip(from).take(5) {
        out.push_str(&format!("  theirs {line}\n"));
    }
    out
}

fn digest(label: &str, text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (label, text).hash(&mut h);
    h.finish()
}

impl Survey {
    fn at(&mut self, place: &str) {
        use std::io::Seek;
        let _ = self.progress.seek(std::io::SeekFrom::Start(0));
        let _ = self.progress.write_all(format!("{place}\n").as_bytes());
        let _ = self.progress.set_len(place.len() as u64 + 1);
    }

    fn reader(&mut self, label: &'static str, dialect: GrammarDialect) -> Option<&mut Reader> {
        if !self.readers.contains_key(label) {
            let grammar = Grammar::new(dialect).ok()?;
            let mut parser = tree_sitter::Parser::new();
            parser.set_language(&dialect.get_ts_language()).ok()?;
            self.readers.insert(label, Reader { grammar, parser });
        }
        self.readers.get_mut(label)
    }

    /// One text in `dialect`, labelled `label`, from `place` (where it came from, for the findings).
    fn text(&mut self, label: &'static str, dialect: GrammarDialect, place: &str, ext: &str, source: &str) {
        if self.skip.contains(place) || !self.seen.insert(digest(label, source)) {
            return;
        }
        let Some(lean) = tree_parser(dialect).filter(|parser| parser.kinds().is_none()) else { return };
        self.files += 1;
        // The file being read: where a run is killed (a parser that does not stop), its last line.
        self.at(place);
        let Some(reader) = self.reader(label, dialect) else { return };
        let grammar = Arc::clone(&reader.grammar);
        // tree-sitter reads what the product gives it (`parse_input`): a PHP file's view (its inline
        // text blanked, `php_view`), not the file, whose inline text `php_only` reads as code.
        let view = crate::code_rule_dsl::language::parse_input(dialect, source);
        let Some(tree) = reader.parser.parse(view.as_ref(), None) else { return };
        let error = tree.root_node().has_error();
        let ours = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| lean.parse(&grammar, source, &|| false)));
        let theirs = TapeImage::encode(tree.root_node(), &grammar, &view, &|| false);
        let (outcome, detail) = match ours {
            Err(panic) => {
                let message = panic.downcast_ref::<String>().cloned().or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()));
                (Outcome::Panic, message.unwrap_or_default())
            }
            Ok(None) if error => (Outcome::RefusedError, String::new()),
            Ok(None) => (Outcome::Refused, String::new()),
            Ok(Some(Err(why))) => (Outcome::Failed, format!("{why:?}")),
            Ok(Some(Ok(_))) if error => (Outcome::AcceptedError, String::new()),
            Ok(Some(Ok(image))) => match &theirs {
                Err(why) => (Outcome::Failed, format!("tree-sitter's tape: {why}")),
                Ok(theirs) => {
                    let detail = compare(&image, theirs, &grammar);
                    (if detail.is_empty() { Outcome::Same } else { Outcome::Differ }, detail)
                }
            },
        };
        self.count(label, outcome, place, ext, source, &detail);
        // The texts a file holds in other dialects, read from tree-sitter's tape (what the
        // product embeds is checked above, as the file's derived documents).
        if let Ok(theirs) = theirs {
            self.inner_texts(dialect, &theirs.primary, &grammar, place, source);
        }
    }

    /// A Markdown file's inline texts (as the engine cuts them: less their block continuations),
    /// and a Vue file's program.
    fn inner_texts(&mut self, dialect: GrammarDialect, image: &TapeImage, grammar: &Grammar, place: &str, source: &str) {
        let Some(root) = crate::code_rule_dsl::embed::host::Host::new(image, grammar).root() else { return };
        match dialect {
            GrammarDialect::Markdown => {
                let mut texts = Vec::new();
                let mut stack = vec![root];
                while let Some(node) = stack.pop() {
                    if matches!(node.kind(), "inline" | "pipe_table_cell") {
                        let (mut text, mut from) = (String::new(), node.start_byte());
                        for child in node.children().skip(1) {
                            if child.kind() == "block_continuation" {
                                text.push_str(&source[from..child.start_byte()]);
                                from = child.end_byte();
                            }
                        }
                        text.push_str(&source[from..node.end_byte()]);
                        texts.push(text);
                    } else {
                        stack.extend(node.children());
                    }
                }
                for text in texts {
                    self.text("markdown-inline", GrammarDialect::MarkdownInline, place, "md", &text);
                }
            }
            GrammarDialect::Vue => {
                if let Some(view) = crate::code_rule_dsl::vue::view::View::build(root, source, &|| false) {
                    let (label, ext) = match view.dialect() {
                        GrammarDialect::TypeScript => ("vue-program-ts", "ts"),
                        GrammarDialect::Tsx => ("vue-program-tsx", "tsx"),
                        _ => ("vue-program-js", "js"),
                    };
                    let text = view.text().to_owned();
                    self.text(label, view.dialect(), place, ext, &text);
                }
            }
            _ => {}
        }
    }

    fn count(&mut self, label: &'static str, outcome: Outcome, place: &str, ext: &str, source: &str, detail: &str) {
        let entry = self.counts.entry((label, outcome)).or_default();
        entry.0 += 1;
        entry.1 += source.len() as u64;
        // A clean text refused is a gap, not a finding: tree-sitter reads it. Its cases are kept
        // too, as the parser's next constructs to read.
        if !outcome.is_finding() && outcome != Outcome::Refused {
            return;
        }
        let summary = detail.lines().next().unwrap_or("").replace('\t', " ");
        let name = format!("{:016x}", digest(label, source));
        let list = if outcome == Outcome::Refused { &mut self.gaps } else { &mut self.findings };
        let _ = writeln!(list, "{label}\t{}\t{name}\t{place}\t{summary}", outcome.name());
        let kept = self.kept.entry((label, outcome)).or_default();
        if *kept < self.keep {
            *kept += 1;
            let dir = self.out.join("cases").join(label).join(outcome.name());
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(dir.join(format!("{name}.{ext}")), source);
            let _ = std::fs::write(dir.join(format!("{name}.txt")), format!("{place}\n{detail}"));
        }
    }

    fn summary(&self) -> String {
        let mut out = String::from("dialect\toutcome\tfiles\tbytes\n");
        for ((label, outcome), (files, bytes)) in &self.counts {
            out.push_str(&format!("{label}\t{}\t{files}\t{bytes}\n", outcome.name()));
        }
        out
    }

    /// A file at `path` (shown as `place`), with `source` its bytes.
    fn file(&mut self, place: &str, path: &str, bytes: &[u8]) {
        let Some(label) = crate::code_rule_dsl::language::label_of_path(path) else { return };
        let Some(dialect) = crate::code_rule_dsl::language::dialect_by_label(label) else { return };
        let Ok(source) = std::str::from_utf8(bytes) else { return };
        let ext = path.rsplit('.').next().unwrap_or("txt").to_owned();
        self.text(label, dialect, place, &ext, source);
    }

    /// Git's bounds on the pack data it maps: a large repository's packs are tens of GB.
const PACK_LIMITS: [&str; 6] = ["-c", "core.packedGitLimit=256m", "-c", "core.packedGitWindowSize=32m", "-c", "core.deltaBaseCacheLimit=64m"];

/// Every blob of a Git repository's `HEAD` whose path has a dialect, up to `max` bytes.
    fn repository(&mut self, git: &Path, max: u64) {
        // The listing is streamed: a large repository's (Chromium's) is hundreds of MB.
        let Ok(mut lister) = std::process::Command::new("git")
            .args(Self::PACK_LIMITS)
            .arg("--git-dir")
            .arg(git)
            .args(["ls-tree", "-r", "-z", "-l", "--full-tree", "HEAD"])
            .stdout(std::process::Stdio::piped())
            .spawn()
        else {
            return;
        };
        let mut listing = std::io::BufReader::new(lister.stdout.take().expect("stdout"));
        let mut blobs = Vec::new();
        let mut entry = Vec::new();
        loop {
            entry.clear();
            if listing.read_until(0, &mut entry).unwrap_or(0) == 0 {
                break;
            }
            entry.pop_if(|b| *b == 0);
            let Ok(entry) = std::str::from_utf8(&entry) else { continue };
            let Some((meta, path)) = entry.split_once('\t') else { continue };
            let fields: Vec<&str> = meta.split_whitespace().collect();
            let [mode, kind, sha, size] = fields[..] else { continue };
            if kind != "blob" || mode == "120000" || size.parse::<u64>().map_or(true, |s| s > max) {
                continue;
            }
            if crate::code_rule_dsl::language::label_of_path(path).is_some() {
                blobs.push((sha.to_owned(), path.to_owned()));
            }
        }
        let _ = lister.wait();
        // One part of a large repository (`RERAIL_SURVEY_PART=k/n`: every n-th file from the k-th).
        if let Some((k, n)) = self.part {
            blobs = blobs.into_iter().enumerate().filter(|(i, _)| i % n == k).map(|(_, b)| b).collect();
        }
        if blobs.is_empty() {
            return;
        }
        let Ok(mut child) = std::process::Command::new("git")
            .args(Self::PACK_LIMITS)
            .arg("--git-dir")
            .arg(git)
            .args(["cat-file", "--batch"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
        else {
            return;
        };
        let mut stdin = child.stdin.take().expect("stdin");
        let shas: Vec<String> = blobs.iter().map(|(sha, _)| sha.clone()).collect();
        let writer = std::thread::spawn(move || {
            for sha in shas {
                if writeln!(stdin, "{sha}").is_err() {
                    break;
                }
            }
        });
        let mut stdout = std::io::BufReader::new(child.stdout.take().expect("stdout"));
        let name = git.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut header = String::new();
        for (_, path) in &blobs {
            header.clear();
            if stdout.read_line(&mut header).unwrap_or(0) == 0 {
                break;
            }
            let Some(size) = header.split_whitespace().nth(2).and_then(|s| s.parse::<usize>().ok()) else { continue };
            let mut bytes = vec![0; size + 1];
            if stdout.read_exact(&mut bytes).is_err() {
                break;
            }
            bytes.pop();
            self.file(&format!("{name}:{path}"), path, &bytes);
        }
        let _ = writer.join();
        let _ = child.wait();
    }

    /// Every file under `dir` whose path has a dialect, up to `max` bytes.
    fn directory(&mut self, dir: &Path, max: u64) {
        let mut stack = vec![dir.to_path_buf()];
        while let Some(at) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&at) else { continue };
            let mut entries: Vec<_> = entries.flatten().collect();
            entries.sort_by_key(|e| e.file_name());
            for entry in entries {
                let path = entry.path();
                let Ok(kind) = entry.file_type() else { continue };
                if kind.is_dir() {
                    if entry.file_name() != ".git" {
                        stack.push(path);
                    }
                } else if kind.is_file() && entry.metadata().is_ok_and(|m| m.len() <= max) {
                    let shown = path.to_string_lossy().into_owned();
                    if crate::code_rule_dsl::language::label_of_path(&shown).is_some()
                        && let Ok(bytes) = std::fs::read(&path)
                    {
                        self.file(&shown, &shown, &bytes);
                    }
                }
            }
        }
    }
}

/// Each tape the parser wrote against tree-sitter's: empty where all are the same, else the first
/// difference.
fn compare(ours: &FileImage, theirs: &FileImage, grammar: &Grammar) -> String {
    if ours.primary.to_bytes().ok() != theirs.primary.to_bytes().ok() {
        return format!("the host's tape\n{}", first_difference(&python_tape_plain(&ours.primary, grammar), &python_tape_plain(&theirs.primary, grammar)));
    }
    if ours.derived.len() != theirs.derived.len() {
        return format!("{} derived documents, tree-sitter's {}", ours.derived.len(), theirs.derived.len());
    }
    for (i, (a, b)) in ours.derived.iter().zip(&theirs.derived).enumerate() {
        if a.text != b.text {
            return format!("derived document {i}'s text");
        }
        if a.tape.to_bytes().ok() != b.tape.to_bytes().ok() {
            return format!("derived document {i}'s tape");
        }
    }
    String::new()
}

/// The survey (see the module's doc). `RERAIL_SURVEY_INPUTS` lists its inputs, a path a line: a
/// Git repository (bare or not; its `HEAD` is read in place), a directory, or a file.
/// `RERAIL_SURVEY_OUT` is where it writes `summary.tsv`, `findings.tsv`, `gaps.tsv` (clean texts
/// refused) and `cases/`.
/// `RERAIL_SURVEY_MAX_BYTES` (default 1,000,000) skips larger files; `RERAIL_SURVEY_KEEP` (default
/// 20) is how many cases of each dialect and outcome are kept. `RERAIL_SURVEY_SKIP` lists places
/// (as `current.txt` names them) not to read. `RERAIL_SURVEY_PART=k/n` reads one part of each
/// repository: every n-th of its files, from the k-th.
#[test]
#[ignore = "needs RERAIL_SURVEY_INPUTS and RERAIL_SURVEY_OUT"]
fn lean_parsers_survey() {
    let inputs = std::fs::read_to_string(std::env::var("RERAIL_SURVEY_INPUTS").expect("set RERAIL_SURVEY_INPUTS")).expect("inputs");
    let out = PathBuf::from(std::env::var("RERAIL_SURVEY_OUT").expect("set RERAIL_SURVEY_OUT"));
    std::fs::create_dir_all(&out).expect("output directory");
    let max = std::env::var("RERAIL_SURVEY_MAX_BYTES").ok().and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let keep = std::env::var("RERAIL_SURVEY_KEEP").ok().and_then(|s| s.parse().ok()).unwrap_or(20);
    let mut survey = Survey {
        findings: std::fs::File::create(out.join("findings.tsv")).expect("findings"),
        gaps: std::fs::File::create(out.join("gaps.tsv")).expect("gaps"),
        progress: std::fs::File::create(out.join("current.txt")).expect("current.txt"),
        out: out.clone(),
        readers: BTreeMap::new(),
        seen: HashSet::new(),
        counts: BTreeMap::new(),
        kept: BTreeMap::new(),
        keep,
        skip: std::env::var("RERAIL_SURVEY_SKIP")
            .ok()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|text| text.lines().map(str::to_owned).collect())
            .unwrap_or_default(),
        part: std::env::var("RERAIL_SURVEY_PART").ok().and_then(|p| {
            let (k, n) = p.split_once('/')?;
            Some((k.parse().ok()?, n.parse::<usize>().ok().filter(|&n| n > 0)?))
        }),
        files: 0,
    };
    // A panic's message is the finding's; the default hook would print each one as well.
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    for input in inputs.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let path = Path::new(input);
        if survey.skip.contains(input) {
            continue;
        }
        survey.at(input);
        if path.is_file() {
            if let Ok(bytes) = std::fs::read(path) {
                survey.file(input, input, &bytes);
            }
        } else if path.join("HEAD").is_file() && path.join("objects").is_dir() {
            survey.repository(path, max);
        } else if path.join(".git").exists() {
            survey.repository(&path.join(".git"), max);
        } else {
            survey.directory(path, max);
        }
        std::fs::write(out.join("summary.tsv"), survey.summary()).expect("summary");
    }
    std::panic::set_hook(hook);
    std::fs::write(out.join("summary.tsv"), survey.summary()).expect("summary");
    survey.at("done");
    eprint!("{}", survey.summary());
}
