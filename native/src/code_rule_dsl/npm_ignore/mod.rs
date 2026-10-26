//! Which files of a package npm publishes, as eslint-plugin-n 18.4.1 reads it
//! (`lib/util/get-npmignore.js`, `file.package.published`): the manifest's `files` whitelist, the
//! package folder's `.npmignore`, or its `.gitignore` where it has neither, through the npm
//! `ignore` 5.3.2 package (node-ignore) and, for `files` entries with braces or extglobs, `globrex`
//! 0.1.2. Ported from those three (MIT; see THIRD_PARTY_NOTICES and `npm_ignore/UPSTREAM.md`).
//!
//! node-ignore turns each pattern into a JavaScript regex by a chain of textual replacements
//! (`make_regex`); the port runs the same chain, and the resulting regexes, on `fancy_regex`, which
//! reads the lookaheads they hold. A pattern whose regex does not compile is left out, where
//! node-ignore would throw and eslint-plugin-n's rule would fail on the file.
//!
//! The tests check the port against `npm_ignore/cases.json`, the packages' own answers recorded by
//! `scripts/generate/npm-ignore/npm-ignore-cases.mjs`.

use crate::code_rule_dsl::package_manifest::resolve_target;
use fancy_regex::{Captures, Regex};
use std::sync::LazyLock;

/// A node-ignore pattern's regex, or a `files` matcher's, run with this bound on backtracking; a
/// path the bound stops counts as unmatched.
const BACKTRACK_LIMIT: usize = 100_000;

/// The most patterns one `files` entry's brace sequences expand to (`expand_sequence_braces`);
/// past it the entry is matched unexpanded, where eslint-plugin-n would expand every one.
const MAX_EXPANDED_PATTERNS: usize = 4_096;

fn compile(source: &str) -> Option<Regex> {
    fancy_regex::RegexBuilder::new(source)
        .case_insensitive(true)
        .backtrack_limit(BACKTRACK_LIMIT)
        .build()
        .ok()
}

fn literal(source: &str) -> Regex {
    Regex::new(source).expect("the port's own regexes compile")
}

