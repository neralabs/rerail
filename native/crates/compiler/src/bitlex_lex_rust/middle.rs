//! Whether a generated tier 1 can lex from the middle: its product build reads the file only in its token pass,
//! and the blocks that pass makes as it reaches them. A whole-file pass (a mask made over every word, a refusal
//! asked of a whole mask, a count scanned over the file) has to finish before any chunk's tokens can be read, so a
//! chunk lexed from its own entry would make it whole again: no lexer meant to run from the middle may have one.
//!
//! `whole_file_passes` reads the generated Rust: it drops every item and statement under
//! `#[cfg(feature = "test-hooks")]` (a test build's stage 1, walk and checks), takes the functions the product
//! entry reaches, and names each whole-file pass in them.

/// The source with each string, byte string and char literal's contents blanked and each comment removed, lengths
/// kept: the braces and calls left are the code's.
pub(super) fn code_only(src: &str) -> Vec<u8> {
    let b = src.as_bytes();
    let mut out = b.to_vec();
    let mut i = 0;
    let blank = |out: &mut Vec<u8>, from: usize, to: usize| {
        for c in &mut out[from..to] {
            if *c != b'\n' {
                *c = b' ';
            }
        }
    };
    while i < b.len() {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'/') => {
                let e = b[i..].iter().position(|&c| c == b'\n').map_or(b.len(), |k| i + k);
                blank(&mut out, i, e);
                i = e;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                let e = src[i + 2..].find("*/").map_or(b.len(), |k| i + 2 + k + 2);
                blank(&mut out, i, e);
                i = e;
            }
            // A raw string: `r"…"`, `r#"…"#`, `br"…"`.
            b'r' if (i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_') || b[i - 1] == b'b') && matches!(b.get(i + 1), Some(b'"' | b'#')) => {
                let hashes = b[i + 1..].iter().take_while(|&&c| c == b'#').count();
                if b.get(i + 1 + hashes) != Some(&b'"') {
                    i += 1;
                    continue;
                }
                let close = format!("\"{}", "#".repeat(hashes));
                let from = i + 2 + hashes;
                let e = src[from..].find(&close).map_or(b.len(), |k| from + k + close.len());
                blank(&mut out, from, e - close.len());
                i = e;
            }
            b'"' => {
                let mut k = i + 1;
                while k < b.len() && b[k] != b'"' {
                    k += if b[k] == b'\\' { 2 } else { 1 };
                }
                blank(&mut out, i + 1, k.min(b.len()));
                i = k + 1;
            }
            // A char literal (`'{'`, `'\''`, `b'}'`), not a lifetime or a label (`'tokens:`).
            b'\'' => {
                let k = if b.get(i + 1) == Some(&b'\\') { b[i + 2..].iter().position(|&c| c == b'\'').map(|p| i + 2 + p) } else { (b.get(i + 2) == Some(&b'\'')).then_some(i + 2) };
                match k {
                    Some(k) => {
                        blank(&mut out, i + 1, k);
                        i = k + 1;
                    }
                    None => i += 1,
                }
            }
            _ => i += 1,
        }
    }
    out
}

/// The end of the item or statement at `i`: past its `;` or `,` at its own depth, or its block's `}` (and an `else`'s
/// blocks after it), or where its enclosing block closes.
fn extent(c: &[u8], mut i: usize) -> usize {
    let mut depth = 0i32;
    while i < c.len() {
        match c[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' => depth -= 1,
            b'}' => {
                depth -= 1;
                if depth < 0 {
                    return i;
                }
                if depth == 0 {
                    let mut k = i + 1;
                    while k < c.len() && c[k].is_ascii_whitespace() {
                        k += 1;
                    }
                    if c[k..].starts_with(b"else") {
                        i = k + 4;
                        continue;
                    }
                    if matches!(c.get(k), Some(b';' | b',')) {
                        return k + 1;
                    }
                    // `let x = { … }.f();`, a block that a statement goes on from.
                    if matches!(c.get(k), Some(b'.' | b'?')) {
                        i = k;
                        continue;
                    }
                    return i + 1;
                }
            }
            b';' | b',' if depth == 0 => return i + 1,
            _ => {}
        }
        i += 1;
    }
    c.len()
}

/// The code with every `#[cfg(feature = "test-hooks")]` item and statement taken out.
fn product(c: &[u8]) -> Vec<u8> {
    product_from(c, c)
}

