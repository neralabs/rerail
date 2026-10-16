//! node-semver 7.8.5 ranges and `subset`, with its default options (`loose` and
//! `includePrerelease` off), as eslint-plugin-n compares a configured Node range to a supported one.
//! Ported from node-semver (ISC, Isaac Z. Schlueter and Contributors; see THIRD_PARTY_NOTICES).
//!
//! Parsing follows `new Range(text)` step by step over the library's `safeRe` regexes. Range text is
//! whitespace-normalized first, so JS `\s` is a plain space in every regex after that; JS `\d` is
//! ASCII `[0-9]`.
//!
//! The tests check the port against `semver/subset_cases.json`, node-semver's own answers recorded by
//! `scripts/generate/semver/semver-subset-cases.mjs`.

use regex::{Captures, Regex};
use std::cmp::Ordering;
use std::collections::HashSet;
use std::fmt;
use std::sync::LazyLock;

/// `MAX_LENGTH`: the longest version string `SemVer` accepts.
const MAX_LENGTH: usize = 256;
/// `Number.MAX_SAFE_INTEGER`.
const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

/// A node-semver 7.8.5 `Range` (loose = false, includePrerelease = false).
#[derive(Clone, Debug)]
pub(crate) struct Range {
    set: Vec<SimpleRange>,
}

/// One `||` alternative: comparators that must all hold.
#[derive(Clone, Debug)]
struct SimpleRange {
    /// The parse-cache key (the trimmed alternative without build metadata). node-semver hands out
    /// one comparator array per key, which `simpleSubset`'s `sub === dom` observes. `None` for a
    /// null set, which is never cached.
    key: Option<String>,
    /// Empty for the ANY comparator, which only ever stands alone.
    comparators: Vec<Comparator>,
}

#[derive(Clone, Debug)]
struct Comparator {
    op: Op,
    semver: SemVer,
}

/// A comparator operator; `=` reads as `Eq`, like `''`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Op {
    Eq,
    Gt,
    Gte,
    Lt,
    Lte,
}

#[derive(Clone, Debug)]
struct SemVer {
    major: u64,
    minor: u64,
    patch: u64,
    prerelease: Vec<Ident>,
    /// `major.minor.patch[-prerelease]`, as `SemVer.version` prints it.
    version: String,
}

/// A prerelease identifier: a JS number when all digits and below `MAX_SAFE_INTEGER`, else a string.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Ident {
    Num(u64),
    Str(String),
}

impl Range {
    /// `new Range(text)`; None where node-semver throws.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let raw = collapse_whitespace(text);
        let mut set = raw
            .split("||")
            .map(|range| parse_range(range.trim_matches(' ')))
            .collect::<Option<Vec<_>>>()?;
        if set.len() > 1 {
            if set.iter().all(SimpleRange::is_null_set) {
                set.truncate(1);
            } else {
                set.retain(|simple| !simple.is_null_set());
                if set.len() > 1
                    && let Some(any) = set.iter().position(|s| s.comparators.is_empty())
                {
                    set = vec![set.swap_remove(any)];
                }
            }
        }
        Some(Self { set })
    }

    /// `subset(self, dom)` (ranges/subset.js) with `self` passed as a Range object.
    pub(crate) fn is_subset_of(&self, dom: &Range) -> bool {
        let mut saw_non_null = false;
        'sub: for sub in &self.set {
            for simple_dom in &dom.set {
                let is_sub = simple_subset(sub, simple_dom);
                saw_non_null |= is_sub.is_some();
                if is_sub == Some(true) {
                    continue 'sub;
                }
            }
            if saw_non_null {
                return false;
            }
        }
        true
    }

    /// `Range.test(version)`.
    fn test(&self, version: &SemVer) -> bool {
        self.set.iter().any(|simple| simple.test(version))
    }
}

impl SimpleRange {
    /// `isNullSet(set[0])`: the set opens with `<0.0.0-0`.
    fn is_null_set(&self) -> bool {
        self.comparators
            .first()
            .is_some_and(Comparator::is_null_set)
    }

    /// `testSet`: every comparator holds, and a prerelease version also shares its tuple with a
    /// prerelease comparator.
    fn test(&self, version: &SemVer) -> bool {
        self.comparators.iter().all(|c| c.test(version))
            && (version.prerelease.is_empty()
                || self
                    .comparators
                    .iter()
                    .any(|c| !c.semver.prerelease.is_empty() && c.semver.same_tuple(version)))
    }
}

