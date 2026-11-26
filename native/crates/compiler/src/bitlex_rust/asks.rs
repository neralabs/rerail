//! A level's questions set where the level is, not at each call (decision 0703).
//!
//! `asks` carries the questions the levels that may end at a mark ask (Kotlin's: `semi`'s, the lean parser's `stmt`).
//! A level's call sets it (`asks & keep | set`), calls, and restores it, so within a level it is the level's entry's.
//! The values each level is entered with are known (a fixpoint over the calls, the root's none). A call whose setting
//! leaves every value its caller may hold as it is, is a plain call. A level whose every setting call sets it alike,
//! and whose plain calls (`self.f()`, `Self::f` given to a list) come from callers its setting leaves as they are, sets
//! it itself, at its start, as the lean parser sets `stmt` where a statement starts: its calls are plain.

use std::collections::{BTreeMap, BTreeSet};

/// A call that sets `asks`: the lines from its save to its restore, its callee and its setting.
struct Setting {
    lines: std::ops::Range<usize>,
    callee: String,
    keep: u64,
    set: u64,
}

/// `rules` (the levels' methods) with each setting of `asks` that changes nothing gone and each level's setting in the
/// level where it is one; `root` the level the parse starts in, where `asks` is none.
pub(super) fn settle(rules: &str, root: &str) -> String {
    let lines: Vec<&str> = rules.split_inclusive('\n').collect();
    // Each method: its name, its first line (`fn`) and its last (`    }`).
    let mut fns: Vec<(String, usize, usize)> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if let Some(rest) = lines[i].strip_prefix("    fn ") {
            let name: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
            let Some(end) = (i + 1..lines.len()).find(|&j| lines[j] == "    }\n") else { return rules.to_owned() };
            fns.push((name, i, end));
            i = end + 1;
        } else {
            i += 1;
        }
    }
    let names: BTreeSet<&str> = fns.iter().map(|f| f.0.as_str()).collect();
    let parse = |t: &str| t.strip_prefix("0x").and_then(|h| u64::from_str_radix(h, 16).ok());
    // Each method's settings and plain calls.
    let mut settings: BTreeMap<&str, Vec<Setting>> = BTreeMap::new();
    let mut plain: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for (name, a, b) in &fns {
        let mut sets = Vec::new();
        let mut calls = Vec::new();
        let mut j = a + 1;
        while j < *b {
            if lines[j].trim() == "let asks = self.asks;"
                && j + 3 < *b
                && let Some((keep, set)) = lines[j + 1].trim().strip_prefix("self.asks = asks & ").and_then(|r| r.strip_suffix(';')).and_then(|r| r.split_once(" | "))
                && let (Some(keep), Some(set)) = (parse(keep), parse(set))
                && let Some(callee) = lines[j + 2].trim().strip_prefix("self.").and_then(|r| r.strip_suffix("()?;"))
                && lines[j + 3].trim() == "self.asks = asks;"
            {
                sets.push(Setting { lines: j..j + 4, callee: callee.to_owned(), keep, set });
                j += 4;
                continue;
            }
            for pat in ["self.", "Self::"] {
                for (k, _) in lines[j].match_indices(pat) {
                    let r = &lines[j][k + pat.len()..];
                    let f: String = r.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                    if names.contains(f.as_str()) {
                        calls.push(f);
                    }
                }
            }
            j += 1;
        }
        settings.insert(name.as_str(), sets);
        plain.insert(name.as_str(), calls);
    }
    // The values each level is entered with.
    let mut entry: BTreeMap<&str, BTreeSet<u64>> = names.iter().map(|n| (*n, BTreeSet::new())).collect();
    entry.entry(root).or_default().insert(0);
    loop {
        let mut changed = false;
        for (name, _, _) in &fns {
            let from = entry[name.as_str()].clone();
            for s in &settings[name.as_str()] {
                let new: BTreeSet<u64> = from.iter().map(|e| e & s.keep | s.set).collect();
                let to = entry.entry(s.callee.as_str()).or_default();
                let n = to.len();
                to.extend(new);
                changed |= to.len() != n;
            }
            for c in &plain[name.as_str()] {
                let to = entry.entry(c.as_str()).or_default();
                let n = to.len();
                to.extend(from.iter().copied());
                changed |= to.len() != n;
            }
        }
        if !changed {
            break;
        }
    }
    let leaves = |caller: &str, keep: u64, set: u64| entry[caller].iter().all(|e| e & keep | set == *e);
    // The values a level's callers hold where they call it.
    let mut callers: BTreeMap<&str, BTreeSet<u64>> = BTreeMap::new();
    for (caller, sets) in &settings {
        for s in sets {
            callers.entry(s.callee.as_str()).or_default().extend(entry[caller].iter().copied());
        }
    }
    for (caller, cs) in &plain {
        for c in cs {
            callers.entry(c.as_str()).or_default().extend(entry[caller].iter().copied());
        }
    }
    let changes = |name: &str, keep: u64, set: u64| callers.get(name).is_some_and(|v| v.iter().any(|e| e & keep | set != *e));
    // The levels that set it themselves: one setting, every plain caller's values left as they are, and one end.
    let mut own: BTreeMap<&str, (u64, u64)> = BTreeMap::new();
    for (name, a, b) in &fns {
        let forms: BTreeSet<(u64, u64)> = settings.values().flatten().filter(|s| s.callee == *name).map(|s| (s.keep, s.set)).collect();
        let [(keep, set)] = forms.iter().copied().collect::<Vec<_>>()[..] else { continue };
        let plain_ok = plain.iter().all(|(caller, cs)| !cs.iter().any(|c| c == name) || leaves(caller, keep, set));
        let body = &lines[a + 1..*b];
        let one_end = body.last().is_some_and(|l| *l == "        Ok(())\n") && !body.iter().any(|l| l.contains("return Ok(") || l.contains("level_asks"));
        if plain_ok && one_end {
            own.insert(name.as_str(), (keep, set));
        }
    }
    let mut drop = vec![false; lines.len()];
    let mut keep_call = vec![false; lines.len()];
    for (caller, sets) in &settings {
        for s in sets {
            if own.contains_key(s.callee.as_str()) || leaves(caller, s.keep, s.set) {
                for l in s.lines.clone() {
                    drop[l] = true;
                }
                keep_call[s.lines.start + 2] = true;
            }
        }
    }
    let mut out = String::with_capacity(rules.len());
    let starts: BTreeMap<usize, &str> = fns.iter().map(|f| (f.1, f.0.as_str())).collect();
    let ends: BTreeMap<usize, &str> = fns.iter().map(|f| (f.2, f.0.as_str())).collect();
    for (j, l) in lines.iter().enumerate() {
        if let Some(name) = ends.get(&j)
            && let Some(_) = own.get(name)
        {
            // Before its `Ok(())`: the caller's back.
            let at = out.rfind("        Ok(())\n").expect("one end");
            out.insert_str(at, "        self.asks = level_asks;\n");
        }
        if drop[j] && !keep_call[j] {
            continue;
        }
        if keep_call[j] {
            // The call alone, at the save's indent.
            let indent = &lines[j - 2][..lines[j - 2].len() - lines[j - 2].trim_start().len()];
            out.push_str(indent);
            out.push_str(l.trim_start());
            continue;
        }
        out.push_str(l);
        if let Some(name) = starts.get(&j)
            && let Some((keep, set)) = own.get(name)
        {
            if changes(name, *keep, *set) {
                let _ = std::fmt::Write::write_fmt(&mut out, format_args!("        // The level's questions (`asks`, decision 0703): its own, set where it starts.\n        let level_asks = self.asks;\n        self.asks = level_asks & {keep:#x} | {set:#x};\n"));
            }
        }
    }
    // A level whose setting left every value it is entered with: no restore either.
    let mut fixed = String::with_capacity(out.len());
    let mut in_noop = false;
    for l in out.split_inclusive('\n') {
        if let Some(rest) = l.strip_prefix("    fn ") {
            let name: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
            in_noop = own.get(name.as_str()).is_some_and(|(k, s)| !changes(name.as_str(), *k, *s));
        }
        if in_noop && l == "        self.asks = level_asks;\n" {
            continue;
        }
        fixed.push_str(l);
    }
    fixed
}

