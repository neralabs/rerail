//! `layout` (rerail extension): the node's lines and first byte, and the lines between it and a
//! neighbour, ported from the native engine over per-file facts.
//!
//! Ported from (native = `native/src/code_rule_dsl`):
//! - `compile/layout.rs:11-42` (`line_count`), `:45-97` (`gap`), `:99-184` (`layout`),
//!   `:217-221` (`compile_layout`): every shape and every shape error, as `Err("layout: ...")`.
//! - `compile/parse.rs:310-319` (`capture_name`): a gap's `capture.NAME`.
//! - `matcher/layout.rs:13-24` (`LineCount`), `:26-70` (types), `:72-91` (`reads`, `outward`),
//!   `:93-164` (`Layout::match_node`), `:173-229` (`Gap::holds`), `:233-276` (`blank_lines`),
//!   `:279-281` (`is_comment`), `:285-303` (`token`), `:306-335` (`leaf`), `:338-344` (`sibling`).
//! - `matcher/util.rs:37-42` (`is_comment_kind`).
//! - `matcher/each.rs:82-105` (`binding_range`): a capture's range is its node's, or a parsed
//!   literal's exported span (`Eval::range_of`, the range `enumerate`/`excluded` read too). A
//!   Python file's tape has no host (`tape.rs:2333-2375`), so `InFile::range`/`node` are identity.
//! - `lines.rs:54-83,102-105,134-142` (`LineTable`): the file's lines are the tape's
//!   (`Facts::lines`, Ruff's `\r\n`/`\r`/`\n` for Python); `line(byte)` holds a terminator's bytes
//!   on the line they end.
//! - `tape.rs:2110-2145` (`next_sibling`/`prev_sibling`/`walk_anchor`) and `:2197-2228`
//!   (`SgNode::next`, `prev`, `next_all`, `prev_all`): the navigation `leaf` and `sibling` use.
//!   `next` skips later siblings ending at or before this node's end; `next_all`/`prev_all` start
//!   at the walk anchor (for the root: its first child). Facts ids are the tape's preorder ids, and
//!   a parent's children list is in sibling order.
//! - `matcher/captures/semantic.rs:62,142`: a layout binds nothing and reads its gaps' captures.
//! - `compile/ast_rule.rs:848-862`: the layout term runs after `count` and `compare`, before
//!   `value` (the integrator's term order).
//!
//! Verdicts are two-valued: native layout never makes a verdict unknown. A gap to a capture that is
//! unbound, or that does not lie wholly on its side of the node, has no neighbour, and a capture gap
//! cannot take `edge`, so it fails (plain false).
//!
//! Refusals:
//! - Evaluation: `layout` holding only `start` on a node of a parsed literal refuses ("layout:
//!   start on a node of a parsed literal"): native reads the literal tree's own byte offsets there
//!   (`node.range()`), which the facts' parsed nodes do not carry. With `span` or an outward key on
//!   such a node native fails it whatever `start` says (`InFile::of` is `None` for a parsed tree),
//!   which is ported exactly.
//! - Not ported here, as for every other key of `ast.rs`: native's whole-document validation that
//!   this module cannot see (`compile/layout.rs:186-215` `maxRegexPatternBytes`;
//!   `compile/compare.rs:836-852` "never bound where this layout runs", `$$$` list and transform
//!   variable `to` captures) and the utility step budget (`spend_util_step`, `matcher/mod.rs:774`).
//!   Policies reach the spike only after native compiled them.
use super::{Bound, Compiler, Env, Eval, Name, Rule};
use crate::facts::{self, Facts};
use regex::Regex;
use rerail_engine::code_rule_dsl::lines::LineTable;
use serde_json::Value as Json;

/// A count's bounds, inclusive; `eq` is `min == max`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineCount {
    min: u64,
    max: Option<u64>,
}

impl LineCount {
    fn holds(self, count: u64) -> bool {
        count >= self.min && self.max.is_none_or(|max| count <= max)
    }
}

/// The node a gap runs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Neighbour {
    /// The nearest token (a leaf with a width, anonymous ones included).
    Token,
    /// The nearest named sibling.
    Sibling,
    /// A single-node capture the rule bound before.
    Capture(Name),
}

/// `before` or `after`: the lines between the node and its neighbour on that side.
#[derive(Clone, Debug)]
pub struct Gap {
    to: Neighbour,
    /// Whether a comment counts as the neighbouring token or sibling (else it is passed over).
    comments: bool,
    /// Whether the file's start (`before`) or end (`after`) stands in for a missing neighbour.
    edge: bool,
    /// Line breaks between the neighbour and the node.
    breaks: Option<LineCount>,
    /// Lines between them that hold no token or comment.
    blank: Option<LineCount>,
}