/// JavaScript's `String.prototype.replace` with a function: the first match, or every match
/// (`global`), replaced by what `replacement` returns for it, never read as a template.
fn replace(
    text: &str,
    pattern: &Regex,
    global: bool,
    mut replacement: impl FnMut(&Captures<'_>) -> String,
) -> String {
    let limit = usize::from(!global);
    pattern
        .replacen(text, limit, |captures: &Captures<'_>| replacement(captures))
        .into_owned()
}

fn group<'c>(captures: &'c Captures<'_>, index: usize) -> &'c str {
    captures.get(index).map_or("", |found| found.as_str())
}

/// A JavaScript character class's body for the Rust regex syntax: where JavaScript reads `[`,
/// `&` and `~` in a class literally, Rust reads nested classes and set operations (`[[:alnum:]]`,
/// `&&`, `~~`), so they are escaped; an escape already there is kept.
fn class_body(body: &str) -> String {
    let mut escaped = String::with_capacity(body.len());
    let mut characters = body.chars();
    while let Some(character) = characters.next() {
        match character {
            '\\' => {
                escaped.push('\\');
                escaped.extend(characters.next());
            }
            '[' | '&' | '~' => {
                escaped.push('\\');
                escaped.push(character);
            }
            _ => escaped.push(character),
        }
    }
    escaped
}

/// One node-ignore rule: a pattern's regex, and whether it re-includes (`!`).
struct IgnoreRule {
    negative: bool,
    regex: Regex,
}

/// node-ignore 5.3.2's `ignore()` with its default options (`ignorecase: true`): `add`, then
/// `ignores`.
#[derive(Default)]
pub(crate) struct NodeIgnore {
    rules: Vec<IgnoreRule>,
}

impl NodeIgnore {
    /// `add(text)`: each line (`\r?\n`) that is a pattern (`checkPattern`).
    pub(crate) fn add(&mut self, text: &str) {
        static LINES: LazyLock<Regex> = LazyLock::new(|| literal(r"\r?\n"));
        let mut start = 0;
        for found in LINES.find_iter(text).flatten() {
            self.add_pattern(&text[start..found.start()]);
            start = found.end();
        }
        self.add_pattern(&text[start..]);
    }

    fn add_pattern(&mut self, pattern: &str) {
        static BLANK: LazyLock<Regex> = LazyLock::new(|| literal(r"^\s+$"));
        static TRAILING_BACKSLASH: LazyLock<Regex> = LazyLock::new(|| literal(r"(?:[^\\]|^)\\$"));
        if pattern.is_empty()
            || BLANK.is_match(pattern).unwrap_or(false)
            || TRAILING_BACKSLASH.is_match(pattern).unwrap_or(false)
            || pattern.starts_with('#')
        {
            return;
        }
        let (negative, pattern) = match pattern.strip_prefix('!') {
            Some(rest) => (true, rest),
            None => (false, pattern),
        };
        let pattern = if let Some(rest) = pattern.strip_prefix("\\!") {
            format!("!{rest}")
        } else if let Some(rest) = pattern.strip_prefix("\\#") {
            format!("#{rest}")
        } else {
            pattern.to_owned()
        };
        if let Some(regex) = compile(&make_regex(&pattern)) {
            self.rules.push(IgnoreRule { negative, regex });
        }
    }

    /// `ignores(path)` for a `path.relative()`d path: ignored where an ancestor folder (tested as
    /// `dir/`) is, since a file below an excluded folder cannot be re-included; else where the
    /// last rule matching the path is no negation.
    pub(crate) fn ignores(&self, path: &str) -> bool {
        if self.rules.is_empty() {
            return false;
        }
        for (at, _) in path.match_indices('/') {
            if self.test_one(&path[..=at]) {
                return true;
            }
        }
        self.test_one(path)
    }

    /// `_testOne(path, false).ignored`: the last matching rule decides.
    fn test_one(&self, path: &str) -> bool {
        let mut ignored = false;
        for rule in &self.rules {
            if rule.negative != ignored {
                continue;
            }
            if rule.regex.is_match(path).unwrap_or(false) {
                ignored = !rule.negative;
            }
        }
        ignored
    }
}

/// node-ignore's `makeRegex`: its `REPLACERS`, in order, over the pattern (the `!` stripped and
/// a leading `\!` or `\#` unescaped).
fn make_regex(pattern: &str) -> String {
    static TRAILING_SPACES: LazyLock<Regex> = LazyLock::new(|| literal(r"((?:\\\\)*?)(\\?\s+)$"));
    static ESCAPED_SPACE: LazyLock<Regex> = LazyLock::new(|| literal(r"(\\+?)\s"));
    static METACHARACTERS: LazyLock<Regex> = LazyLock::new(|| literal(r"[\\$.|*+(){^]"));
    static LEADING_GLOBSTAR: LazyLock<Regex> = LazyLock::new(|| literal(r"^\^*\\\*\\\*\\/"));
    static INNER_SLASH: LazyLock<Regex> = LazyLock::new(|| literal(r"/(?!$)"));
    static TWO_GLOBSTARS: LazyLock<Regex> = LazyLock::new(|| literal(r"\\/\\\*\\\*(?=\\/|$)"));
    static WILDCARDS: LazyLock<Regex> = LazyLock::new(|| literal(r"(^|[^\\]+)(\\\*)+(?=.+)"));
    static ESCAPED_WILDCARD: LazyLock<Regex> = LazyLock::new(|| literal(r"\\\*"));
    static UNESCAPE_METACHARACTER: LazyLock<Regex> =
        LazyLock::new(|| literal(r"\\\\\\(?=[$.|*+(){^])"));
    static DOUBLE_BACKSLASH: LazyLock<Regex> = LazyLock::new(|| literal(r"\\\\"));
    static RANGE: LazyLock<Regex> = LazyLock::new(|| literal(r"(\\)?\[([^\]/]*?)(\\*)($|\])"));
    static RANGE_BOUNDS: LazyLock<Regex> = LazyLock::new(|| literal(r"([0-z])-([0-z])"));
    static ENDING: LazyLock<Regex> = LazyLock::new(|| literal(r"(?:[^*])$"));
    static TRAILING_WILDCARD: LazyLock<Regex> = LazyLock::new(|| literal(r"(\^|\\/)?\\\*$"));

    // Remove a BOM.
    let mut source = pattern
        .strip_prefix('\u{FEFF}')
        .unwrap_or(pattern)
        .to_owned();
    // Trailing spaces are ignored unless escaped.
    source = replace(&source, &TRAILING_SPACES, false, |captures| {
        let escaped = group(captures, 2).starts_with('\\');
        format!("{}{}", group(captures, 1), if escaped { " " } else { "" })
    });
    // `\ ` is a space.
    source = replace(&source, &ESCAPED_SPACE, true, |captures| {
        let slashes = group(captures, 1);
        format!("{} ", &slashes[..slashes.len() - slashes.len() % 2])
    });
    // Escape the regex metacharacters.
    source = replace(&source, &METACHARACTERS, true, |captures| {
        format!("\\{}", group(captures, 0))
    });
    // `?` matches one character but a slash.
    source = source.replace('?', "[^/]");
    // A leading slash anchors at the start.
    if let Some(rest) = source.strip_prefix('/') {
        source = format!("^{rest}");
    }
    source = source.replace('/', "\\/");
    // A leading `**/` matches in every folder.
    source = replace(&source, &LEADING_GLOBSTAR, false, |_| {
        "^(?:.*\\/)?".to_owned()
    });
    // Without a leading `^`, a pattern with a slash before its end is relative to the base,
    // and any other matches at any depth.
    if source.chars().next().is_some_and(|first| first != '^') {
        let anchored = INNER_SLASH.is_match(pattern).unwrap_or(false);
        source = format!("{}{source}", if anchored { "^" } else { "(?:^|\\/)" });
    }
    // `/**/` matches zero or more folders, a trailing `/**` everything inside.
    let length = source.len();
    source = replace(&source, &TWO_GLOBSTARS, true, |captures| {
        let at = captures.get(0).map_or(0, |found| found.start());
        if at + 6 < length {
            "(?:\\/[^\\/]+)*".to_owned()
        } else {
            "\\/.+".to_owned()
        }
    });
    // An unescaped `*` (consecutive ones as one) matches anything but a slash, where it is not
    // the pattern's last character.
    source = replace(&source, &WILDCARDS, true, |captures| {
        let wildcards = ESCAPED_WILDCARD
            .replace_all(group(captures, 2), |_: &Captures<'_>| "[^\\/]*".to_owned())
            .into_owned();
        format!("{}{wildcards}", group(captures, 1))
    });
    // Unescape what step 3 escaped twice, then `\\` to `\`.
    source = replace(&source, &UNESCAPE_METACHARACTER, true, |_| "\\".to_owned());
    source = replace(&source, &DOUBLE_BACKSLASH, true, |_| "\\".to_owned());
    // Range notation.
    source = replace(&source, &RANGE, true, |captures| {
        let (range, end_escape, close) =
            (group(captures, 2), group(captures, 3), group(captures, 4));
        if captures.get(1).is_some() {
            let even = &end_escape[..end_escape.len() - end_escape.len() % 2];
            format!("\\[{range}{even}{close}")
        } else if close == "]" && end_escape.len() % 2 == 0 {
            let sanitized = replace(range, &RANGE_BOUNDS, true, |bounds| {
                if group(bounds, 1) <= group(bounds, 2) {
                    group(bounds, 0).to_owned()
                } else {
                    String::new()
                }
            });
            format!("[{}]", class_body(&format!("{sanitized}{end_escape}")))
        } else {
            "[]".to_owned()
        }
    });
    // The ending: `foo/` matches a folder only, `foo` a file or a folder.
    source = replace(&source, &ENDING, false, |captures| {
        let last = group(captures, 0);
        if last.ends_with('/') {
            format!("{last}$")
        } else {
            format!("{last}(?=$|\\/$)")
        }
    });
    // A trailing wildcard.
    replace(&source, &TRAILING_WILDCARD, false, |captures| {
        let prefix = match captures.get(1) {
            Some(slash) => format!("{}[^/]+", slash.as_str()),
            None => "[^/]*".to_owned(),
        };
        format!("{prefix}(?=$|\\/$)")
    })
}

/// globrex 0.1.2's regex for `glob` with `extended` and `globstar` (eslint-plugin-n's
/// `createFilesMatcher` options), on a POSIX host, anchored at both ends.
fn globrex(glob: &str) -> String {
    const GLOBSTAR: &str = "((?:[^/]*(?:/|$))*)";
    const WILDCARD: &str = "([^/]*)";
    let glob = glob.chars().collect::<Vec<_>>();
    let mut regex = String::new();
    let mut in_group = false;
    let mut in_range = false;
    let mut ext = Vec::new();
    let mut i = 0;
    while i < glob.len() {
        let c = glob[i];
        let n = glob.get(i + 1).copied();
        match c {
            '\\' | '$' | '^' | '.' | '=' => {
                regex.push('\\');
                regex.push(c);
            }
            '/' => {
                regex.push_str("\\/");
                if n == Some('/') {
                    regex.push('?');
                }
            }
            '(' | '|' if !ext.is_empty() => regex.push(c),
            '(' | '|' => {
                regex.push('\\');
                regex.push(c);
            }
            ')' => match ext.pop() {
                Some(kind) => {
                    regex.push(')');
                    match kind {
                        '@' => regex.push_str("{1}"),
                        '!' => regex.push_str("([^/]*)"),
                        other => regex.push(other),
                    }
                }
                None => regex.push_str("\\)"),
            },
            '+' if n == Some('(') => ext.push(c),
            '+' => regex.push_str("\\+"),
            '@' if n == Some('(') => ext.push(c),
            '!' if in_range => regex.push('^'),
            '!' if n == Some('(') => {
                ext.push(c);
                regex.push_str("(?!");
                i += 1;
            }
            '!' => regex.push_str("\\!"),
            '?' if n == Some('(') => ext.push(c),
            '?' => regex.push('.'),
            '[' if in_range && n == Some(':') => {
                i += 2;
                let mut class = String::new();
                while i < glob.len() && glob[i] != ':' {
                    class.push(glob[i]);
                    i += 1;
                }
                match class.as_str() {
                    "alnum" => regex.push_str("(\\w|\\d)"),
                    "space" => regex.push_str("\\s"),
                    "digit" => regex.push_str("\\d"),
                    _ => {}
                }
                i += 1;
            }
            // JavaScript reads `[`, `&` and `~` in a class literally (`class_body`).
            '[' | '&' | '~' if in_range => {
                regex.push('\\');
                regex.push(c);
            }
            '[' => {
                in_range = true;
                regex.push(c);
            }
            ']' => {
                in_range = false;
                regex.push(c);
            }
            '{' => {
                in_group = true;
                regex.push('(');
            }
            '}' => {
                in_group = false;
                regex.push(')');
            }
            ',' if in_group => regex.push('|'),
            ',' => regex.push_str("\\,"),
            '*' if n == Some('(') => ext.push(c),
            '*' => {
                let previous = i.checked_sub(1).map(|at| glob[at]);
                let mut stars = 1;
                while glob.get(i + 1) == Some(&'*') {
                    stars += 1;
                    i += 1;
                }
                let next = glob.get(i + 1).copied();
                if stars > 1
                    && matches!(previous, None | Some('/'))
                    && matches!(next, None | Some('/'))
                {
                    regex.push_str(GLOBSTAR);
                    i += 1;
                } else {
                    regex.push_str(WILDCARD);
                }
            }
            _ => regex.push(c),
        }
        i += 1;
    }
    format!("^{regex}$")
}

/// eslint-plugin-n's `expandSequenceBraces`: each numeric or alphabetic brace sequence
/// (`{1..3}`, `{a..c}`, `{01..10..2}`) expanded, other braces left to globrex.
fn expand_sequence_braces(patterns: Vec<String>, budget: &mut usize) -> Vec<String> {
    static SEQUENCE: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(?i)\{(-?[0-9]+|[a-z])\.\.(-?[0-9]+|[a-z])(?:\.\.(-?[0-9]+))?\}")
            .expect("the sequence regex compiles")
    });
    let mut expanded = Vec::new();
    for pattern in patterns {
        let Some(found) = SEQUENCE.captures(&pattern) else {
            expanded.push(pattern);
            continue;
        };
        let (start, end) = (&found[1], &found[2]);
        let is_number = |text: &str| {
            text.trim_start_matches('-')
                .bytes()
                .all(|b| b.is_ascii_digit())
        };
        let numeric = is_number(start) && is_number(end);
        let alphabetic = start.len() == 1
            && end.len() == 1
            && start.bytes().all(|b| b.is_ascii_alphabetic())
            && end.bytes().all(|b| b.is_ascii_alphabetic());
        let step = found
            .get(3)
            .map_or(Some(1), |step| step.as_str().parse::<i64>().ok());
        let values = if numeric {
            start.parse::<i64>().ok().zip(end.parse::<i64>().ok())
        } else if alphabetic {
            Some((i64::from(start.as_bytes()[0]), i64::from(end.as_bytes()[0])))
        } else {
            None
        };
        let (Some((first, last)), Some(step)) = (values, step.filter(|step| *step > 0)) else {
            expanded.push(pattern);
            continue;
        };
        let count = (first.abs_diff(last) / step.unsigned_abs()).saturating_add(1);
        if usize::try_from(count).map_or(true, |count| count > *budget) {
            expanded.push(pattern);
            continue;
        }
        *budget -= count as usize;
        let width = if numeric && start.len() > 1 && start.starts_with('0') {
            start.len()
        } else {
            0
        };
        let whole = found.get(0).expect("the whole match");
        let (prefix, suffix) = (&pattern[..whole.start()], &pattern[whole.end()..]);
        let direction = if first <= last { 1 } else { -1 };
        let mut replacements = Vec::new();
        let mut value = first;
        while (direction > 0 && value <= last) || (direction < 0 && value >= last) {
            let text = if numeric {
                let text = value.to_string();
                format!("{}{text}", "0".repeat(width.saturating_sub(text.len())))
            } else {
                char::from_u32(value as u32)
                    .map(String::from)
                    .unwrap_or_default()
            };
            replacements.push(format!("{prefix}{text}{suffix}"));
            value += direction * step;
        }
        expanded.extend(expand_sequence_braces(replacements, budget));
    }
    expanded
}

/// eslint-plugin-n's `createFilesMatcher`: the pattern and everything below it (`pattern/**`),
/// with, for a negated entry without a slash (`match_basename`), the name at any depth.
fn files_matcher(pattern: &str, match_basename: bool) -> Vec<Regex> {
    let mut patterns = vec![pattern.to_owned(), format!("{pattern}/**")];
    if match_basename && !pattern.contains('/') {
        patterns.push(format!("**/{pattern}"));
    }
    let mut budget = MAX_EXPANDED_PATTERNS;
    expand_sequence_braces(patterns, &mut budget)
        .iter()
        .filter_map(|pattern| compile(&globrex(pattern)))
        .collect()
}

fn any_match(matchers: &[Regex], path: &str) -> bool {
    matchers
        .iter()
        .any(|matcher| matcher.is_match(path).unwrap_or(false))
}

/// Node's `path.posix.normalize`.
fn posix_normalize(path: &str) -> String {
    if path.is_empty() {
        return ".".to_owned();
    }
    let absolute = path.starts_with('/');
    let trailing = path.ends_with('/');
    let mut segments: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                if segments.last().is_some_and(|last| *last != "..") {
                    segments.pop();
                } else if !absolute {
                    segments.push("..");
                }
            }
            _ => segments.push(segment),
        }
    }
    let mut normalized = segments.join("/");
    if normalized.is_empty() {
        return match (absolute, trailing) {
            (true, _) => "/".to_owned(),
            (false, true) => "./".to_owned(),
            (false, false) => ".".to_owned(),
        };
    }
    if trailing {
        normalized.push('/');
    }
    if absolute {
        format!("/{normalized}")
    } else {
        normalized
    }
}