impl Comparator {
    /// `new Comparator(comp)`: `None` where it throws, `Some(None)` for ANY (`''`).
    fn parse(comp: &str) -> Option<Option<Self>> {
        let m = REGEXES.comparator.captures(comp)?;
        let Some(version) = m.get(2) else {
            return Some(None);
        };
        let op = match &m[1] {
            "" | "=" => Op::Eq,
            ">" => Op::Gt,
            ">=" => Op::Gte,
            "<" => Op::Lt,
            _ => Op::Lte,
        };
        let semver = SemVer::parse(version.as_str())?;
        Some(Some(Self { op, semver }))
    }

    fn is_null_set(&self) -> bool {
        self.op == Op::Lt && self.semver.version == "0.0.0-0"
    }

    /// `Comparator.test(version)`: the operator alone, without testSet's prerelease rule.
    fn test(&self, version: &SemVer) -> bool {
        let order = version.compare(&self.semver);
        match self.op {
            Op::Eq => order.is_eq(),
            Op::Gt => order.is_gt(),
            Op::Gte => order.is_ge(),
            Op::Lt => order.is_lt(),
            Op::Lte => order.is_le(),
        }
    }
}

/// `String(comparator)`: its `value`.
impl fmt::Display for Comparator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let op = match self.op {
            Op::Eq => "",
            Op::Gt => ">",
            Op::Gte => ">=",
            Op::Lt => "<",
            Op::Lte => "<=",
        };
        write!(f, "{op}{}", self.semver.version)
    }
}

impl SemVer {
    /// `new SemVer(text)`; None where it throws.
    fn parse(text: &str) -> Option<Self> {
        if text.len() > MAX_LENGTH {
            return None;
        }
        let m = REGEXES.full.captures(text)?;
        let component = |i: usize| m[i].parse::<u64>().ok().filter(|&n| n <= MAX_SAFE_INTEGER);
        let (major, minor, patch) = (component(1)?, component(2)?, component(3)?);
        let prerelease = m.get(4).map_or_else(Vec::new, |pre| {
            pre.as_str()
                .split('.')
                .map(|id| match id.parse::<u64>() {
                    Ok(n) if n < MAX_SAFE_INTEGER => Ident::Num(n),
                    _ => Ident::Str(id.to_owned()),
                })
                .collect()
        });
        let mut version = format!("{major}.{minor}.{patch}");
        for (i, id) in prerelease.iter().enumerate() {
            version.push(if i == 0 { '-' } else { '.' });
            match id {
                Ident::Num(n) => version.push_str(&n.to_string()),
                Ident::Str(s) => version.push_str(s),
            }
        }
        Some(Self {
            major,
            minor,
            patch,
            prerelease,
            version,
        })
    }

    fn same_tuple(&self, other: &SemVer) -> bool {
        (self.major, self.minor, self.patch) == (other.major, other.minor, other.patch)
    }

    /// `SemVer.compare`.
    fn compare(&self, other: &SemVer) -> Ordering {
        if self.version == other.version {
            return Ordering::Equal;
        }
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| self.compare_pre(other))
    }

    /// `SemVer.comparePre`: no prerelease ranks above any.
    fn compare_pre(&self, other: &SemVer) -> Ordering {
        match (self.prerelease.is_empty(), other.prerelease.is_empty()) {
            (false, true) => return Ordering::Less,
            (true, false) => return Ordering::Greater,
            (true, true) => return Ordering::Equal,
            (false, false) => {}
        }
        let mut ids = self.prerelease.iter();
        let mut other_ids = other.prerelease.iter();
        loop {
            match (ids.next(), other_ids.next()) {
                (None, None) => return Ordering::Equal,
                (Some(_), None) => return Ordering::Greater,
                (None, Some(_)) => return Ordering::Less,
                (Some(a), Some(b)) if a == b => {}
                (Some(a), Some(b)) => return compare_identifiers(a, b),
            }
        }
    }
}

