//! Decode Python literal runs and retain each constant's source byte start.
use super::core_rr::py_unescape;

pub(super) fn with_offsets<N: Copy>(parts: &[(N, &str)]) -> Vec<(String, N, u8, usize)> {
    let mut pieces: Vec<(String, N, u8, usize)> = Vec::new();
    let mut cur: Option<(String, N, usize)> = None;
    let mut any_f = false;
    for &(sid, t) in parts {
        let pre: String = t.chars().take_while(char::is_ascii_alphabetic).collect::<String>().to_lowercase();
        if pre.contains('b') || pre.contains('t') {
            return Vec::new();
        }
        let rest = &t[pre.len()..];
        let Some(q) = ["\"\"\"", "'''", "\"", "'"].into_iter().find(|q| rest.len() >= 2 * q.len() && rest.starts_with(q) && rest.ends_with(q)) else {
            return Vec::new();
        };
        let body = &rest[q.len()..rest.len() - q.len()];
        let raw = pre.contains('r');
        let dec = |x: &str| if raw { x.to_owned() } else { py_unescape(x) };
        let push = |cur: &mut Option<(String, N, usize)>, v: &str, offset: usize| match cur {
            Some((s, _, _)) => s.push_str(v),
            None => *cur = Some((v.to_owned(), sid, offset)),
        };
        if !pre.contains('f') {
            push(&mut cur, &dec(body), 0);
            continue;
        }
        any_f = true;
        let b = body.as_bytes();
        // Syntax delimiters are ASCII, so byte scanning preserves UTF-8 boundaries.
        let byte = |i: usize| pre.len() + q.len() + i;
        let mut run_start = 0;
        let mut i = 0;
        let mut run = Vec::new();
        while i < b.len() {
            let c = b[i];
            if c == b'\\' && !raw && i + 1 < b.len() {
                // An escape, kept whole for `py_unescape`: `\N{…}` holds braces that are not a field.
                let mut j = i + 2;
                if b[i + 1] == b'N' && b.get(i + 2) == Some(&b'{') {
                    while j < b.len() && b[j - 1] != b'}' {
                        j += 1;
                    }
                }
                run.extend_from_slice(&b[i..j.min(b.len())]);
                i = j;
            } else if (c == b'{' || c == b'}') && b.get(i + 1) == Some(&c) {
                run.extend_from_slice(if c == b'{' { b"{{" } else { b"}}" });
                i += 2;
            } else if c == b'{' {
                let lit = dec(std::str::from_utf8(&run).expect("literal byte boundary")).replace("{{", "{").replace("}}", "}");
                run.clear();
                // The field: an expression up to a depth-0 `!`, `:` or `}`.
                let mut j = i + 1;
                let mut depth = 0i32;
                let mut quote: Option<u8> = None;
                while j < b.len() {
                    let d = b[j];
                    if let Some(qc) = quote {
                        if d == qc {
                            quote = None;
                        }
                    } else if d == b'\'' || d == b'"' {
                        quote = Some(d);
                    } else if b"([{".contains(&d) {
                        depth += 1;
                    } else if b")]}".contains(&d) {
                        if depth == 0 {
                            break;
                        }
                        depth -= 1;
                    } else if depth == 0 && (d == b':' || d == b'!' && b.get(j + 1) != Some(&b'=')) {
                        break;
                    }
                    j += 1;
                }
                let expr = &body[i + 1..j.min(b.len())];
                let te = expr.trim_end();
                // `f"{x=}"`: the expression's text is part of the constant.
                let debug = te.ends_with('=') && !["==", "!=", "<=", ">="].iter().any(|o| te.ends_with(o));
                let text_start = byte(if lit.is_empty() && debug { i } else { run_start });
                let mut text = lit;
                if debug {
                    text.push_str(&expr);
                }
                if !text.is_empty() {
                    push(&mut cur, &text, text_start);
                }
                if let Some((v, at, offset)) = cur.take()
                    && !v.is_empty()
                {
                    pieces.push((v, at, 0, offset));
                }
                // A conversion, then a format spec up to the field's closing brace.
                if j < b.len() && b[j] == b'!' {
                    while j < b.len() && b[j] != b':' && b[j] != b'}' {
                        j += 1;
                    }
                }
                if j < b.len() && b[j] == b':' {
                    j += 1;
                    let mut spec = Vec::new();
                    let mut spec_start = j;
                    let mut depth = 0i32;
                    while j < b.len() {
                        let d = b[j];
                        if d == b'{' {
                            if depth == 0 && !spec.is_empty() {
                                pieces.push((dec(std::str::from_utf8(&spec).expect("format byte boundary")), sid, 1, byte(spec_start)));
                                spec.clear();
                            }
                            depth += 1;
                        } else if d == b'}' {
                            if depth == 0 {
                                break;
                            }
                            depth -= 1;
                            if depth == 0 { spec_start = j + 1; }
                        } else if depth == 0 {
                            spec.push(d);
                        }
                        j += 1;
                    }
                    if !spec.is_empty() {
                        pieces.push((dec(std::str::from_utf8(&spec).expect("format byte boundary")), sid, 1, byte(spec_start)));
                    }
                }
                i = j + 1;
                run_start = i;
            } else {
                run.push(c);
                i += 1;
            }
        }
        if !run.is_empty() {
            let lit = dec(std::str::from_utf8(&run).expect("literal byte boundary")).replace("{{", "{").replace("}}", "}");
            push(&mut cur, &lit, byte(run_start));
        }
    }
    if let Some((v, at, offset)) = cur.take()
        && (!any_f || !v.is_empty())
    {
        pieces.push((v, at, 0, offset));
    }
    pieces
}