/// eslint-plugin-n's `parseWhiteList`: the `files` field as a matcher of the files it leaves out.
struct FilesWhitelist {
    /// The plain entries, each as `/{body}` and `/{body}/**`.
    include: NodeIgnore,
    /// The plain negated entries, each as `{body}` and `{body}/**`.
    exclude: NodeIgnore,
    /// The entries with braces or extglobs (`hasExtendedGlob`), through globrex.
    include_globs: Vec<Vec<Regex>>,
    exclude_globs: Vec<Vec<Regex>>,
}

impl FilesWhitelist {
    fn new(files: &[String]) -> Self {
        static EXTENDED_GLOB: LazyLock<regex::Regex> = LazyLock::new(|| {
            regex::Regex::new(r"[{}]|[@+?!*]\(").expect("the extglob test compiles")
        });
        let mut whitelist = Self {
            include: NodeIgnore::default(),
            exclude: NodeIgnore::default(),
            include_globs: Vec::new(),
            exclude_globs: Vec::new(),
        };
        for file in files.iter().filter(|file| !file.is_empty()) {
            let negated = file.starts_with('!');
            let stripped = file.strip_prefix(['!', '/']).unwrap_or(file.as_str());
            let normalized = posix_normalize(stripped);
            let body = normalized.trim_end_matches('/');
            if EXTENDED_GLOB.is_match(body) {
                if negated {
                    whitelist.exclude_globs.push(files_matcher(body, true));
                } else {
                    whitelist.include_globs.push(files_matcher(body, false));
                }
            } else if negated {
                whitelist.exclude.add(body);
                whitelist.exclude.add(&format!("{body}/**"));
            } else {
                whitelist.include.add(&format!("/{body}"));
                whitelist.include.add(&format!("/{body}/**"));
            }
        }
        whitelist
    }