/// `compareIdentifiers`: numbers (and all-digit strings, as JS numbers) rank below other strings.
fn compare_identifiers(a: &Ident, b: &Ident) -> Ordering {
    if let (Ident::Num(a), Ident::Num(b)) = (a, b) {
        return a.cmp(b);
    }
    match (numeric_value(a), numeric_value(b)) {
        (Ok(a), Ok(b)) if a == b => Ordering::Equal,
        (Ok(a), Ok(b)) if a < b => Ordering::Less,
        (Ok(_), Ok(_)) => Ordering::Greater,
        (Ok(_), Err(_)) => Ordering::Less,
        (Err(_), Ok(_)) => Ordering::Greater,
        (Err(a), Err(b)) => a.cmp(b),
    }
}

/// `+id` for an identifier `/^[0-9]+$/` accepts; the text of any other.
fn numeric_value(id: &Ident) -> Result<f64, &str> {
    match id {
        Ident::Num(n) => Ok(*n as f64),
        Ident::Str(s) if s.bytes().all(|b| b.is_ascii_digit()) => s.parse().map_err(|_| s.as_str()),
        Ident::Str(s) => Err(s.as_str()),
    }
}

/// `simpleSubset(sub, dom)`: `None` when `sub` is a null set.
fn simple_subset(sub: &SimpleRange, dom: &SimpleRange) -> Option<bool> {
    if sub.key.is_some() && sub.key == dom.key {
        return Some(true);
    }
    let minimum = &MINIMUM_VERSION[..];
    let sub = if sub.comparators.is_empty() {
        if dom.comparators.is_empty() {
            return Some(true);
        }
        minimum
    } else {
        &sub.comparators
    };
    let dom = if dom.comparators.is_empty() {
        minimum
    } else {
        &dom.comparators
    };

    let mut eq = None;
    let mut gt: Option<&Comparator> = None;
    let mut lt: Option<&Comparator> = None;
    for c in sub {
        match c.op {
            Op::Gt | Op::Gte => {
                if gt.is_none_or(|gt| higher_gt_is_b(gt, c)) {
                    gt = Some(c);
                }
            }
            Op::Lt | Op::Lte => {
                if lt.is_none_or(|lt| lower_lt_is_b(lt, c)) {
                    lt = Some(c);
                }
            }
            // Comparators are deduplicated by value, so a second `=` is a second version.
            Op::Eq if eq.is_some() => return None,
            Op::Eq => eq = Some(&c.semver),
        }
    }

    let mut gtlt_comp = None;
    if let (Some(gt), Some(lt)) = (gt, lt) {
        let order = gt.semver.compare(&lt.semver);
        if order.is_gt() || (order.is_eq() && (gt.op != Op::Gte || lt.op != Op::Lte)) {
            return None;
        }
        gtlt_comp = Some(order);
    }

    if let Some(eq) = eq {
        if gt.is_some_and(|gt| !satisfies(eq, gt)) || lt.is_some_and(|lt| !satisfies(eq, lt)) {
            return None;
        }
        return Some(dom.iter().all(|c| satisfies(eq, c)));
    }

    // A prerelease bound needs a dom comparator with a prerelease on the same tuple; `<x.y.z-0`
    // is exempt, being `<x.y.z`.
    let mut need_dom_lt_pre = lt
        .map(|lt| &lt.semver)
        .filter(|semver| !semver.prerelease.is_empty());
    let mut need_dom_gt_pre = gt
        .map(|gt| &gt.semver)
        .filter(|semver| !semver.prerelease.is_empty());
    if lt.is_some_and(|lt| lt.op == Op::Lt && lt.semver.prerelease == [Ident::Num(0)]) {
        need_dom_lt_pre = None;
    }

    let mut has_dom_gt = false;
    let mut has_dom_lt = false;
    for c in dom {
        let c_is_gt = matches!(c.op, Op::Gt | Op::Gte);
        let c_is_lt = matches!(c.op, Op::Lt | Op::Lte);
        has_dom_gt |= c_is_gt;
        has_dom_lt |= c_is_lt;
        let matches_pre =
            |need: &SemVer| !c.semver.prerelease.is_empty() && c.semver.same_tuple(need);
        if let Some(gt) = gt {
            if need_dom_gt_pre.is_some_and(matches_pre) {
                need_dom_gt_pre = None;
            }
            if c_is_gt {
                if higher_gt_is_b(gt, c) {
                    return Some(false);
                }
            } else if gt.op == Op::Gte && !c.test(&gt.semver) {
                return Some(false);
            }
        }
        if let Some(lt) = lt {
            if need_dom_lt_pre.is_some_and(matches_pre) {
                need_dom_lt_pre = None;
            }
            if c_is_lt {
                if lower_lt_is_b(lt, c) {
                    return Some(false);
                }
            } else if lt.op == Op::Lte && !c.test(&lt.semver) {
                return Some(false);
            }
        }
        if c.op == Op::Eq && (lt.is_some() || gt.is_some()) && gtlt_comp != Some(Ordering::Equal) {
            return Some(false);
        }
    }

    // A one-sided sub is not inside a dom bounded on the other side.
    if (gt.is_some() && has_dom_lt && lt.is_none()) || (lt.is_some() && has_dom_gt && gt.is_none())
    {
        return Some(false);
    }
    Some(need_dom_gt_pre.is_none() && need_dom_lt_pre.is_none())
}

