//! enhanced-resolve 5.26's synchronous resolver (`ResolverFactory.createResolver`), as
//! eslint-plugin-n's `ImportTarget` configures it: its hooks and plugins, in their order, with the
//! options the plugin sets (condition names, main fields and main files by import style, its try
//! extensions, the Node built-ins as a `false` fallback, and for TypeScript files the tsconfig
//! `paths` aliases and the backward extension map) and enhanced-resolve's defaults for the rest
//! (`package.json` description files, `node_modules`, `exports` and `imports` fields, symlinks
//! resolved).

use super::entrypoints;
use super::fs::View;
use super::path;
use super::json::Json;
use std::sync::{Arc, LazyLock};

/// `doResolve` steps one resolution may take; past them it fails.
const MAX_STEPS: usize = 20_000;

/// One alias: a name and what it maps to (`false`: the module is ignored).
#[derive(Clone, Debug)]
pub(crate) struct Alias {
    pub(crate) name: String,
    /// `None` is `false`; else the targets, tried in order.
    pub(crate) targets: Option<Vec<String>>,
}

/// The options a resolver is created with.
#[derive(Clone, Debug, Default)]
pub(crate) struct Options {
    pub(crate) conditions: Vec<&'static str>,
    pub(crate) main_fields: Vec<&'static str>,
    pub(crate) main_files: Vec<&'static str>,
    pub(crate) extensions: Vec<String>,
    pub(crate) alias: Vec<Alias>,
    pub(crate) extension_alias: Vec<(String, Vec<String>)>,
    pub(crate) fallback: Vec<Alias>,
}

/// What a resolution found: a path, or a module an alias maps to `false`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Resolved {
    Path(String),
    Ignored,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hook {
    Resolve,
    InternalResolve,
    ImportsResolve,
    ParsedResolve,
    DescribedResolve,
    RawResolve,
    NormalResolve,
    Internal,
    ImportsFieldRelative,
    RawModule,
    Module,
    ResolveAsModule,
    UndescribedResolveInPackage,
    ResolveInPackage,
    ResolveInExistingDirectory,
    Relative,
    DescribedRelative,
    Directory,
    UndescribedExistingDirectory,
    ExistingDirectory,
    UndescribedRawFile,
    RawFile,
    File,
    FinalFile,
    ExistingFile,
    Resolved,
}

/// A description file found for a request: its path, folder and content.
#[derive(Clone)]
struct Description {
    path: String,
    root: String,
    data: Arc<Json>,
}

#[derive(Clone, Default)]
struct Request {
    path: String,
    request: Option<String>,
    query: String,
    fragment: String,
    module: bool,
    directory: bool,
    internal: bool,
    fully_specified: bool,
    description: Option<Description>,
    relative_path: Option<String>,
    /// MainFieldPlugin's `alreadyTriedMainField`: the description file whose main it followed.
    tried_main: Option<String>,
}

/// A tap's answer: try the hook's next tap (`undefined`), stop the hook with no result (`null`),
/// or a result.
enum Tap {
    Next,
    Stop,
    Found(Found),
}

#[derive(Clone)]
enum Found {
    Request(Box<Request>),
    Ignored,
}

type Step = Result<Tap, String>;

/// `doResolve`'s callback over a target hook: the result, or `undefined`.
type Outcome = Result<Option<Found>, String>;

fn passthrough(outcome: Outcome) -> Step {
    outcome.map(|found| found.map_or(Tap::Next, Tap::Found))
}

/// The callback that turns `undefined` into `null` (`callback(null, null)`).
fn stopping(outcome: Outcome) -> Step {
    outcome.map(|found| found.map_or(Tap::Stop, Tap::Found))
}

/// enhanced-resolve's `getType`.
#[derive(PartialEq, Eq)]
enum PathType {
    Empty,
    Normal,
    Relative,
    Absolute,
    Internal,
}

fn path_type(path: &str) -> PathType {
    let bytes = path.as_bytes();
    match bytes {
        [] => PathType::Empty,
        [b'.'] | [b'.', b'.' | b'/'] | [b'.', b'/', _, ..] | [b'.', b'.', b'/', ..] => {
            PathType::Relative
        }
        [b'/', ..] => PathType::Absolute,
        [b'#', ..] => PathType::Internal,
        _ => PathType::Normal,
    }
}
/// `util/path.js`'s `join`: a relative result keeps its `./`.
fn join(base: &str, request: &str) -> String {
    if request.is_empty() {
        return normalize(base);
    }
    if request.starts_with('/') {
        return path::normalize(request);
    }
    path::normalize(&format!("{base}/{request}"))
}