    fn leaves_out(&self, path: &str) -> bool {
        let included = self
            .include_globs
            .iter()
            .any(|matchers| any_match(matchers, path));
        let excluded = self
            .exclude_globs
            .iter()
            .any(|matchers| any_match(matchers, path));
        (!self.include.ignores(path) && !included) || self.exclude.ignores(path) || excluded
    }
}

/// eslint-plugin-n's `getNpmignore(...).match` for one package: which files below its folder npm
/// leaves out of the published package.
pub(crate) struct NpmPublication {
    files: Option<FilesWhitelist>,
    /// The `.npmignore`, else (without a `files` field) the `.gitignore`.
    npmignore: Option<NodeIgnore>,
    /// `main`, as `path.join` resolves it against the folder, relative to it; `None` without a
    /// string `main`, or for one that names the folder itself, a folder (a trailing slash) or a
    /// path outside it.
    main: Option<String>,
}

impl NpmPublication {
    /// The package in `folder` (relative to the workspace root) whose manifest holds `files` (its
    /// non-empty string entries, where the field is an array) and `main` (where a string), and
    /// whose folder holds a `.npmignore` and a `.gitignore` with these texts.
    pub(crate) fn new(
        folder: &str,
        files: Option<&[String]>,
        main: Option<&str>,
        npmignore: Option<&str>,
        gitignore: Option<&str>,
    ) -> Self {
        let ignore = match (npmignore, files) {
            (Some(text), _) => Some(text),
            (None, Some(_)) => None,
            (None, None) => gitignore,
        };
        Self {
            files: files.map(FilesWhitelist::new),
            npmignore: ignore.map(|text| {
                let mut ignore = NodeIgnore::default();
                ignore.add(text);
                ignore
            }),
            main: main
                .filter(|main| !main.ends_with('/'))
                .and_then(|main| resolve_target(folder, main.trim_start_matches('/')))
                .filter(|main| !main.is_empty()),
        }
    }

