//! The `rir` rows of tier 2's tables and of quoted strings, as the transducer's assembler
//! reads them.

use std::fmt::Write as _;

/// `bytes` as a quoted string: printable ASCII as itself, `"` and `\` escaped, the rest `\xNN`.
pub fn quote(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() + 2);
    out.push('"');
    for &byte in bytes {
        match byte {
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            0x20..=0x7e => out.push(byte as char),
            _ => {
                let _ = write!(out, "\\x{byte:02x}");
            }
        }
    }
    out.push('"');
    out
}

/// A target of a switch: `@PC`, or `refuse`.
fn switch_target(t: u16) -> String {
    if t == crate::stream_ir::REFUSE { "refuse".to_owned() } else { format!("@{t:04}") }
}

/// Switch table `at` (decision 0087): `switch N at OFFSET end TARGET`, then a row per run of
/// bytes with one target, `switch N 0xLO-0xHI TARGET`; bytes in no row refuse.
pub fn switch_rows(out: &mut String, at: usize, s: &crate::stream_ir::ByteSwitch) {
    let _ = writeln!(out, "  switch {at} at {} end {}", s.offset, switch_target(s.end));
    let mut b = 0usize;
    while b < 256 {
        let t = s.targets[b];
        let mut e = b;
        while e + 1 < 256 && s.targets[e + 1] == t {
            e += 1;
        }
        if t != crate::stream_ir::REFUSE {
            let _ = writeln!(out, "  switch {at} {b:#04x}-{e:#04x} {}", switch_target(t));
        }
        b = e + 1;
    }
    for &(w, t) in &s.words {
        let _ = writeln!(out, "  switch {at} word s{w} {}", switch_target(t));
    }
}

/// Tier 2's rules (`Words::rules`): `rule N @P`, each rule's first place.
/// The marks' words: `markword K sW` (`Words::mark_words`).
pub fn mark_word_rows(out: &mut String, mark_words: &[(u8, u16)]) {
    use std::fmt::Write as _;
    for &(k, s) in mark_words {
        let _ = writeln!(out, "  markword {k} s{s}");
    }
}

pub fn word_kind_rows(out: &mut String, word_kinds: &[(u16, u16)]) {
    use std::fmt::Write as _;
    for &(s, k) in word_kinds {
        let _ = writeln!(out, "  wordkind s{s} #{k}");
    }
}

pub fn extra_rows(out: &mut String, extras: &[(u8, u16)]) {
    use std::fmt::Write as _;
    for &(b, k) in extras {
        let _ = writeln!(out, "  tokenextra b{b} #{k}");
    }
}

pub fn token_mark_rows(out: &mut String, marks: &[crate::program::TokenMark]) {
    mark_rows(out, "tokenmark", marks);
}

/// The gap patterns (`Words::gap_patterns`), as the token marks' rows.
pub fn gap_pattern_rows(out: &mut String, patterns: &[crate::program::TokenMark]) {
    mark_rows(out, "gappattern", patterns);
}

fn mark_rows(out: &mut String, row: &str, marks: &[crate::program::TokenMark]) {
    use std::fmt::Write as _;
    for m in marks {
        let _ = write!(out, "  {row} {}{}{}", m.order, if m.refuses { " refuses" } else { "" }, if m.unless { " unless" } else { "" });
        for e in &m.elems {
            let _ = write!(out, " o{} k{:x}", e.offset, e.kinds);
            if e.rows != 0 {
                let _ = write!(out, " r{:x}", e.rows);
            }
            for s in &e.words {
                let _ = write!(out, " s{s}");
            }
            for s in &e.except {
                let _ = write!(out, " x{s}");
            }
            if e.not {
                out.push_str(" not");
            }
            if e.end {
                out.push_str(" end");
            }
            if e.run {
                out.push_str(" run");
            }
            if e.without != 0 {
                let _ = write!(out, " w{:x}", e.without);
            }
            if e.with != 0 {
                let _ = write!(out, " h{:x}", e.with);
            }
            // `starting "B"`: the first bytes' set, four words.
            if e.first != [0; 4] {
                let _ = write!(out, " f{:x}.{:x}.{:x}.{:x}", e.first[0], e.first[1], e.first[2], e.first[3]);
            }
            // A run's `ending` (Q342): its kinds, words and keywords.
            if let Some(x) = &e.ending {
                let _ = write!(out, " ending k{:x}", x.kinds);
                if x.rows != 0 {
                    let _ = write!(out, " r{:x}", x.rows);
                }
                for s in &x.words {
                    let _ = write!(out, " s{s}");
                }
                for s in &x.except {
                    let _ = write!(out, " x{s}");
                }
            }
            // A run's depths (Q339): each side's counter, then its kinds, words and keywords.
            for n in &e.nests {
                let _ = write!(out, " {}{} k{:x}", if n.open { "open" } else { "close" }, n.counter, n.test.kinds);
                if n.test.rows != 0 {
                    let _ = write!(out, " r{:x}", n.test.rows);
                }
                for s in &n.test.words {
                    let _ = write!(out, " s{s}");
                }
                for s in &n.test.except {
                    let _ = write!(out, " x{s}");
                }
            }
        }
        out.push('\n');
    }
}