/// `orig`'s bytes where `c` (its code, `code_only(orig)`, of its length) keeps them: every `#[cfg(feature =
/// "test-hooks")]` item and statement taken out.
fn product_from(c: &[u8], orig: &[u8]) -> Vec<u8> {
    const TAG: &[u8] = b"#[cfg(feature = \"test-hooks\")]";
    let mut out = Vec::with_capacity(c.len());
    let mut i = 0;
    while i < c.len() {
        // The tag's string is blanked in `c`: match its shape.
        if c[i..].starts_with(b"#[cfg(feature = \"") && c.len() >= i + TAG.len() && c[i + TAG.len() - 3..i + TAG.len()] == *b"\")]" {
            i = extent(c, i + TAG.len());
            continue;
        }
        out.push(orig[i]);
        i += 1;
    }
    out
}

/// `src`'s product build as text, its literals and comments kept: every `#[cfg(feature = "test-hooks")]` item and
/// statement taken out (as `whole_file_passes` reads it).
pub(super) fn product_text(src: &str) -> String {
    String::from_utf8(product_from(&code_only(src), src.as_bytes())).expect("cut at code bytes")
}

/// The product build's code of `src` (no literals, no comments, no `test-hooks` items or statements), and of it the
/// rest of the block that holds the first `at` past its start: a reader asks what the product reads after it.
pub(super) fn product_after(src: &str, at: &str) -> Option<String> {
    let c = product(&code_only(src));
    let text = String::from_utf8_lossy(&c);
    let k = text.find(at)? + at.len();
    let mut depth = 0i32;
    let end = (k..c.len())
        .find(|&i| {
            match c[i] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                _ => {}
            }
            depth < 0
        })
        .unwrap_or(c.len());
    Some(text[k..end].to_owned())
}

/// Each function's name and body (`fn NAME … { … }`), in a code with no literals.
fn functions(c: &[u8]) -> Vec<(String, std::ops::Range<usize>)> {
    let s = String::from_utf8_lossy(c);
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(k) = s[from..].find("fn ") {
        let at = from + k;
        from = at + 3;
        if at > 0 && (c[at - 1].is_ascii_alphanumeric() || c[at - 1] == b'_') {
            continue;
        }
        let name: String = s[at + 3..].chars().take_while(|ch| ch.is_alphanumeric() || *ch == '_').collect();
        if name.is_empty() {
            continue;
        }
        // Its body: the first `{` past its signature at depth 0, unless a `;` (a declaration) comes first.
        let mut i = at + 3 + name.len();
        let mut depth = 0i32;
        let open = loop {
            match c.get(i) {
                None => break None,
                Some(b'(' | b'[' | b'<') => depth += 1,
                Some(b')' | b']' | b'>') if !(c[i] == b'>' && c[i - 1] == b'-') => depth -= 1,
                Some(b'{') if depth == 0 => break Some(i),
                Some(b';') if depth == 0 => break None,
                Some(b'=') if depth == 0 && c.get(i + 1) != Some(&b'>') => break None,
                _ => {}
            }
            i += 1;
        };
        let Some(open) = open else { continue };
        let end = extent(c, open);
        out.push((name, open..end));
    }
    out
}