/// `rules` with a level whose every call that sets `asks` sets it alike, and which reads none of it itself (nor do the
/// levels it calls plainly, through theirs), setting it once at its start: the context, as the lean parser sets `stmt`
/// where a statement or a parenthesis starts, not at each of its calls.
pub(super) fn contexts(rules: &str) -> String {
    let lines: Vec<&str> = rules.split_inclusive('\n').collect();
    let mut fns: Vec<(String, usize, usize)> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if let Some(rest) = lines[i].strip_prefix("    fn ") {
            let name: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
            let Some(end) = (i + 1..lines.len()).find(|&j| lines[j] == "    }\n") else { return rules.to_owned() };
            fns.push((name, i, end));
            i = end + 1;
        } else {
            i += 1;
        }
    }
    let names: BTreeSet<&str> = fns.iter().map(|f| f.0.as_str()).collect();
    let parse = |t: &str| t.strip_prefix("0x").and_then(|h| u64::from_str_radix(h, 16).ok());
    let mut settings: BTreeMap<&str, Vec<Setting>> = BTreeMap::new();
    let mut plain: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    let mut reads: BTreeMap<&str, bool> = BTreeMap::new();
    for (name, a, b) in &fns {
        let mut sets = Vec::new();
        let mut calls = BTreeSet::new();
        let mut read = false;
        let mut j = a + 1;
        while j < *b {
            if lines[j].trim() == "let asks = self.asks;"
                && j + 3 < *b
                && let Some((keep, set)) = lines[j + 1].trim().strip_prefix("self.asks = asks & ").and_then(|r| r.strip_suffix(';')).and_then(|r| r.split_once(" | "))
                && let (Some(keep), Some(set)) = (parse(keep), parse(set))
                && let Some(callee) = lines[j + 2].trim().strip_prefix("self.").and_then(|r| r.strip_suffix("()?;"))
                && lines[j + 3].trim() == "self.asks = asks;"
            {
                sets.push(Setting { lines: j..j + 4, callee: callee.to_owned(), keep, set });
                j += 4;
                continue;
            }
            let t = lines[j].trim();
            // A read of it: any but a level's own setting and restore.
            if t.contains("self.asks") && !t.starts_with("let level_asks = self.asks;") && !t.starts_with("self.asks = level_asks") {
                read = true;
            }
            for pat in ["self.", "Self::"] {
                for (k, _) in lines[j].match_indices(pat) {
                    let f: String = lines[j][k + pat.len()..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                    if names.contains(f.as_str()) {
                        calls.insert(f);
                    }
                }
            }
            j += 1;
        }
        settings.insert(name.as_str(), sets);
        plain.insert(name.as_str(), calls);
        reads.insert(name.as_str(), read);
    }
    // Whether a level, or one it calls plainly, reads `asks` (a level that sets its own reads none of its caller's).
    let sets_own: BTreeSet<&str> = fns.iter().filter(|(_, a, b)| lines[a + 1..*b].iter().any(|l| l.trim().starts_with("self.asks = level_asks & 0x0 |"))).map(|f| f.0.as_str()).collect();
    let mut reaches: BTreeMap<&str, bool> = names.iter().map(|n| (*n, reads[n] && !sets_own.contains(n))).collect();
    loop {
        let mut changed = false;
        for (name, _, _) in &fns {
            if reaches[name.as_str()] || sets_own.contains(name.as_str()) {
                continue;
            }
            if plain[name.as_str()].iter().any(|c| reaches[c.as_str()]) {
                reaches.insert(name.as_str(), true);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let mut own: BTreeMap<&str, (u64, u64)> = BTreeMap::new();
    for (name, a, b) in &fns {
        let sets = &settings[name.as_str()];
        let forms: BTreeSet<(u64, u64)> = sets.iter().map(|s| (s.keep, s.set)).collect();
        let [(keep, set)] = forms.iter().copied().collect::<Vec<_>>()[..] else { continue };
        let body = &lines[a + 1..*b];
        let one_end = body.last().is_some_and(|l| *l == "        Ok(())\n") && !body.iter().any(|l| l.contains("return Ok(") || l.contains("level_asks"));
        if sets.len() >= 2 && one_end && !reads[name.as_str()] && !plain[name.as_str()].iter().any(|c| reaches[c.as_str()]) {
            own.insert(name.as_str(), (keep, set));
        }
    }
    let mut out = String::with_capacity(rules.len());
    let starts: BTreeMap<usize, &str> = fns.iter().map(|f| (f.1, f.0.as_str())).collect();
    let ends: BTreeMap<usize, &str> = fns.iter().map(|f| (f.2, f.0.as_str())).collect();
    let mut skip: BTreeMap<usize, usize> = BTreeMap::new();
    for (name, _) in &own {
        for s in &settings[name] {
            skip.insert(s.lines.start, s.lines.end);
        }
    }
    let mut j = 0;
    while j < lines.len() {
        if let Some(name) = ends.get(&j)
            && own.contains_key(name)
        {
            let at = out.rfind("        Ok(())\n").expect("one end");
            out.insert_str(at, "        self.asks = level_asks;\n");
        }
        if let Some(&e) = skip.get(&j) {
            let indent = &lines[j][..lines[j].len() - lines[j].trim_start().len()];
            out.push_str(indent);
            out.push_str(lines[j + 2].trim_start());
            j = e;
            continue;
        }
        out.push_str(lines[j]);
        if let Some(name) = starts.get(&j)
            && let Some((keep, set)) = own.get(name)
        {
            let _ = std::fmt::Write::write_fmt(&mut out, format_args!("        // The context's questions (`asks`, decision 0703): set where it starts, for every level it calls.\n        let level_asks = self.asks;\n        self.asks = level_asks & {keep:#x} | {set:#x};\n"));
        }
        j += 1;
    }
    out
}

/// A level's own setting of `asks` (`level_asks & keep | set` at its start): the lines from its comment to its
/// setting, its restore's line, and the setting.
struct Own {
    lines: std::ops::Range<usize>,
    restore: Option<usize>,
    keep: u64,
    set: u64,
}

/// What a level's method is, for `lift`: its own setting, whether it reads `asks` (`self.asks`, an `ask_M(true)`),
/// the levels it calls, and whether it makes a node.
struct Level<'a> {
    at: (usize, usize),
    own: Option<Own>,
    reads: bool,
    calls: BTreeSet<&'a str>,
    node: bool,
    /// It ends at its last `Ok(())` alone: a setting of its own is restored there.
    one_end: bool,
}

/// `rules` with a level's own setting that ignores its caller's (`level_asks & 0x0 | set`) set where its body starts
/// instead, as the lean parser sets `stmt` where a body starts (`statements`, `class_body`, `when`'s), not at each
/// statement or member: up through its callers that make no node (a list's loop, `member_ends`), to the one that makes
/// the body's. There it is that level's own setting where its own reads (constants of its old value then) and its
/// other callees let it be, and else a setting of the call. The callee ignored its callers' value: the parse is the
/// same.
/// `asking`: each mark's question asker (`ask_M`, its name `M`) and the bit of `asks` it reads at a level's end.
pub(super) fn lift(rules: &str, root: &str, asking: &BTreeMap<String, u32>) -> String {
    let lines: Vec<&str> = rules.split_inclusive('\n').collect();
    let mut spans: Vec<(String, usize, usize)> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if let Some(rest) = lines[i].strip_prefix("    fn ") {
            let name: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
            let Some(end) = (i + 1..lines.len()).find(|&j| lines[j] == "    }\n") else { return rules.to_owned() };
            spans.push((name, i, end));
            i = end + 1;
        } else {
            i += 1;
        }
    }
    // A mark's question asker (`ask_M(end)`): at a level's end (`true`) it reads the level's bit, else nothing.
    let askers: BTreeMap<String, u32> = asking.iter().map(|(m, b)| (format!("ask_{m}"), *b)).collect();
    let names: BTreeSet<&str> = spans.iter().map(|f| f.0.as_str()).filter(|n| !askers.contains_key(*n)).collect();
    let parse = |t: &str| t.strip_prefix("0x").and_then(|h| u64::from_str_radix(h, 16).ok());
    let mut lv: BTreeMap<&str, Level> = BTreeMap::new();
    let mut set_calls: BTreeSet<&str> = BTreeSet::new();
    for (name, a, b) in &spans {
        if !names.contains(name.as_str()) {
            continue;
        }
        let mut own = None;
        for j in a + 1..(a + 4).min(*b) {
            if lines[j].trim() == "let level_asks = self.asks;"
                && let Some((k, s)) = lines[j + 1].trim().strip_prefix("self.asks = level_asks & ").and_then(|r| r.strip_suffix(';')).and_then(|r| r.split_once(" | "))
                && let (Some(keep), Some(set)) = (parse(k), parse(s))
            {
                let from = if lines[j - 1].trim().starts_with("// The ") { j - 1 } else { j };
                let restore = (j + 2..*b).rev().find(|&r| lines[r] == "        self.asks = level_asks;\n");
                own = Some(Own { lines: from..j + 2, restore, keep, set });
                break;
            }
        }
        let skip: BTreeSet<usize> = own.iter().flat_map(|o: &Own| o.lines.clone().chain(o.restore)).collect();
        let (mut reads, mut node, mut calls) = (false, false, BTreeSet::new());
        for j in a + 1..*b {
            let t = lines[j].trim();
            if !skip.contains(&j) && t.contains("self.asks") && t != "let asks = self.asks;" && t != "self.asks = asks;" && !t.starts_with("self.asks = asks & 0x0 |") {
                reads = true;
            }
            if askers.keys().any(|k| t.contains(&format!("self.{k}(true)"))) {
                reads = true;
            }
            if t.starts_with("self.open(") {
                node = true;
            }
            if t.starts_with("self.asks = asks & ")
                && let Some(f) = lines.get(j + 1).and_then(|l| l.trim().strip_prefix("self.")).and_then(|r| r.strip_suffix("()?;"))
                && let Some(f) = names.get(f)
            {
                set_calls.insert(f);
            }
            for pat in ["self.", "Self::"] {
                for (k, _) in lines[j].match_indices(pat) {
                    let f: String = lines[j][k + pat.len()..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                    if let Some(f) = names.get(f.as_str()) {
                        calls.insert(*f);
                    }
                }
            }
        }
        let one_end = lines[b - 1] == "        Ok(())\n" && !lines[a + 1..*b].iter().any(|l| l.contains("return Ok(") || l.contains("level_asks"));
        lv.insert(name.as_str(), Level { at: (*a, *b), own, reads, calls, node, one_end });
    }
    let mut callers: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (n, l) in &lv {
        for c in &l.calls {
            callers.entry(*c).or_default().insert(*n);
        }
    }
    // Whether a level reads its caller's `asks`, or one it calls without setting it does.
    let mut reaches: BTreeMap<&str, bool> = lv.iter().map(|(n, l)| (*n, l.reads && l.own.is_none())).collect();
    loop {
        let mut changed = false;
        for (n, l) in &lv {
            if !reaches[n] && l.own.is_none() && l.calls.iter().any(|c| reaches[c]) {
                reaches.insert(n, true);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    // Where a setting goes from level `l`, reached through its callee `via`: `l` sets it itself (`setters`), sets it
    // for its call of `via` (`wraps`), or passes it to its callers (a level that makes no node and reads none of it).
    #[allow(clippy::too_many_arguments)]
    fn up<'a>(l: &'a str, via: &'a str, root: &str, lv: &BTreeMap<&'a str, Level<'a>>, reaches: &BTreeMap<&'a str, bool>, callers: &BTreeMap<&'a str, BTreeSet<&'a str>>, seen: &mut BTreeSet<&'a str>, setters: &mut BTreeSet<&'a str>, wraps: &mut BTreeSet<(&'a str, &'a str)>) {
        let x = &lv[l];
        let others_read = x.calls.iter().any(|c| *c != via && reaches[c]);
        let body = x.node || l == root || x.own.is_some();
        if !body && !x.reads && !others_read && callers.get(l).is_some_and(|c| !c.is_empty()) {
            if seen.insert(l) {
                for c in &callers[l] {
                    up(c, l, root, lv, reaches, callers, seen, setters, wraps);
                }
            }
            return;
        }
        let can_own = x.own.as_ref().is_none_or(|o| o.keep == 0) && !others_read && (x.own.is_some() || !x.reads && x.one_end);
        if body && can_own {
            setters.insert(l);
        } else {
            wraps.insert((l, via));
        }
    }
    let mut lifted: BTreeMap<&str, u64> = BTreeMap::new();
    let mut setters: BTreeMap<&str, u64> = BTreeMap::new();
    let mut wraps: BTreeMap<(&str, &str), u64> = BTreeMap::new();
    for (c, x) in &lv {
        let Some(o) = &x.own else { continue };
        if o.keep != 0 || o.set == 0 || *c == root || set_calls.contains(c) || !callers.contains_key(c) {
            continue;
        }
        let (mut seen, mut s, mut w) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
        seen.insert(*c);
        for l in &callers[c] {
            up(l, c, root, &lv, &reaches, &callers, &mut seen, &mut s, &mut w);
        }
        // A level lifted itself, or set by another lift alike, keeps this one where it is.
        let clash = s.iter().chain(w.iter().map(|p| &p.0)).any(|l| lifted.contains_key(l) || setters.get(l).is_some_and(|v| *v != o.set) || l == c);
        let passed: Vec<&str> = seen.iter().copied().filter(|l| l != c).collect();
        if clash || passed.iter().any(|p| lifted.contains_key(p) || setters.contains_key(p)) {
            continue;
        }
        lifted.insert(c, o.set);
        for l in s {
            setters.insert(l, o.set);
        }
        for p in w {
            wraps.insert(p, o.set);
        }
    }
    if std::env::var_os("RERAIL_BITLEX_LIFT").is_some() {
        eprintln!("asks lifted: {lifted:?}; set by {setters:?}; around {wraps:?}");
    }
    if lifted.is_empty() {
        return rules.to_owned();
    }
    let mut out = String::with_capacity(rules.len());
    let mut j = 0;
    while j < lines.len() {
        let Some((name, x)) = lv.iter().find(|(_, x)| x.at.0 == j) else {
            out.push_str(lines[j]);
            j += 1;
            continue;
        };
        let (a, b) = x.at;
        let mut skip: BTreeSet<usize> = BTreeSet::new();
        if lifted.contains_key(name)
            && let Some(o) = &x.own
        {
            skip.extend(o.lines.clone());
            skip.extend(o.restore);
        }
        let set = setters.get(name).copied();
        let old = x.own.as_ref().map_or(0, |o| o.set);
        out.push_str(lines[a]);
        if let (Some(v), None) = (set, &x.own) {
            let _ = std::fmt::Write::write_fmt(&mut out, format_args!("        // The context's questions (`asks`, decision 0703): set where its body starts, for every level it calls.\n        let level_asks = self.asks;\n        self.asks = level_asks & 0x0 | {v:#x};\n"));
        }
        for k in a + 1..=b {
            if skip.contains(&k) {
                continue;
            }
            let l = lines[k];
            if let Some(v) = set {
                if let Some(o) = &x.own {
                    if k == o.lines.end - 1 {
                        let _ = std::fmt::Write::write_fmt(&mut out, format_args!("        self.asks = level_asks & 0x0 | {v:#x};\n"));
                        continue;
                    }
                    if k == o.lines.start && l.trim().starts_with("// The ") {
                        out.push_str("        // The context's questions (`asks`, decision 0703): set where its body starts, for every level it calls.\n");
                        continue;
                    }
                }
                if x.own.is_none() && k == b - 1 && l == "        Ok(())\n" {
                    out.push_str("        self.asks = level_asks;\n");
                }
                // Its own reads: of its old value, constants.
                let mut t = l.to_owned();
                for bit in 0..64 {
                    let pat = format!("self.asks >> {bit} & 1 != 0");
                    if t.contains(&pat) {
                        t = t.replace(&pat, if old >> bit & 1 != 0 { "true" } else { "false" });
                    }
                }
                let mut gone = false;
                for (f, bit) in &askers {
                    let call = format!("self.{f}(true)?;");
                    if t.trim() == call {
                        if old >> bit & 1 == 0 {
                            gone = true;
                        } else {
                            t = t.replace(&call, &format!("self.{f}(false)?;"));
                        }
                    }
                }
                if !gone {
                    out.push_str(&t);
                }
                continue;
            }
            // A call its setting is set around.
            if let Some(f) = l.trim().strip_prefix("self.").and_then(|r| r.strip_suffix("()?;"))
                && let Some(v) = wraps.get(&(*name, f))
            {
                let ind = &l[..l.len() - l.trim_start().len()];
                let _ = std::fmt::Write::write_fmt(&mut out, format_args!("{ind}let asks = self.asks;\n{ind}self.asks = asks & 0x0 | {v:#x};\n{l}{ind}self.asks = asks;\n"));
                continue;
            }
            out.push_str(l);
        }
        j = b + 1;
    }
    simplify(&out)
}

/// `rules` with a constant read gone: `if false && … {} else if X` is `if X`, `true && X` is `X`.
fn simplify(rules: &str) -> String {
    let mut s = rules.replace("(true && ", "(").replace(" true && ", " ");
    let mut from = 0;
    while let Some(k) = s[from..].find("if false && ") {
        let at = from + k;
        let Some(open) = s[at..].find(" {\n").map(|o| at + o) else { break };
        let rest = &s[open + 3..];
        let ws = rest.len() - rest.trim_start_matches(' ').len();
        if rest[ws..].starts_with("} else if ") {
            let after = open + 3 + ws + "} else if ".len();
            s = format!("{}if {}", &s[..at], &s[after..]);
            from = at;
        } else {
            from = at + 1;
        }
    }
    s
}