fn normalize(path_: &str) -> String {
    match path_type(path_) {
        PathType::Empty => path_.to_owned(),
        PathType::Relative => {
            let normalized = path::normalize(path_);
            if path_type(&normalized) == PathType::Relative {
                normalized
            } else {
                format!("./{normalized}")
            }
        }
        _ => path::normalize(path_),
    }
}

/// `parseIdentifier`: the path, its query (from `?`) and fragment (from a `#` past the first
/// character).
fn parse_identifier(identifier: &str) -> (String, String, String) {
    let path_part = entrypoints::identifier_path(identifier);
    let rest = &identifier[path_part.len()..];
    let (query, fragment) = match rest.find('#') {
        Some(at) if rest.starts_with('?') => (&rest[..at], &rest[at..]),
        _ if rest.starts_with('?') => (rest, ""),
        _ => ("", rest),
    };
    (path_part.to_owned(), query.to_owned(), fragment.to_owned())
}

/// `cdUp`.
fn cd_up(directory: &str) -> Option<&str> {
    if directory.len() <= 1 {
        return None;
    }
    let at = directory.rfind('/')?;
    Some(if at == 0 { "/" } else { &directory[..at] })
}

/// The invalid segments of an `exports` target (`deprecatedInvalidSegmentRegEx`): `.`, `..` or
/// `node_modules`, percent-encoded or not, in any case.
static INVALID_SEGMENT: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r"(?i)(^|\\|/)((\.|%2e)(\.|%2e)?|(n|%6e|%4e)(o|%6f|%4f)(d|%64|%44)(e|%65|%45)(_|%5f)(m|%6d|%4d)(o|%6f|%4f)(d|%64|%44)(u|%75|%55)(l|%6c|%4c)(e|%65|%45)(s|%73|%53))(\\|/|$)",
    )
    .expect("the invalid segment pattern compiles")
});
/// `invalidSegmentRegEx`: the same, or an empty segment.
static INVALID_OR_EMPTY_SEGMENT: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r"(?i)(^|\\|/)((\.|%2e)(\.|%2e)?|(n|%6e|%4e)(o|%6f|%4f)(d|%64|%44)(e|%65|%45)(_|%5f)(m|%6d|%4d)(o|%6f|%4f)(d|%64|%44)(u|%75|%55)(l|%6c|%4c)(e|%65|%45)(s|%73|%53))?(\\|/|$)",
    )
    .expect("the invalid segment pattern compiles")
});

struct Resolver<'v> {
    view: &'v View,
    options: &'v Options,
    stack: Vec<String>,
    steps: usize,
}

/// Resolves `specifier` from the folder `base` (absolute): a path, `Ignored`, or the error
/// enhanced-resolve's `resolveSync` throws (its message).
pub(crate) fn resolve(
    view: &View,
    options: &Options,
    base: &str,
    specifier: &str,
) -> Result<Resolved, String> {
    let mut resolver = Resolver {
        view,
        options,
        stack: Vec::new(),
        steps: 0,
    };
    let request = Request {
        path: base.to_owned(),
        request: Some(specifier.to_owned()),
        ..Request::default()
    };
    match resolver.resolve_hook(Hook::Resolve, request)? {
        Some(Found::Request(found)) => Ok(Resolved::Path(format!(
            "{}{}{}",
            found.path, found.query, found.fragment
        ))),
        Some(Found::Ignored) => Ok(Resolved::Ignored),
        None => Err(format!("Can't resolve '{specifier}' in '{base}'")),
    }
}