/// `[new Comparator('>=0.0.0')]`, which stands in for ANY.
static MINIMUM_VERSION: LazyLock<[Comparator; 1]> = LazyLock::new(|| {
    [Comparator::parse(">=0.0.0")
        .flatten()
        .expect("`>=0.0.0` is a comparator")]
});

/// Whether `higherGT(a, b)` picks `b`: the higher bound, `>` over `>=` on a tie.
fn higher_gt_is_b(a: &Comparator, b: &Comparator) -> bool {
    match a.semver.compare(&b.semver) {
        Ordering::Greater => false,
        Ordering::Less => true,
        Ordering::Equal => b.op == Op::Gt && a.op == Op::Gte,
    }
}

/// Whether `lowerLT(a, b)` picks `b`: the lower bound, `<` over `<=` on a tie.
fn lower_lt_is_b(a: &Comparator, b: &Comparator) -> bool {
    match a.semver.compare(&b.semver) {
        Ordering::Less => false,
        Ordering::Greater => true,
        Ordering::Equal => b.op == Op::Lt && a.op == Op::Lte,
    }
}

/// `satisfies(version, String(c))`: the comparator's value reparsed as a range, so `>=0.0.0` is
/// ANY and testSet's prerelease rule applies.
fn satisfies(version: &SemVer, c: &Comparator) -> bool {
    Range::parse(&c.to_string()).is_some_and(|range| range.test(version))
}

/// `range.trim().replace(/\s+/g, ' ')` with JS's whitespace set.
fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pending_space = false;
    for ch in text.trim_matches(is_js_whitespace).chars() {
        if is_js_whitespace(ch) {
            pending_space = true;
        } else {
            if pending_space {
                out.push(' ');
                pending_space = false;
            }
            out.push(ch);
        }
    }
    out
}

/// JS `\s`: WhiteSpace and LineTerminator (U+FEFF included, U+0085 not).
fn is_js_whitespace(ch: char) -> bool {
    matches!(
        ch,
        '\t'..='\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2028}' | '\u{2029}'
    ) || matches!(
        ch,
        '\u{2000}'..='\u{200a}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
    )
}

/// `parseRange` for one trimmed `||` alternative.
fn parse_range(range: &str) -> Option<SimpleRange> {
    let re = &*REGEXES;
    let key = re.build.replace_all(range, "").into_owned();
    let range = re.hyphen_range.replace(&key, hyphen_replace);
    let range = re.comparator_trim.replace_all(&range, "${1}${2}${3}");
    let range = re.tilde_trim.replace_all(&range, "${1}~");
    let range = re.caret_trim.replace_all(&range, "${1}^");
    let range = range
        .split(' ')
        .map(parse_comparator)
        .collect::<Vec<_>>()
        .join(" ");
    // Every comparator is built (and may throw) before the null-set check.
    let parsed = re
        .spaces
        .split(&range)
        .map(|comp| Comparator::parse(&re.gte0.replace(comp.trim_matches(' '), "")))
        .collect::<Option<Vec<_>>>()?;
    // node-semver's `rangeMap`: each comparator whose operator and version are new, in first-seen
    // order. Hashed, since a manifest's range is untrusted and may hold any number of them.
    let mut seen = HashSet::with_capacity(parsed.len());
    let mut comparators: Vec<Comparator> = Vec::with_capacity(parsed.len());
    for comp in parsed.into_iter().flatten() {
        if comp.is_null_set() {
            return Some(SimpleRange {
                key: None,
                comparators: vec![comp],
            });
        }
        if seen.insert((comp.op, comp.semver.version.clone())) {
            comparators.push(comp);
        }
    }
    Some(SimpleRange {
        key: Some(key),
        comparators,
    })
}

