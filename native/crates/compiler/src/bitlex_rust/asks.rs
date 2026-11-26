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