#[derive(Clone, Debug)]
pub struct LineContext {
    before: usize,
    after: usize,
    through: Option<Regex>,
    regex: Regex,
}

/// A compiled `layout`.
#[derive(Clone, Debug)]
pub struct Layout {
    /// Line breaks inside the node.
    span: Option<LineCount>,
    /// The node's first byte.
    start: Option<LineCount>,
    before: Option<Gap>,
    after: Option<Gap>,
    line: Option<Regex>,
    previous_line: Option<Regex>,
    context: Option<LineContext>,
}

impl Layout {
    /// Every nested rule: a layout holds none.
    pub(super) fn sub_rules(&self) -> Vec<&Rule> {
        Vec::new()
    }

    /// The captures the layout binds: none.
    pub(super) fn binds(&self) -> Vec<Name> {
        Vec::new()
    }

    /// The captures the gaps read (`Layout::reads`).
    pub(super) fn reads(&self) -> Vec<Name> {
        [&self.before, &self.after]
            .into_iter()
            .flatten()
            .filter_map(|gap| match gap.to {
                Neighbour::Capture(name) => Some(name),
                _ => None,
            })
            .collect()
    }

    /// Whether the layout reads outside the node: a gap, or the lines around it.
    fn outward(&self) -> bool {
        self.before.is_some()
            || self.after.is_some()
            || self.line.is_some()
            || self.previous_line.is_some()
            || self.context.is_some()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Before,
    After,
}

// --- compile (compile/layout.rs) ---

fn object<'j>(value: &'j Json, path: &str) -> Result<&'j serde_json::Map<String, Json>, String> {
    value
        .as_object()
        .ok_or_else(|| format!("layout: {path} must be an object"))
}

fn reject_unknown(
    map: &serde_json::Map<String, Json>,
    allowed: &[&str],
    path: &str,
) -> Result<(), String> {
    match map.keys().find(|key| !allowed.contains(&key.as_str())) {
        Some(key) => Err(format!("layout: unknown field `{key}` at {path}")),
        None => Ok(()),
    }
}

fn regex(pattern: &str, path: &str) -> Result<Regex, String> {
    crate::rx::get(pattern).map_err(|error| format!("layout: {path}: {error}"))
}

/// A count of lines or bytes: `{ eq }` or `{ min, max }`, non-negative integers.
fn line_count(value: &Json, path: &str) -> Result<LineCount, String> {
    let bounds = object(value, path)?;
    reject_unknown(bounds, &["eq", "min", "max"], path)?;
    let number = |key: &str| -> Result<Option<u64>, String> {
        bounds
            .get(key)
            .map(|value| {
                value
                    .as_u64()
                    .ok_or_else(|| format!("layout: {path}/{key} must be a non-negative integer"))
            })
            .transpose()
    };
    let (eq, min, max) = (number("eq")?, number("min")?, number("max")?);
    match (eq, min, max) {
        (Some(_), Some(_), _) | (Some(_), _, Some(_)) => Err(format!(
            "layout: {path}: eq cannot be combined with min or max"
        )),
        (Some(eq), None, None) => Ok(LineCount { min: eq, max: Some(eq) }),
        (None, None, None) => Err(format!("layout: {path} must set eq, min or max")),
        (None, min, max) => {
            let min = min.unwrap_or(0);
            if max.is_some_and(|max| min > max) {
                return Err(format!("layout: {path}: min must not exceed max"));
            }
            Ok(LineCount { min, max })
        }
    }
}