    /// `match(relative)` for a file's path relative to the package folder (`/`-separated, as
    /// `path.relative` gives it): whether npm leaves the file out. A path above the folder always
    /// is; without a `files` field, `.npmignore` or `.gitignore`, nothing else is. Else `main`,
    /// `package.json` and the names npm always publishes (a readme, licence, changelog or history
    /// file) never are: upstream tests those names on the path relative to the package folder as
    /// resolved from the working directory, so only when ESLint runs in the package folder, as
    /// it is read here; the rest is left out where the `files` whitelist or the ignore file
    /// leaves it out.
    pub(crate) fn leaves_out(&self, relative: &str) -> bool {
        static NEVER_IGNORED: LazyLock<regex::Regex> = LazyLock::new(|| {
            regex::Regex::new(
                r"(?i)^(?:readme\.[^.]*|(?:licen[cs]e|changes|changelog|history)(?:\.[^.]*)?)$",
            )
            .expect("NEVER_IGNORED compiles")
        });
        let ancestor = relative.starts_with("..");
        if self.files.is_none() && self.npmignore.is_none() {
            return ancestor;
        }
        let never_ignored = self.main.as_deref() == Some(posix_normalize(relative).as_str())
            || relative == "package.json"
            || NEVER_IGNORED.is_match(relative);
        !never_ignored
            && (ancestor
                || self
                    .files
                    .as_ref()
                    .is_some_and(|files| files.leaves_out(relative))
                || self
                    .npmignore
                    .as_ref()
                    .is_some_and(|ignore| ignore.ignores(relative)))
    }
}

#[cfg(test)]
mod tests;