pub fn count_rows(out: &mut String, counts: &[u16]) {
    use std::fmt::Write as _;
    for (at, m) in counts.iter().enumerate() {
        let _ = writeln!(out, "  tokencount {at} max {m}");
    }
}

pub fn local_check_rows(out: &mut String, checks: &[crate::program::LocalCheck]) {
    for c in checks {
        out.push_str(&format!("  localcheck {}\n", local_check_text(c)));
    }
}

/// A local check's words: `mM dD` and its ops (`localcheck`, `reclocal`); a token's check
/// first `t` (`LocalCheck::token`), and `rB` after `dD` where it reads `B` bytes back.
pub fn local_check_text(c: &crate::program::LocalCheck) -> String {
    use crate::program::LocalOp;
    use std::fmt::Write as _;
    let mut out = String::new();
    {
        if c.token {
            out.push_str("t ");
        }
        let _ = write!(out, "m{} d{}", c.mask, c.delay);
        if c.back != 0 {
            let _ = write!(out, " r{}", c.back);
        }
        for op in &c.ops {
            let _ = match *op {
                LocalOp::Bytes(s) => write!(out, " b{:x}.{:x}.{:x}.{:x}", s[0], s[1], s[2], s[3]),
                LocalOp::Const(p) => write!(out, " c{p:x}"),
                LocalOp::Within(d) => write!(out, " w{d}"),
                LocalOp::Not(a) => write!(out, " n{a}"),
                LocalOp::And(a, b) => write!(out, " a{a}.{b}"),
                LocalOp::Or(a, b) => write!(out, " o{a}.{b}"),
                LocalOp::Xor(a, b) => write!(out, " x{a}.{b}"),
                LocalOp::AndNot(a, b) => write!(out, " m{a}.{b}"),
                LocalOp::Prev(a, k) => write!(out, " p{a}.{k}"),
                LocalOp::Fill(a, b) => write!(out, " f{a}.{b}"),
                LocalOp::Start => write!(out, " s"),
                LocalOp::Last => write!(out, " l"),
                LocalOp::Ended => write!(out, " e"),
            };
        }
    }
    out
}

/// The grammar's names (`Words::names`): `name rule R "N"`, `name kind #K "N"`, `name token K "N"`,
/// `name mark K "N"`.
pub fn name_rows(out: &mut String, names: &crate::program::Names) {
    use std::fmt::Write as _;
    for (r, n) in &names.rules {
        let _ = writeln!(out, "  name rule {r} {}", quote(n.as_bytes()));
    }
    for (k, n) in &names.kinds {
        let _ = writeln!(out, "  name kind #{k} {}", quote(n.as_bytes()));
    }
    for (k, n) in &names.tokens {
        let _ = writeln!(out, "  name token {k} {}", quote(n.as_bytes()));
    }
    for (k, n) in &names.marks {
        let _ = writeln!(out, "  name mark {k} {}", quote(n.as_bytes()));
    }
}

pub fn rule_rows(out: &mut String, rules: &[u16]) {
    use std::fmt::Write as _;
    for (at, p) in rules.iter().enumerate() {
        let _ = writeln!(out, "  rule {at} @{p:04}");
    }
}

/// A reading's rows: `reading N #K as #M`, `reading N #K gone`, `reading N #K keep`,
/// `reading N field #F as #G`; then `provisional #F`; then `extra #K`, the extras' kinds.
pub fn reading_rows(out: &mut String, readings: &crate::stream_ir::Readings) {
    use std::fmt::Write as _;
    for (at, r) in readings.maps.iter().enumerate() {
        // A reading with no row would be lost: a kind row each has one (the compiler's).
        for &(k, m) in &r.kinds {
            let _ = match m {
                crate::stream_ir::KEEP => writeln!(out, "  reading {at} #{k} keep"),
                crate::stream_ir::GONE => writeln!(out, "  reading {at} #{k} gone"),
                m => writeln!(out, "  reading {at} #{k} as #{m}"),
            };
        }
        for &(f, g) in &r.fields {
            let _ = writeln!(out, "  reading {at} field #{f} as #{g}");
        }
        let _ = match r.root {
            None => Ok(()),
            Some(crate::stream_ir::GONE) => writeln!(out, "  reading {at} root gone"),
            Some(m) => writeln!(out, "  reading {at} root as #{m}"),
        };
    }
    for f in &readings.provisional {
        let _ = writeln!(out, "  provisional #{f}");
    }
    for k in &readings.extras {
        let _ = writeln!(out, "  extra #{k}");
    }
}