impl Compiler<'_> {
    /// `before` or `after`: `to`, `comments`, `edge`, `breaks` and `blank`.
    fn layout_gap(&mut self, value: &Json, path: &str) -> Result<Gap, String> {
        let gap = object(value, path)?;
        reject_unknown(gap, &["to", "comments", "edge", "breaks", "blank"], path)?;
        let to = match gap.get("to").and_then(Json::as_str) {
            Some("token") => Neighbour::Token,
            Some("sibling") => Neighbour::Sibling,
            Some(to) if to.starts_with("capture.") => {
                let name = &to["capture.".len()..];
                if name.is_empty() {
                    return Err(format!("layout: {path}/to must be a non-empty string"));
                }
                if !name
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
                {
                    return Err(format!(
                        "layout: {path}/to: `{name}` is not a capture name (A-Z, 0-9, _, without `$`)"
                    ));
                }
                Neighbour::Capture(self.names.id(name))
            }
            _ => {
                return Err(format!(
                    "layout: {path}/to must be \"token\", \"sibling\" or \"capture.<NAME>\""
                ));
            }
        };
        let flag = |key: &str| -> Result<bool, String> {
            match gap.get(key) {
                None => Ok(false),
                Some(Json::Bool(flag)) => Ok(*flag),
                Some(_) => Err(format!("layout: {path}/{key} must be a boolean")),
            }
        };
        let (comments, edge) = (flag("comments")?, flag("edge")?);
        if matches!(to, Neighbour::Capture(_)) {
            for key in ["comments", "edge"] {
                if gap.contains_key(key) {
                    return Err(format!(
                        "layout: {path}/{key}: {key} chooses the neighbouring token or sibling; a capture names its node"
                    ));
                }
            }
        }
        let count = |key: &str| {
            gap.get(key)
                .map(|value| line_count(value, &format!("{path}/{key}")))
                .transpose()
        };
        let (breaks, blank) = (count("breaks")?, count("blank")?);
        if breaks.is_none() && blank.is_none() {
            return Err(format!("layout: {path} must count breaks, blank or both"));
        }
        Ok(Gap { to, comments, edge, breaks, blank })
    }

    /// `layout`, at `at` (the path of the layout object itself, e.g. `rule/layout`).
    pub(super) fn compile_layout(&mut self, v: &Json, at: &str) -> Result<Rule, String> {
        let layout = object(v, at)?;
        reject_unknown(
            layout,
            &["span", "start", "before", "after", "line", "previousLine", "context"],
            at,
        )?;
        if layout.is_empty() {
            return Err(format!("layout: {at} must hold span, start, before or after"));
        }
        let count = |key: &str| {
            layout
                .get(key)
                .map(|value| line_count(value, &format!("{at}/{key}")))
                .transpose()
        };
        let (span, start) = (count("span")?, count("start")?);
        let before = layout
            .get("before")
            .map(|value| self.layout_gap(value, &format!("{at}/before")))
            .transpose()?;
        let after = layout
            .get("after")
            .map(|value| self.layout_gap(value, &format!("{at}/after")))
            .transpose()?;
        let text = |key: &str| -> Result<Option<Regex>, String> {
            layout
                .get(key)
                .map(|value| {
                    let pattern = value
                        .as_str()
                        .ok_or_else(|| format!("layout: {at}/{key} must be a regex string"))?;
                    regex(pattern, &format!("{at}/{key}"))
                })
                .transpose()
        };
        let (line, previous_line) = (text("line")?, text("previousLine")?);
        let context = layout
            .get("context")
            .map(|value| -> Result<LineContext, String> {
                let at = format!("{at}/context");
                let spec = object(value, &at)?;
                reject_unknown(spec, &["before", "after", "through", "regex"], &at)?;
                let number = |key: &str| -> Result<usize, String> {
                    match spec.get(key) {
                        None => Ok(0),
                        Some(v) => v
                            .as_u64()
                            .filter(|n| *n <= 64)
                            .map(|n| n as usize)
                            .ok_or_else(|| {
                                format!("layout: {at}/{key} must be an integer from 0 to 64")
                            }),
                    }
                };
                let pattern = |key: &str| -> Result<Regex, String> {
                    let pattern = spec
                        .get(key)
                        .and_then(Json::as_str)
                        .ok_or_else(|| format!("layout: {at}/{key} must be a regex string"))?;
                    regex(pattern, &format!("{at}/{key}"))
                };
                Ok(LineContext {
                    before: number("before")?,
                    after: number("after")?,
                    through: spec.get("through").map(|_| pattern("through")).transpose()?,
                    regex: pattern("regex")?,
                })
            })
            .transpose()?;
        Ok(Rule::Layout(Box::new(Layout {
            span,
            start,
            before,
            after,
            line,
            previous_line,
            context,
        })))
    }
}

// --- match (matcher/layout.rs) ---