/// The whole-file passes in `src`'s product build, each `function: what`: none for a lexer that can run from the
/// middle.
pub fn whole_file_passes(src: &str) -> Vec<String> {
    let c = product(&code_only(src));
    let text = String::from_utf8_lossy(&c).into_owned();
    let fns = functions(&c);
    // The entry: `lex`, through a `kernel!`'s `= NAME, on BEST` where it is one.
    let mut roots: Vec<String> = vec!["lex".to_owned()];
    for (k, _) in text.match_indices("pub fn lex(") {
        let line = &text[k..text[k..].find('\n').map_or(text.len(), |e| k + e)];
        if let Some(rest) = line.split(") = ").nth(1).or_else(|| line.split("= ").nth(1)) {
            roots.extend(rest.split(|ch: char| !(ch.is_alphanumeric() || ch == '_')).filter(|w| !w.is_empty() && *w != "on").map(str::to_owned));
        }
    }
    // The functions reached from it: each named in a reached body.
    let mut reached: Vec<usize> = Vec::new();
    let mut todo: Vec<String> = roots;
    while let Some(n) = todo.pop() {
        for (j, (name, body)) in fns.iter().enumerate() {
            if *name != n || reached.contains(&j) {
                continue;
            }
            reached.push(j);
            let b = &text[body.clone()];
            for (m, _) in &fns {
                if !todo.contains(m) && !reached.iter().any(|&r| fns[r].0 == *m) && b.match_indices(m.as_str()).any(|(p, _)| {
                    let before = b[..p].chars().last().is_none_or(|ch| !(ch.is_alphanumeric() || ch == '_'));
                    let after = b[p + m.len()..].trim_start();
                    before && (after.starts_with('(') || after.starts_with("::") || after.starts_with(',') || after.starts_with(')'))
                }) {
                    todo.push(m.clone());
                }
            }
        }
    }
    // The whole-file helpers (a mask made of every word, a refusal of a whole mask), and the loops over the file.
    const HELPERS: &[&str] = &["fill(", "escaped(", "any(", "stage1_best(src)", "stage1_best(&src[..n])"];
    const LOOPS: &[&str] = &["in 0..nw", "(0..nw)", "in 0..n {", "(0..n)", ".iter().enumerate()"];
    let mut out = Vec::new();
    for &j in &reached {
        let (name, body) = &fns[j];
        if HELPERS.iter().any(|h| name == h.trim_end_matches('(')) {
            continue;
        }
        let b = &text[body.clone()];
        for h in HELPERS {
            for (p, _) in b.match_indices(h) {
                if b[..p].chars().last().is_none_or(|ch| !(ch.is_alphanumeric() || ch == '_')) && !b[..p].ends_with("fn ") {
                    let line = b[..p].matches('\n').count();
                    out.push(format!("{name}: `{}` (line {line} of the function)", h.trim_end_matches('(')));
                }
            }
        }
        for l in LOOPS {
            for (p, _) in b.match_indices(l) {
                let line = b[..p].matches('\n').count();
                out.push(format!("{name}: a loop `{l}` (line {line} of the function)"));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::whole_file_passes;

    /// Each lexer meant to run from the middle has no whole-file pass in its product build.
    #[test]
    fn the_lexers_meant_to_run_from_the_middle_have_no_whole_file_pass() {
        let lexers = [
            ("rr", include_str!("../../../rr_aot/src/tier1.rs")),
            ("json", include_str!("../../../json_aot/src/tier1.rs")),
            ("kotlin", include_str!("../../../kotlin_aot/src/tier1.rs")),
            ("python", include_str!("../../../python_aot/src/tier1.rs")),
        ];
        let found: Vec<String> = lexers.iter().flat_map(|(n, src)| whole_file_passes(src).into_iter().map(move |p| format!("{n}: {p}"))).collect();
        assert!(found.is_empty(), "whole-file passes in a product build:\n{}", found.join("\n"));
    }

    /// The check finds each kind of pass, and none under `test-hooks` or in a function the entry never reaches.
    #[test]
    fn a_whole_file_pass_is_found() {
        let src = r#"
pub fn lex(src: &[u8]) -> Lexed = lex_on, on lex_on_best;
unsafe fn lex_on(src: &[u8]) -> Lexed {
    let nw = n.div_ceil(64);
    // fill(&a, &b) in a comment
    let s = "any(&x) in a string, and a brace }";
    let c = '}';
    #[cfg(feature = "test-hooks")]
    let m = {
        let f = fill(&a, &b);
        for q in 0..nw { f[q] = 0; }
        f
    };
    #[cfg(feature = "test-hooks")]
    if any(&m) {
        panic!("x");
    } else {
        let _ = escaped(&m);
    }
    refused |= any(&t74);
    helper(src);
    'tokens: while b < nw {
        b += 1;
    }
}
fn helper(src: &[u8]) {
    for (q, &w) in m.iter().enumerate() {}
}
fn unreached(src: &[u8]) {
    let x = fill(&a, &b);
}
fn any(x: &[u64]) -> bool {
    x.iter().any(|&w| w != 0)
}
"#;
        let found = whole_file_passes(src);
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found[0].starts_with("lex_on: `any`") || found[1].starts_with("lex_on: `any`"), "{found:?}");
        assert!(found.iter().any(|f| f.starts_with("helper: a loop `.iter().enumerate()`")), "{found:?}");
    }
}