impl Resolver<'_> {
    /// `doResolve`: runs `hook`'s taps on `request`, refusing a step it is already inside.
    fn resolve_hook(&mut self, hook: Hook, request: Request) -> Outcome {
        self.steps += 1;
        if self.steps > MAX_STEPS {
            return Err("resolution took too many steps".to_owned());
        }
        let entry = format!(
            "{hook:?}: ({}) {}{}{}{}{}",
            request.path,
            request.request.as_deref().unwrap_or(""),
            request.query,
            request.fragment,
            if request.directory { " directory" } else { "" },
            if request.module { " module" } else { "" },
        );
        if self.stack.contains(&entry) {
            return Err(format!(
                "Recursion in resolving\nStack:\n  {}",
                self.stack.join("\n  ")
            ));
        }
        self.stack.push(entry);
        let outcome = self.taps(hook, request);
        self.stack.pop();
        match outcome? {
            Tap::Found(found) => Ok(Some(found)),
            Tap::Next | Tap::Stop => Ok(None),
        }
    }

    /// The hook's taps in order, until one answers anything but `Next`.
    fn taps(&mut self, hook: Hook, request: Request) -> Step {
        let taps: &[fn(&mut Self, Hook, &Request) -> Step] = match hook {
            Hook::Resolve | Hook::InternalResolve | Hook::ImportsResolve => &[Self::parse],
            Hook::ParsedResolve => &[Self::describe_directory, Self::next],
            Hook::DescribedResolve => &[Self::next, Self::fallback],
            Hook::RawResolve => &[Self::alias, Self::extension_alias, Self::next],
            Hook::NormalResolve => &[Self::if_module, Self::if_internal, Self::join_request],
            Hook::Internal => &[Self::imports_field],
            Hook::ImportsFieldRelative => &[Self::extension_alias, Self::next],
            Hook::RawModule => &[Self::self_reference, Self::modules],
            Hook::Module => &[Self::join_request_part],
            Hook::ResolveAsModule => &[Self::single_file_module, Self::directory_exists],
            Hook::UndescribedResolveInPackage => &[Self::describe_directory, Self::next],
            Hook::ResolveInPackage => &[Self::exports_field, Self::next],
            Hook::ResolveInExistingDirectory => &[Self::join_request],
            Hook::Relative => &[Self::describe_file, Self::next],
            Hook::DescribedRelative => &[Self::if_not_directory, Self::as_directory],
            Hook::Directory => &[Self::directory_exists],
            Hook::UndescribedExistingDirectory => &[Self::describe_directory, Self::use_files],
            Hook::ExistingDirectory => &[Self::main_fields, Self::use_files],
            Hook::UndescribedRawFile => &[Self::describe_file, Self::next],
            Hook::RawFile => &[Self::if_fully_specified, Self::no_extension, Self::append],
            Hook::File => &[Self::alias, Self::next],
            Hook::FinalFile => &[Self::file_exists],
            Hook::ExistingFile => &[Self::symlink, Self::next],
            Hook::Resolved => &[Self::result],
        };
        for tap in taps {
            match tap(self, hook, &request)? {
                Tap::Next => {}
                answer => return Ok(answer),
            }
        }
        Ok(Tap::Next)
    }

    /// `NextPlugin`: the hook each forwards to.
    fn next(&mut self, hook: Hook, request: &Request) -> Step {
        let target = match hook {
            Hook::ParsedResolve => Hook::DescribedResolve,
            Hook::DescribedResolve => Hook::RawResolve,
            Hook::RawResolve => Hook::NormalResolve,
            Hook::ImportsFieldRelative => Hook::Relative,
            Hook::UndescribedResolveInPackage => Hook::ResolveInPackage,
            Hook::ResolveInPackage => Hook::ResolveInExistingDirectory,
            Hook::Relative => Hook::DescribedRelative,
            Hook::UndescribedRawFile => Hook::RawFile,
            Hook::File => Hook::FinalFile,
            Hook::ExistingFile => Hook::Resolved,
            _ => unreachable!("{hook:?} has no NextPlugin"),
        };
        passthrough(self.resolve_hook(target, request.clone()))
    }

    /// `ParsePlugin`.
    fn parse(&mut self, hook: Hook, request: &Request) -> Step {
        let identifier = request.request.as_deref().unwrap_or("");
        let mut parsed = request.clone();
        let (mut path_part, query, fragment) = if identifier.is_empty() {
            (String::new(), String::new(), String::new())
        } else {
            parse_identifier(identifier)
        };
        parsed.module = false;
        parsed.directory = false;
        parsed.internal = false;
        if !path_part.is_empty() {
            let kind = path_type(&path_part);
            parsed.module = kind == PathType::Normal;
            parsed.internal = kind == PathType::Internal;
            parsed.directory = path_part.ends_with('/');
            if parsed.directory {
                path_part.pop();
            }
        }
        parsed.request = Some(path_part);
        if !query.is_empty() {
            parsed.query = query;
        }
        if !fragment.is_empty() {
            parsed.fragment = fragment;
        }
        parsed.fully_specified = false;
        if hook == Hook::ImportsResolve {
            parsed.internal = false;
        }
        let has_request = parsed.request.as_deref().is_some_and(|request| !request.is_empty());
        if has_request && parsed.query.is_empty() && !parsed.fragment.is_empty() {
            let directory = parsed.fragment.ends_with('/');
            let fragment = if directory {
                &parsed.fragment[..parsed.fragment.len() - 1]
            } else {
                parsed.fragment.as_str()
            };
            let alternative = Request {
                directory,
                request: Some(format!(
                    "{}{}{fragment}",
                    parsed.request.as_deref().unwrap_or(""),
                    if parsed.directory { "/" } else { "" }
                )),
                fragment: String::new(),
                ..parsed.clone()
            };
            if let Some(found) = self.resolve_hook(Hook::ParsedResolve, alternative)? {
                return Ok(Tap::Found(found));
            }
        }
        passthrough(self.resolve_hook(Hook::ParsedResolve, parsed))
    }

    /// `loadDescriptionFile`: the nearest `package.json` at or above `directory`.
    fn description(&self, mut directory: &str) -> Result<Option<Description>, String> {
        loop {
            let file = join(directory, "package.json");
            match self.view.json(&file) {
                Some(Ok(data)) => {
                    return Ok(Some(Description {
                        path: file,
                        root: directory.to_owned(),
                        data,
                    }));
                }
                Some(Err(error)) => {
                    return Err(format!(
                        "{file} (directory description file): SyntaxError: {error}"
                    ));
                }
                None => {}
            }
            match cd_up(directory) {
                Some(up) => directory = up,
                None => return Ok(None),
            }
        }
    }

    /// `DescriptionFilePlugin`, from the request's path (a folder, or a file's folder).
    fn describe(&mut self, hook: Hook, request: &Request, path_is_file: bool) -> Step {
        if request.path.is_empty() {
            return Ok(Tap::Next);
        }
        let directory = if path_is_file {
            match cd_up(&request.path) {
                Some(directory) => directory.to_owned(),
                None => return Ok(Tap::Next),
            }
        } else {
            request.path.clone()
        };
        let Some(description) = self.description(&directory)? else {
            return Ok(Tap::Next);
        };
        let relative = format!(".{}", &request.path[description.root.len().min(request.path.len())..]);
        let target = match hook {
            Hook::ParsedResolve => Hook::DescribedResolve,
            Hook::UndescribedResolveInPackage => Hook::ResolveInPackage,
            Hook::Relative => Hook::DescribedRelative,
            Hook::UndescribedExistingDirectory => Hook::ExistingDirectory,
            Hook::UndescribedRawFile => Hook::RawFile,
            _ => unreachable!("{hook:?} has no DescriptionFilePlugin"),
        };
        let described = Request {
            description: Some(description),
            relative_path: Some(relative),
            ..request.clone()
        };
        stopping(self.resolve_hook(target, described))
    }

    fn describe_directory(&mut self, hook: Hook, request: &Request) -> Step {
        self.describe(hook, request, false)
    }

    fn describe_file(&mut self, hook: Hook, request: &Request) -> Step {
        self.describe(hook, request, true)
    }

    /// `AliasPlugin` over `aliases` (`aliasResolveHandler`), forwarding to internal-resolve.
    fn alias_with(&mut self, aliases: &[Alias], request: &Request) -> Step {
        let inner = match request.request.as_deref() {
            Some(inner) if !inner.is_empty() => inner.to_owned(),
            _ => request.path.clone(),
        };
        if inner.is_empty() {
            return Ok(Tap::Next);
        }
        let has_request = request.request.as_deref().is_some_and(|inner| !inner.is_empty());
        for item in aliases {
            let star = item.name.find('*').filter(|&at| !item.name[at + 1..].contains('*'));
            let wildcard = star.map(|at| (&item.name[..at], &item.name[at + 1..]));
            let first = match wildcard {
                Some(("", _)) => None,
                _ => item.name.chars().next(),
            };
            if first.is_some_and(|first| !inner.starts_with(first)) {
                continue;
            }
            let absolute = item
                .name
                .starts_with('/')
                .then(|| format!("{}/", path::normalize(&item.name).trim_end_matches('/')));
            let matches_request = inner == item.name
                || (has_request && inner.starts_with(&format!("{}/", item.name)))
                || absolute
                    .as_ref()
                    .is_some_and(|absolute| inner.starts_with(absolute.as_str()));
            if !matches_request && wildcard.is_none() {
                continue;
            }
            let Some(targets) = &item.targets else {
                return Ok(Tap::Found(Found::Ignored));
            };
            let mut should_stop = false;
            for alias in targets {
                let mut rewritten = None;
                if let Some((prefix, suffix)) = wildcard
                    && inner.starts_with(prefix)
                    && inner.ends_with(suffix)
                    && inner.len() >= prefix.len() + suffix.len()
                {
                    let matched = &inner[prefix.len()..inner.len() - suffix.len()];
                    rewritten = Some(alias.replacen('*', matched, 1));
                }
                if matches_request && inner != *alias && !inner.starts_with(&format!("{alias}/")) {
                    rewritten = Some(format!("{alias}{}", &inner[item.name.len()..]));
                }
                let Some(rewritten) = rewritten else {
                    continue;
                };
                should_stop = true;
                let aliased = Request {
                    request: Some(rewritten),
                    fully_specified: false,
                    ..request.clone()
                };
                if let Some(found) = self.resolve_hook(Hook::InternalResolve, aliased)? {
                    return Ok(Tap::Found(found));
                }
            }
            if should_stop {
                return Ok(Tap::Stop);
            }
        }
        Ok(Tap::Next)
    }

    fn alias(&mut self, _hook: Hook, request: &Request) -> Step {
        let options = self.options;
        if options.alias.is_empty() {
            return Ok(Tap::Next);
        }
        self.alias_with(&options.alias, request)
    }

    fn fallback(&mut self, _hook: Hook, request: &Request) -> Step {
        let options = self.options;
        if options.fallback.is_empty() {
            return Ok(Tap::Next);
        }
        self.alias_with(&options.fallback, request)
    }

    /// `ExtensionAliasPlugin`, one tap per extension in order.
    fn extension_alias(&mut self, hook: Hook, request: &Request) -> Step {
        let target = if hook == Hook::RawResolve {
            Hook::NormalResolve
        } else {
            Hook::Relative
        };
        for (extension, aliases) in &self.options.extension_alias {
            let use_request = request.request.is_some();
            let source = if use_request {
                request.request.clone().unwrap_or_default()
            } else {
                request.path.clone()
            };
            if source.is_empty() || !source.ends_with(extension.as_str()) {
                continue;
            }
            let base = &source[..source.len() - extension.len()];
            let relative_base = (!use_request)
                .then(|| {
                    request
                        .relative_path
                        .as_deref()
                        .and_then(|relative| relative.strip_suffix(extension.as_str()))
                })
                .flatten();
            let mut found = None;
            for (index, alias) in aliases.iter().enumerate() {
                let value = format!("{base}{alias}");
                let next = if use_request {
                    Request {
                        request: Some(value),
                        fully_specified: true,
                        ..request.clone()
                    }
                } else {
                    Request {
                        path: value,
                        relative_path: match relative_base {
                            Some(relative) => Some(format!("{relative}{alias}")),
                            None => request.relative_path.clone(),
                        },
                        fully_specified: true,
                        ..request.clone()
                    }
                };
                match self.resolve_hook(target, next) {
                    Ok(Some(answer)) => {
                        found = Some(answer);
                        break;
                    }
                    Ok(None) => {}
                    // Only the last alias's error stands where there are several.
                    Err(_) if aliases.len() > 1 && index + 1 < aliases.len() => {}
                    Err(error) => return Err(error),
                }
            }
            if let Some(found) = found {
                return Ok(Tap::Found(found));
            }
            if aliases.iter().any(|alias| alias == extension) {
                continue;
            }
            return Ok(Tap::Stop);
        }
        Ok(Tap::Next)
    }

    /// `ConditionalPlugin({ module: true })` to raw-module, alternatives refused.
    fn if_module(&mut self, _hook: Hook, request: &Request) -> Step {
        if !request.module {
            return Ok(Tap::Next);
        }
        stopping(self.resolve_hook(Hook::RawModule, request.clone()))
    }

    /// `ConditionalPlugin({ internal: true })` to internal, alternatives refused.
    fn if_internal(&mut self, _hook: Hook, request: &Request) -> Step {
        if !request.internal {
            return Ok(Tap::Next);
        }
        stopping(self.resolve_hook(Hook::Internal, request.clone()))
    }

    /// `JoinRequestPlugin` to relative.
    fn join_request(&mut self, _hook: Hook, request: &Request) -> Step {
        let inner = request.request.as_deref().unwrap_or("");
        let joined = Request {
            path: join(&request.path, inner),
            relative_path: request
                .relative_path
                .as_deref()
                .map(|relative| join(relative, inner)),
            request: None,
            ..request.clone()
        };
        passthrough(self.resolve_hook(Hook::Relative, joined))
    }

    /// `ImportsFieldPlugin`.
    fn imports_field(&mut self, _hook: Hook, request: &Request) -> Step {
        let (Some(description), Some(inner)) = (&request.description, &request.request) else {
            return Ok(Tap::Next);
        };
        let remaining = format!("{inner}{}{}", request.query, request.fragment);
        let Some(field) = description.data.get("imports").filter(|field| field.truthy()) else {
            return Ok(Tap::Next);
        };
        if request.directory {
            return Err(format!(
                "Resolving to directories is not possible with the imports field (request was {remaining}/)"
            ));
        }
        let (paths, used) = entrypoints::imports(field, &remaining, &self.options.conditions)
            .map_err(|error| format!("{error} (see imports field in {})", description.path))?;
        if paths.is_empty() {
            return Err(format!(
                "Package import {remaining} is not imported from package {} (see imports field in {})",
                description.root, description.path
            ));
        }
        let count = paths.len();
        for (index, target) in paths.iter().enumerate() {
            let (path_part, query, fragment) = parse_identifier(target);
            let last = index + 1 == count;
            let outcome = if path_part.starts_with('.') {
                let without = path_part.get(2..).unwrap_or("");
                if INVALID_OR_EMPTY_SEGMENT.is_match(without) {
                    if last {
                        return Err(invalid_target("imports", target, used.as_deref(), &description.path));
                    }
                    continue;
                }
                self.resolve_hook(
                    Hook::ImportsFieldRelative,
                    Request {
                        request: None,
                        path: join(&description.root, &path_part),
                        relative_path: Some(path_part),
                        query,
                        fragment,
                        ..request.clone()
                    },
                )?
            } else {
                self.resolve_hook(
                    Hook::ImportsResolve,
                    Request {
                        request: Some(path_part.clone()),
                        relative_path: Some(path_part),
                        fully_specified: true,
                        query,
                        fragment,
                        ..request.clone()
                    },
                )?
            };
            return Ok(outcome.map_or(Tap::Stop, Tap::Found));
        }
        Ok(Tap::Stop)
    }

    /// `SelfReferencePlugin`: a package naming itself, through its `exports`.
    fn self_reference(&mut self, _hook: Hook, request: &Request) -> Step {
        let (Some(description), Some(inner)) = (&request.description, &request.request) else {
            return Ok(Tap::Next);
        };
        if inner.is_empty() || !description.data.get("exports").is_some_and(Json::truthy) {
            return Ok(Tap::Next);
        }
        let Some(name) = description.data.get("name").and_then(Json::as_str) else {
            return Ok(Tap::Next);
        };
        let Some(rest) = inner.strip_prefix(name) else {
            return Ok(Tap::Next);
        };
        if !rest.is_empty() && !rest.starts_with('/') {
            return Ok(Tap::Next);
        }
        let referenced = Request {
            request: Some(format!(".{rest}")),
            path: description.root.clone(),
            relative_path: Some(".".to_owned()),
            ..request.clone()
        };
        passthrough(self.resolve_hook(Hook::ResolveAsModule, referenced))
    }

    /// `ModulesInHierarchicalDirectoriesPlugin`: `node_modules` in the request's folder and each
    /// one above it.
    fn modules(&mut self, _hook: Hook, request: &Request) -> Step {
        let inner = request.request.clone().unwrap_or_default();
        let mut directory = request.path.clone();
        loop {
            let folder = join(&directory, "node_modules");
            if self.view.is_directory(&folder) {
                let found = self.resolve_hook(
                    Hook::Module,
                    Request {
                        path: folder,
                        request: Some(format!("./{inner}")),
                        module: false,
                        ..request.clone()
                    },
                )?;
                if let Some(found) = found {
                    return Ok(Tap::Found(found));
                }
            }
            match cd_up(&directory) {
                Some(up) => directory = up.to_owned(),
                None => return Ok(Tap::Next),
            }
        }
    }

    /// `JoinRequestPartPlugin`: the module's folder, and the request within it.
    fn join_request_part(&mut self, _hook: Hook, request: &Request) -> Step {
        let inner = request.request.as_deref().unwrap_or("");
        let mut split = inner.get(3..).and_then(|rest| rest.find('/')).map(|at| at + 3);
        if let Some(at) = split
            && inner.as_bytes().get(2) == Some(&b'@')
        {
            split = inner[at + 1..].find('/').map(|next| at + 1 + next);
        }
        let (module, remaining, fully_specified) = match split {
            None => (inner, ".".to_owned(), false),
            Some(at) => (&inner[..at], format!(".{}", &inner[at..]), request.fully_specified),
        };
        let joined = Request {
            path: join(&request.path, module),
            relative_path: request
                .relative_path
                .as_deref()
                .map(|relative| join(relative, module)),
            request: Some(remaining),
            fully_specified,
            ..request.clone()
        };
        passthrough(self.resolve_hook(Hook::ResolveAsModule, joined))
    }

    /// `ConditionalPlugin({ directory: false, request: "." })`: a module that is a single file.
    fn single_file_module(&mut self, _hook: Hook, request: &Request) -> Step {
        if request.directory || request.request.as_deref() != Some(".") {
            return Ok(Tap::Next);
        }
        passthrough(self.resolve_hook(Hook::UndescribedRawFile, request.clone()))
    }

    /// `DirectoryExistsPlugin`.
    fn directory_exists(&mut self, hook: Hook, request: &Request) -> Step {
        if request.path.is_empty() || !self.view.is_directory(&request.path) {
            return Ok(Tap::Next);
        }
        let target = match hook {
            Hook::ResolveAsModule => Hook::UndescribedResolveInPackage,
            _ => Hook::UndescribedExistingDirectory,
        };
        passthrough(self.resolve_hook(target, request.clone()))
    }

    /// `ExportsFieldPlugin`.
    fn exports_field(&mut self, _hook: Hook, request: &Request) -> Step {
        let Some(description) = &request.description else {
            return Ok(Tap::Next);
        };
        let Some(inner) = &request.request else {
            return Ok(Tap::Next);
        };
        if request.relative_path.as_deref() != Some(".") {
            return Ok(Tap::Next);
        }
        let remaining = if request.query.is_empty() && request.fragment.is_empty() {
            inner.clone()
        } else {
            format!(
                "{}{}{}",
                if inner == "." { "./" } else { inner },
                request.query,
                request.fragment
            )
        };
        let Some(field) = description.data.get("exports").filter(|field| field.truthy()) else {
            return Ok(Tap::Next);
        };
        if request.directory {
            return Err(format!(
                "Resolving to directories is not possible with the exports field (request was {remaining}/)"
            ));
        }
        let (paths, used) = entrypoints::exports(field, &remaining, &self.options.conditions)
            .map_err(|error| format!("{error} (see exports field in {})", description.path))?;
        if paths.is_empty() {
            let conditions = &self.options.conditions;
            let conditions = if conditions.len() == 1 {
                format!("the condition \"{}\"", conditions[0])
            } else {
                format!(
                    "the conditions {}",
                    serde_json::to_string(conditions).unwrap_or_default()
                )
            };
            return Err(format!(
                "\"{remaining}\" is not exported under {conditions} from package {} (see exports field in {})",
                description.root, description.path
            ));
        }
        let count = paths.len();
        for (index, target) in paths.iter().enumerate() {
            let (path_part, query, fragment) = parse_identifier(target);
            let last = index + 1 == count;
            let valid = path_part.starts_with("./")
                && !(INVALID_OR_EMPTY_SEGMENT.is_match(&path_part[2..])
                    && INVALID_SEGMENT.is_match(&path_part[2..]));
            if !valid {
                if last {
                    return Err(invalid_target("exports", target, used.as_deref(), &description.path));
                }
                continue;
            }
            let found = self.resolve_hook(
                Hook::Relative,
                Request {
                    request: None,
                    path: join(&description.root, &path_part),
                    relative_path: Some(path_part),
                    query,
                    fragment,
                    ..request.clone()
                },
            )?;
            return match found {
                Some(found) => Ok(Tap::Found(found)),
                None => Err(format!(
                    "Package path {remaining} is exported from package {}, but no valid target file was found (see exports field in {})",
                    description.root, description.path
                )),
            };
        }
        Err(format!(
            "Package path {remaining} is exported from package {}, but no valid target file was found (see exports field in {})",
            description.root, description.path
        ))
    }

    /// `ConditionalPlugin({ directory: false })` to raw-file, alternatives allowed.
    fn if_not_directory(&mut self, _hook: Hook, request: &Request) -> Step {
        if request.directory {
            return Ok(Tap::Next);
        }
        passthrough(self.resolve_hook(Hook::RawFile, request.clone()))
    }

    /// `ConditionalPlugin({ fullySpecified: false })` to directory, alternatives allowed.
    fn as_directory(&mut self, _hook: Hook, request: &Request) -> Step {
        if request.fully_specified {
            return Ok(Tap::Next);
        }
        passthrough(self.resolve_hook(Hook::Directory, request.clone()))
    }

    /// `MainFieldPlugin`, one tap per field.
    fn main_fields(&mut self, _hook: Hook, request: &Request) -> Step {
        let Some(description) = &request.description else {
            return Ok(Tap::Next);
        };
        if request.path != description.root
            || request.tried_main.as_deref() == Some(description.path.as_str())
        {
            return Ok(Tap::Next);
        }
        for field in &self.options.main_fields {
            let Some(raw) = description.data.get(*field).and_then(Json::as_str) else {
                continue;
            };
            if raw.is_empty() || raw == "." || raw == "./" {
                continue;
            }
            let main = if raw.starts_with("./") || raw.starts_with("../") {
                raw.to_owned()
            } else {
                format!("./{raw}")
            };
            let found = self.resolve_hook(
                Hook::ResolveInExistingDirectory,
                Request {
                    directory: main.ends_with('/'),
                    request: Some(main),
                    module: false,
                    tried_main: Some(description.path.clone()),
                    ..request.clone()
                },
            )?;
            if let Some(found) = found {
                return Ok(Tap::Found(found));
            }
        }
        Ok(Tap::Next)
    }

    /// `UseFilePlugin`, one tap per main file.
    fn use_files(&mut self, _hook: Hook, request: &Request) -> Step {
        for file in &self.options.main_files {
            let found = self.resolve_hook(
                Hook::UndescribedRawFile,
                Request {
                    path: join(&request.path, file),
                    relative_path: request
                        .relative_path
                        .as_deref()
                        .map(|relative| join(relative, file)),
                    ..request.clone()
                },
            )?;
            if let Some(found) = found {
                return Ok(Tap::Found(found));
            }
        }
        Ok(Tap::Next)
    }

    /// `ConditionalPlugin({ fullySpecified: true })` to file, alternatives refused.
    fn if_fully_specified(&mut self, _hook: Hook, request: &Request) -> Step {
        if !request.fully_specified {
            return Ok(Tap::Next);
        }
        stopping(self.resolve_hook(Hook::File, request.clone()))
    }

    /// `TryNextPlugin("no extension")`.
    fn no_extension(&mut self, _hook: Hook, request: &Request) -> Step {
        passthrough(self.resolve_hook(Hook::File, request.clone()))
    }

    /// `AppendPlugin`, one tap per extension.
    fn append(&mut self, _hook: Hook, request: &Request) -> Step {
        for extension in &self.options.extensions {
            let found = self.resolve_hook(
                Hook::File,
                Request {
                    path: format!("{}{extension}", request.path),
                    relative_path: request
                        .relative_path
                        .as_deref()
                        .map(|relative| format!("{relative}{extension}")),
                    ..request.clone()
                },
            )?;
            if let Some(found) = found {
                return Ok(Tap::Found(found));
            }
        }
        Ok(Tap::Next)
    }

    /// `FileExistsPlugin`.
    fn file_exists(&mut self, _hook: Hook, request: &Request) -> Step {
        if request.path.is_empty() || !self.view.is_file(&request.path) {
            return Ok(Tap::Next);
        }
        passthrough(self.resolve_hook(Hook::ExistingFile, request.clone()))
    }

    /// `SymlinkPlugin`: the file's path with its symlinks resolved.
    fn symlink(&mut self, _hook: Hook, request: &Request) -> Step {
        let real = self.view.real_path(&request.path);
        if real == request.path {
            return Ok(Tap::Next);
        }
        stopping(self.resolve_hook(
            Hook::ExistingFile,
            Request {
                path: real,
                ..request.clone()
            },
        ))
    }

    /// `ResultPlugin`.
    fn result(&mut self, _hook: Hook, request: &Request) -> Step {
        Ok(Tap::Found(Found::Request(Box::new(request.clone()))))
    }
}

fn invalid_target(field: &str, target: &str, used: Option<&str>, description: &str) -> String {
    format!(
        "Invalid \"{field}\" target \"{target}\" defined for \"{}\" in the package config {description}, targets must start with \"./\"",
        used.unwrap_or("")
    )
}

/// The description file `json` of a package folder, where it is an object.
pub(crate) fn package_json(view: &View, folder: &str) -> Option<Arc<Json>> {
    match view.json(&join(folder, "package.json")) {
        Some(Ok(data)) if data.entries().is_some() => Some(data),
        _ => None,
    }
}