/// `parseComparator`: carets, tildes, x-ranges and stars desugared to plain comparators. Its
/// leading build strip never matches after `parseRange`'s global one, so it is left out.
fn parse_comparator(comp: &str) -> String {
    let re = &*REGEXES;
    let comp = replace_each(comp.trim_matches(' '), |c| {
        re.caret.replace(c, replace_caret).into_owned()
    });
    let comp = replace_each(comp.trim_matches(' '), |c| {
        re.tilde.replace(c, replace_tilde).into_owned()
    });
    let comp = replace_each(&comp, replace_x_range);
    re.star.replace(comp.trim_matches(' '), "").into_owned()
}

/// `comp.split(/\s+/).map(replace).join(' ')`.
fn replace_each(comp: &str, replace: impl Fn(&str) -> String) -> String {
    REGEXES
        .spaces
        .split(comp)
        .map(replace)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Capture `i` unless missing or an X (`x`, `X`, `*`), as `isX` reads it.
fn number<'h>(caps: &Captures<'h>, i: usize) -> Option<&'h str> {
    caps.get(i)
        .map(|m| m.as_str())
        .filter(|part| !matches!(*part, "x" | "X" | "*"))
}

/// JS `+part + 1` for a matched numeric part. Past `MAX_SAFE_INTEGER` the result only has to be
/// out of range, which `SemVer` then rejects, so 2^53 stands in for it.
fn succ(part: &str) -> String {
    match part.parse::<u64>() {
        Ok(n) if n < MAX_SAFE_INTEGER => (n + 1).to_string(),
        _ => (MAX_SAFE_INTEGER + 1).to_string(),
    }
}

/// The `replaceCaret` callback: `^M.m.p-pr` up to the bump of its first nonzero part.
fn replace_caret(caps: &Captures<'_>) -> String {
    let Some(maj) = number(caps, 1) else {
        return String::new();
    };
    let Some(min) = number(caps, 2) else {
        return format!(">={maj}.0.0 <{}.0.0-0", succ(maj));
    };
    let Some(pat) = number(caps, 3) else {
        return if maj == "0" {
            format!(">={maj}.{min}.0 <{maj}.{}.0-0", succ(min))
        } else {
            format!(">={maj}.{min}.0 <{}.0.0-0", succ(maj))
        };
    };
    let lower = match caps.get(4) {
        Some(pre) => format!(">={maj}.{min}.{pat}-{}", pre.as_str()),
        None => format!(">={maj}.{min}.{pat}"),
    };
    if maj != "0" {
        format!("{lower} <{}.0.0-0", succ(maj))
    } else if min != "0" {
        format!("{lower} <{maj}.{}.0-0", succ(min))
    } else {
        format!("{lower} <{maj}.{min}.{}-0", succ(pat))
    }
}

/// The `replaceTilde` callback: `~M.m.p-pr` (or `~>`) up to the next minor.
fn replace_tilde(caps: &Captures<'_>) -> String {
    let Some(maj) = number(caps, 1) else {
        return String::new();
    };
    let Some(min) = number(caps, 2) else {
        return format!(">={maj}.0.0 <{}.0.0-0", succ(maj));
    };
    let upper = format!("<{maj}.{}.0-0", succ(min));
    match (number(caps, 3), caps.get(4)) {
        (None, _) => format!(">={maj}.{min}.0 {upper}"),
        (Some(pat), Some(pre)) => format!(">={maj}.{min}.{pat}-{} {upper}", pre.as_str()),
        (Some(pat), None) => format!(">={maj}.{min}.{pat} {upper}"),
    }
}