impl Eval<'_> {
    pub(super) fn match_layout(&self, x: &Layout, node: Bound, env: &mut Env) -> bool {
        let n = match node {
            Bound::Node(n) => n,
            // A parsed tree's nodes have no lines of the file: with `span` or an outward key the
            // node fails whatever `start` says. `start` alone reads the literal tree's own offsets.
            Bound::Parsed(_) => {
                if x.span.is_none() && !x.outward() {
                    self.refuse("layout: start on a node of a parsed literal".into());
                }
                return false;
            }
        };
        let f = self.f;
        let range = (f.start[n as usize] as usize, f.end[n as usize] as usize);
        if x.start.is_some_and(|start| !start.holds(range.0 as u64)) {
            return false;
        }
        if x.span.is_none() && !x.outward() {
            return true;
        }
        let lines: &LineTable = &f.lines;
        let source: &str = &f.source;
        let text = move |line: usize| &source[lines.start(line)..lines.end(line)];
        let at = lines.line(range.0);
        if x.line.as_ref().is_some_and(|regex| !regex.is_match(text(at))) {
            return false;
        }
        if let Some(regex) = &x.previous_line {
            if at == 0 || !regex.is_match(text(at - 1)) {
                return false;
            }
        }
        if let Some(context) = &x.context {
            let first = at.saturating_sub(context.before);
            let mut last = (at + context.after).min(lines.line(source.len()));
            if let Some(through) = &context.through {
                for line in first..=last {
                    if through.is_match(text(line)) {
                        last = line;
                        break;
                    }
                }
            }
            if !context.regex.is_match(&source[lines.start(first)..lines.end(last)]) {
                return false;
            }
        }
        if let Some(span) = x.span {
            let breaks = lines.line(range.1) - lines.line(range.0);
            if !span.holds(breaks as u64) {
                return false;
            }
        }
        for (gap, side) in [(&x.before, Side::Before), (&x.after, Side::After)] {
            if let Some(gap) = gap {
                if !self.layout_gap_holds(gap, n, range, side, env) {
                    return false;
                }
            }
        }
        true
    }

    /// Whether the lines between node `n` and its neighbour on `side` count as `gap` asks.
    fn layout_gap_holds(
        &self,
        gap: &Gap,
        n: u32,
        range: (usize, usize),
        side: Side,
        env: &Env,
    ) -> bool {
        let f = self.f;
        let span = |m: u32| (f.start[m as usize] as usize, f.end[m as usize] as usize);
        let neighbour = match gap.to {
            Neighbour::Token => token(f, n, side, gap.comments).map(span),
            Neighbour::Sibling => sibling(f, n, side, gap.comments).map(span),
            // The capture must lie wholly on its side of the node.
            Neighbour::Capture(name) => env
                .get(name)
                .map(|b| {
                    let (start, end) = self.range_of(b);
                    (start as usize, end as usize)
                })
                .filter(|capture| match side {
                    Side::Before => capture.1 <= range.0,
                    Side::After => capture.0 >= range.1,
                }),
        };
        let length = f.source.len();
        let edge = match side {
            Side::Before => (0, 0),
            Side::After => (length, length),
        };
        let Some(neighbour) = neighbour.or_else(|| gap.edge.then_some(edge)) else {
            return false;
        };
        // The neighbour's end and the node's start, or the node's end and the neighbour's start.
        let (from, to) = match side {
            Side::Before => (neighbour.1, range.0),
            Side::After => (range.1, neighbour.0),
        };
        let lines: &LineTable = &f.lines;
        let (first, last) = (lines.line(from), lines.line(to));
        if gap
            .breaks
            .is_some_and(|breaks| !breaks.holds(last.saturating_sub(first) as u64))
        {
            return false;
        }
        let Some(blank) = gap.blank else {
            return true;
        };
        let blank_lines = if last <= first + 1 {
            0
        } else {
            blank_lines(f, n, side, from, to)
        };
        blank.holds(blank_lines as u64)
    }
}

/// The lines strictly between the gap's first and last lines that no token or comment inside the
/// gap touches, walking from the node toward its neighbour.
fn blank_lines(f: &Facts, n: u32, side: Side, gap_start: usize, gap_end: usize) -> usize {
    let lines: &LineTable = &f.lines;
    let (first, last) = (lines.line(gap_start), lines.line(gap_end));
    let mut blank = 0;
    // The line the walk last left: the node's first (before) or last (after).
    let mut at = match side {
        Side::Before => last,
        Side::After => first,
    };
    let mut current = n;
    while let Some(next) = leaf(f, current, side) {
        let (start, end) = (f.start[next as usize] as usize, f.end[next as usize] as usize);
        let inside = match side {
            Side::Before => end > gap_start,
            Side::After => start < gap_end,
        };
        if !inside {
            break;
        }
        let (near, far) = match side {
            Side::Before => (lines.line(end), lines.line(start)),
            Side::After => (lines.line(start), lines.line(end)),
        };
        blank += near.abs_diff(at).saturating_sub(1);
        at = far;
        current = next;
    }
    let end = match side {
        Side::Before => first,
        Side::After => last,
    };
    blank + end.abs_diff(at).saturating_sub(1)
}

/// Whether a node is a comment in some grammar (`util::is_comment_kind`).
fn is_comment(f: &Facts, n: u32) -> bool {
    matches!(
        &*facts::name(f.kind[n as usize]),
        "comment" | "html_comment" | "line_comment" | "block_comment"
    )
}

/// The node's parent's children and the node's index among them; `None` for the root.
fn siblings(f: &Facts, n: u32) -> Option<(&[u32], usize)> {
    let parent = f.parent(n)?;
    let kids = f.children(parent);
    // A parent's children are listed in preorder, so by increasing id.
    let index = kids.binary_search(&n).ok()?;
    Some((kids, index))
}

/// The structurally previous sibling (`TapeNode::prev_sibling`, which `SgNode::prev` is).
fn prev_sibling(f: &Facts, n: u32) -> Option<u32> {
    let (kids, index) = siblings(f, n)?;
    index.checked_sub(1).map(|i| kids[i])
}

/// The structurally next sibling (`TapeNode::next_sibling`).
fn next_sibling(f: &Facts, n: u32) -> Option<u32> {
    let (kids, index) = siblings(f, n)?;
    kids.get(index + 1).copied()
}

/// Tree-sitter's next sibling (`SgNode::next`): the first later sibling ending after this node's
/// end.
fn ts_next(f: &Facts, n: u32) -> Option<u32> {
    let (kids, index) = siblings(f, n)?;
    let end = f.end[n as usize];
    kids[index + 1..]
        .iter()
        .copied()
        .find(|&m| f.end[m as usize] > end)
}

/// The next leaf with a width on `side` of the node, in document order.
fn leaf(f: &Facts, n: u32, side: Side) -> Option<u32> {
    let mut current = n;
    loop {
        let mut next = loop {
            let sibling = match side {
                Side::Before => prev_sibling(f, current),
                Side::After => ts_next(f, current),
            };
            match sibling {
                Some(sibling) => break sibling,
                None => current = f.parent(current)?,
            }
        };
        // Descend to the leaf nearest the node.
        loop {
            let kids = f.children(next);
            let child = match side {
                Side::Before => kids.last(),
                Side::After => kids.first(),
            };
            match child {
                Some(&child) => next = child,
                None => break,
            }
        }
        if f.start[next as usize] < f.end[next as usize] {
            return Some(next);
        }
        current = next;
    }
}

/// The nearest token on `side` of the node that has a width, comments included or passed over.
fn token(f: &Facts, n: u32, side: Side, comments: bool) -> Option<u32> {
    let mut current = n;
    loop {
        let next = leaf(f, current, side)?;
        if comments || !is_comment(f, next) {
            return Some(next);
        }
        current = next;
    }
}

/// Where ast-grep's `next_all`/`prev_all` start (`TapeNode::walk_anchor`): the first child of the
/// parent (the root's first child for the root) that ends after this node's start byte.
fn walk_anchor(f: &Facts, n: u32) -> Option<u32> {
    let start = f.start[n as usize];
    let ends_after = |m: u32| f.end[m as usize] > start;
    let mut anchor = match f.parent(n) {
        Some(_) => n,
        None => *f.children(n).first()?,
    };
    if ends_after(anchor) {
        while let Some(prev) = prev_sibling(f, anchor).filter(|&prev| ends_after(prev)) {
            anchor = prev;
        }
        return Some(anchor);
    }
    loop {
        anchor = next_sibling(f, anchor)?;
        if ends_after(anchor) {
            return Some(anchor);
        }
    }
}

/// The nearest named sibling on `side` of the node (`prev_all`/`next_all`), comments included or
/// passed over.
fn sibling(f: &Facts, n: u32, side: Side, comments: bool) -> Option<u32> {
    let anchor = walk_anchor(f, n)?;
    let (kids, index) = siblings(f, anchor)?;
    let wanted = |m: &u32| f.named(*m) && (comments || !is_comment(f, *m));
    match side {
        Side::Before => kids[..index].iter().rev().copied().find(|m| wanted(m)),
        Side::After => kids[index + 1..].iter().copied().find(|m| wanted(m)),
    }
}