/// `replaceXRange` for one comparator: an operator over a partial version (`1.x`, `>=1.2`,
/// `<=1.x`, `>*`) as plain comparators; anything else is left as is.
fn replace_x_range(comp: &str) -> String {
    let comp = comp.trim_matches(' ');
    let Some(caps) = REGEXES.xrange.captures(comp) else {
        return comp.to_owned();
    };
    let (major, minor, patch) = (number(&caps, 2), number(&caps, 3), number(&caps, 4));
    // `x.1` and `1.x.2` stay, for `Comparator` to reject.
    if (major.is_none() && minor.is_some()) || (minor.is_none() && patch.is_some()) {
        return comp.to_owned();
    }
    let gtlt = &caps[1];
    let Some(maj) = major else {
        let any = if matches!(gtlt, ">" | "<") {
            "<0.0.0-0"
        } else {
            "*"
        };
        return any.to_owned();
    };
    let Some(min) = minor else {
        return match gtlt {
            "" | "=" => format!(">={maj}.0.0 <{}.0.0-0", succ(maj)),
            ">" => format!(">={}.0.0", succ(maj)),
            "<=" => format!("<{}.0.0-0", succ(maj)),
            "<" => format!("<{maj}.0.0-0"),
            _ => format!(">={maj}.0.0"),
        };
    };
    if patch.is_some() {
        return comp.to_owned();
    }
    match gtlt {
        "" | "=" => format!(">={maj}.{min}.0 <{maj}.{}.0-0", succ(min)),
        ">" => format!(">={maj}.{}.0", succ(min)),
        "<=" => format!("<{maj}.{}.0-0", succ(min)),
        "<" => format!("<{maj}.{min}.0-0"),
        _ => format!(">={maj}.{min}.0"),
    }
}

/// The `hyphenReplace` callback: `1.2 - 3.4` => `>=1.2.0 <3.5.0-0`.
fn hyphen_replace(caps: &Captures<'_>) -> String {
    let from = match (number(caps, 2), number(caps, 3), number(caps, 4)) {
        (None, ..) => String::new(),
        (Some(maj), None, _) => format!(">={maj}.0.0"),
        (Some(maj), Some(min), None) => format!(">={maj}.{min}.0"),
        (Some(_), Some(_), Some(_)) => format!(">={}", &caps[1]),
    };
    let to = match (number(caps, 8), number(caps, 9), number(caps, 10)) {
        (None, ..) => String::new(),
        (Some(maj), None, _) => format!("<{}.0.0-0", succ(maj)),
        (Some(maj), Some(min), None) => format!("<{maj}.{}.0-0", succ(min)),
        (Some(maj), Some(min), Some(pat)) => match caps.get(11) {
            Some(pre) => format!("<={maj}.{min}.{pat}-{}", pre.as_str()),
            None => format!("<={}", &caps[7]),
        },
    };
    format!("{from} {to}").trim_matches(' ').to_owned()
}

/// The `safeRe` regexes `Range` uses, over whitespace-normalized text.
struct Regexes {
    full: Regex,
    comparator: Regex,
    xrange: Regex,
    tilde: Regex,
    caret: Regex,
    hyphen_range: Regex,
    comparator_trim: Regex,
    tilde_trim: Regex,
    caret_trim: Regex,
    /// `src[t.BUILD]`, unbounded, as `parseRange` strips build metadata.
    build: Regex,
    star: Regex,
    gte0: Regex,
    /// `/\s+/`.
    spaces: Regex,
}

static REGEXES: LazyLock<Regexes> = LazyLock::new(|| {
    let num = "0|[1-9][0-9]{0,256}";
    let num_loose = "[0-9]{1,256}";
    let non_num = "[0-9]{0,256}[a-zA-Z-][a-zA-Z0-9-]{0,250}";
    let main = format!(r"({num})\.({num})\.({num})");
    let main_loose = format!(r"({num_loose})\.({num_loose})\.({num_loose})");
    let pre_id = format!("(?:{non_num}|{num})");
    let pre_id_loose = format!("(?:{non_num}|{num_loose})");
    let pre = format!(r"(?:-({pre_id}(?:\.{pre_id})*))");
    let pre_loose = format!(r"(?:-?({pre_id_loose}(?:\.{pre_id_loose})*))");
    let build_id = "[a-zA-Z0-9-]{1,250}";
    let build = format!(r"(?:\+({build_id}(?:\.{build_id})*))");
    let full_plain = format!("v?{main}{pre}?{build}?");
    let loose_plain = format!("[v= ]*{main_loose}{pre_loose}?{build}?");
    let gtlt = "((?:<|>)?=?)";
    let xid = r"0|[1-9][0-9]{0,256}|x|X|\*";
    let xrange_plain = format!(r"[v= ]*({xid})(?:\.({xid})(?:\.({xid})(?:{pre})?{build}?)?)?");
    Regexes {
        full: compile(&format!("^{full_plain}$")),
        comparator: compile(&format!("^{gtlt} ?({full_plain})$|^$")),
        xrange: compile(&format!("^{gtlt} ?{xrange_plain}$")),
        tilde: compile(&format!("^(?:~>?){xrange_plain}$")),
        caret: compile(&format!(r"^(?:\^){xrange_plain}$")),
        hyphen_range: compile(&format!("^ ?({xrange_plain}) - ({xrange_plain}) ?$")),
        comparator_trim: compile(&format!("( ?){gtlt} ?({loose_plain}|{xrange_plain})")),
        tilde_trim: compile("( ?)(?:~>?) "),
        caret_trim: compile(r"( ?)(?:\^) "),
        build: compile(r"(?:\+([a-zA-Z0-9-]+(?:\.[a-zA-Z0-9-]+)*))"),
        star: compile(r"(<|>)?=? ?\*"),
        gte0: compile(r"^ ?>= ?0\.0\.0 ?$"),
        spaces: compile(" +"),
    }
});

fn compile(source: &str) -> Regex {
    Regex::new(source).expect("node-semver regexes compile")
}

#[cfg(test)]
mod tests {
    use super::{Op, Range};

    /// `[configured, given, subset(new Range(configured), given)]` and `[text, parses]`, recorded
    /// from node-semver 7.8.5.
    const CASES: &str = include_str!("subset_cases.json");

    #[test]
    fn matches_node_semver() {
        let cases: serde_json::Value = serde_json::from_str(CASES).unwrap();
        let mut mismatches = Vec::new();
        for case in cases["valid"].as_array().unwrap() {
            let (text, valid) = (case[0].as_str().unwrap(), case[1].as_bool().unwrap());
            if Range::parse(text).is_some() != valid {
                mismatches.push(format!("parse {text:?}: expected valid = {valid}"));
            }
        }
        for case in cases["subset"].as_array().unwrap() {
            let (configured, given) = (case[0].as_str().unwrap(), case[1].as_str().unwrap());
            let expected = case[2].as_bool().unwrap();
            let (Some(sub), Some(dom)) = (Range::parse(configured), Range::parse(given)) else {
                mismatches.push(format!("subset {configured:?} {given:?}: does not parse"));
                continue;
            };
            if sub.is_subset_of(&dom) != expected {
                mismatches.push(format!(
                    "subset {configured:?} {given:?}: expected {expected}"
                ));
            }
        }
        assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    }

    /// A range repeating its comparators keeps each once, in its first place, and answers
    /// `subset` as it does without the repeats; a null set among them is the range alone.
    #[test]
    fn a_range_keeps_each_comparator_once() {
        let range = Range::parse(">=1.2.0 <2.0.0 >=1.2.0 >1.0.0 <2.0.0").unwrap();
        let kept = range.set[0]
            .comparators
            .iter()
            .map(|comparator| (comparator.op, comparator.semver.version.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            kept,
            [(Op::Gte, "1.2.0"), (Op::Lt, "2.0.0"), (Op::Gt, "1.0.0")]
        );
        assert!(range.is_subset_of(&Range::parse(">=1.0.0 <3.0.0").unwrap()));
        assert!(!range.is_subset_of(&Range::parse(">=1.3.0").unwrap()));
        let null = Range::parse(">=1.2.0 >=1.2.0 <0.0.0-0 >=3.0.0").unwrap();
        assert_eq!(null.set[0].comparators.len(), 1);
        assert!(null.set[0].key.is_none());
    }

    /// A manifest's `engines.node` is untrusted, so a range of 50,000 distinct comparators parses
    /// in time linear in their number: pairwise deduplication makes that over 10⁹ comparisons.
    #[test]
    fn a_range_of_many_comparators_parses_quickly() {
        let text = (0..50_000)
            .map(|minor| format!(">=1.{minor}.0"))
            .collect::<Vec<_>>()
            .join(" ");
        let started = std::time::Instant::now();
        let range = Range::parse(&text).unwrap();
        let elapsed = started.elapsed();
        assert_eq!(range.set[0].comparators.len(), 50_000);
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "parsing took {elapsed:?}"
        );
        assert!(range.is_subset_of(&Range::parse(">=1.0.0").unwrap()));
    }
}
