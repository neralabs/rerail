//! A string reader's tokens in the one pass (`read R { … }`, decision 0698): the loop reads a string as the lean
//! lexer's `string` reads one, from stop to stop of stage 1's masks, and writes its pieces as records, as lean's `push`
//! does: its opener (its prefix and its quote), a triple's two more quotes, its text, its escapes, its braces, its
//! closer or an interpolation's `{`. An interpolation's code is the loop's tokens; at its level 0, its `}` (a tail),
//! its spec's `:` and its conversion's `!` are the reader's again. The templates open, their brackets and their bounds
//! are the walk's (`block::Walk`'s level), kept by the loop.
//!
//! Each refusal is tested where its place is in the arm (`RefuseAt`), its mask stage 1's. Nothing an arm changes is
//! changed before its last pause: an arm read again finds the loop as it was.

use std::fmt::Write as _;

use ::bitlex::syntax::{Expr, RefuseAt, Refusal, StringReader};

use super::{Gen, ident};

/// The reader's code for the loop: its state, the pick at a template's level 0, its rows' reads (each the token's last
/// byte), the brackets of a template's code, and its checks' functions.
pub(super) struct ReaderCode {
    pub decl: String,
    /// Before the rows' pick: `rd_pick`, the row of a tail, a spec or a conversion (each its arm's place, `arm_of`, filled
    /// in once the arms are numbered).
    pub pick: String,
    /// Each reader row (by grammar row): what it reads, and its read.
    pub rows: Vec<(usize, String, String)>,
    /// The rows the pick takes (tail, spec, conversion), by grammar row; none the rows' pick takes.
    pub picked: Vec<usize>,
    /// The brackets of a template's code, in a token of code (`s` to `l`).
    pub level: String,
    /// The level's brackets: a row whose bytes may hold one counts them.
    pub level_bytes: [bool; 256],
    pub fns: String,
    /// The rows the arms read in the first's place, and the string's row's start.
    pub skip: Vec<usize>,
    pub head: (usize, Expr),
    /// After the token loop: what holds to the file's end.
    pub end: String,
}

const PAUSE: &str = "wait = (2 * made - b).min(nw);\n                    continue 'pass;";

impl Gen<'_> {
    /// The reader's code (`ReaderCode`), for rows `rows` (grammar row, kind, rule's regions), the counts that read the
    /// regions where they hold (`counts`: name, `up`, `down`, bound), and the check functions written so far (`fns`).
    pub(super) fn reader_code(&mut self, rows: &[(usize, String, Expr)], counts: &[(String, Expr, Expr, u32)], fns: &str) -> Result<ReaderCode, String> {
        let [rd] = self.readers.as_slice() else { return Err("tier 1 as Rust: more than one string reader is not written".into()) };
        let rd: StringReader = rd.clone();
        let x = self.x;
        let n = |s: &str| Expr::Name(s.to_owned());
        let or = |v: Vec<Expr>| v.into_iter().reduce(|a, b| Expr::Or(Box::new(a), Box::new(b))).unwrap_or(Expr::Empty);
        let quotes = rd.quotes.clone();
        let qs = self.val(&or(quotes.iter().map(|q| n(q)).collect()))?;
        let q_each: Vec<String> = quotes.iter().map(|q| self.val(&n(q))).collect::<Result<_, _>>()?;
        let q3_each: Vec<String> = quotes.iter().map(|q| self.val(&n(&format!("{q}3")))).collect::<Result<_, _>>()?;
        let q3c_each: Vec<String> = quotes.iter().map(|q| self.val(&n(&format!("{q}3_c")))).collect::<Result<_, _>>()?;
        let e = self.val(&n(&rd.escape))?;
        let flag_masks: Vec<String> = rd.flags.iter().map(|(f, _)| self.val(&n(&format!("{f}_q")))).collect::<Result<_, _>>()?;
        let flag_bit = |f: &str| rd.flags.iter().position(|(g, _)| g == f).map(|i| format!("{:#04x}", 1u8 << i));
        if rd.flags.len() > 7 {
            return Err("tier 1 as Rust: a string reader of more than 7 flags".into());
        }
        const TRIPLE: &str = "0x80";
        // Its rows: the string's (its regions no tail's), its tail's, its spec's, its conversion's.
        let interp = rd.interpolation.clone();
        let tail_kinds: Vec<String> = interp.as_ref().map(|i| vec![i.close_kind.clone()]).unwrap_or_default();
        // Each the first of its kind's rows (a row a region, as the walk's reader reads them); the others the arm reads.
        let mut head_row = None;
        let mut tail_row = None;
        let mut spec_row = None;
        let mut conv_row = None;
        let mut skip = Vec::new();
        let mut head_start = Expr::Empty;
        for (row, kind, start) in rows {
            let slot = if tail_kinds.contains(kind) {
                &mut tail_row
            } else if interp.as_ref().and_then(|i| i.spec.as_ref()).is_some_and(|(k, _)| k == kind) {
                &mut spec_row
            } else if interp.as_ref().and_then(|i| i.conversion.as_ref()).is_some_and(|(k, ..)| k == kind) {
                &mut conv_row
            } else {
                match &head_row {
                    Some((_, k)) if k != kind => return Err(format!("tier 1 as Rust: string reader {}: two strings' tokens, `{k}` and `{kind}`", rd.name)),
                    Some(_) => skip.push(*row),
                    None => {
                        // Its token's start, without the region it reaches (`Parts::reach`): the arm reads the region.
                        head_start = match start {
                            Expr::And(a, _) => (**a).clone(),
                            other => other.clone(),
                        };
                        head_row = Some((*row, kind.clone()));
                    }
                }
                continue;
            };
            if slot.is_some() {
                skip.push(*row);
            } else {
                *slot = Some(*row);
            }
        }
        let Some((head_row, head_kind)) = head_row else { return Err(format!("tier 1 as Rust: string reader {}: no string's row", rd.name)) };
        let piece = |of: &str, name: &str, closes: bool| -> Result<String, String> {
            super::piece_kind(x.stmts, x.row_kinds, of, name, closes).map(|k| k.to_string()).ok_or_else(|| format!("tier 1 as Rust: string reader {}: `{name}` of `{of}`, a piece of no one record kind", rd.name))
        };
        // Each of the string's tokens' pieces, of the string's token and of the tail's.
        let kinds_of = |of: &str| -> Result<[String; 7], String> {
            Ok([
                piece(of, &rd.text, false)?,
                piece(of, &rd.escape_kind, false)?,
                piece(of, &rd.escape_else, false)?,
                match &rd.braces {
                    Some(b) => piece(of, b, false)?,
                    None => String::new(),
                },
                piece(of, &rd.close, true)?,
                match &interp {
                    Some(i) => piece(of, &i.open_kind, true)?,
                    None => String::new(),
                },
                match &rd.more {
                    Some(m) if of == head_kind => piece(of, m, false)?,
                    _ => String::new(),
                },
            ])
        };
        let head_k = kinds_of(&head_kind)?;
        let tail_k = match &interp {
            Some(i) => Some(kinds_of(&i.close_kind)?),
            None => None,
        };
        // The check of an escape: its function, as tier 2's.
        let mut new_fns = String::new();
        let check = ident(&rd.escape_kind);
        if !fns.contains(&format!("fn check_{check}(")) {
            // Its grammar, as the reader's token's parts hold it (`P = E check else K`).
            let Some(body) = x.stmts.iter().find_map(|t| match t {
                ::bitlex::syntax::Stmt::Token { rule: Some(::bitlex::syntax::TokenRule::ReadParts(_, parts)), .. } => parts.checked.iter().find(|c| c.0 == rd.escape_kind).map(|c| &c.2),
                ::bitlex::syntax::Stmt::Check { kind, body } if *kind == rd.escape_kind => Some(body),
                _ => None,
            }) else {
                return Err(format!("tier 1 as Rust: string reader {}: no `check {}`", rd.name, rd.escape_kind));
            };
            new_fns.push_str(&crate::bitlex_check_rust::Checks::new(x.stmts, "tier 1 as Rust").check_fn(&check, &rd.escape_kind, body, false)?);
        }
        // The refusals' masks: each its `X`, each quote's where it names one (`Q…`).
        let mut tests: Vec<(Refusal, Vec<String>)> = Vec::new();
        for r in &rd.refusals {
            let masks = match &r.expr {
                None => Vec::new(),
                Some(e) if names_quote(e) => quotes.iter().map(|q| self.val(&each_quote(e, q))).collect::<Result<_, _>>()?,
                Some(e) => vec![self.val(e)?],
            };
            tests.push((r.clone(), masks));
        }
        let raw = match &rd.raw {
            Some(f) => Some(flag_bit(f).ok_or_else(|| format!("tier 1 as Rust: string reader {}: `unless {f}`, no flag", rd.name))?),
            None => None,
        };
        // A raw string's plain bytes (`raw backslashes`): no escape, nor what one steps over that its `X` names.
        let raw_plain = match tests.iter().find(|(r, _)| r.at == RefuseAt::RawBackslashes) {
            Some((r, _)) => {
                let x = r.expr.clone().unwrap_or(Expr::Empty);
                let esc = Expr::Call("escaped".into(), vec![::bitlex::syntax::Arg::E(n(&rd.escape))]);
                Some(self.val(&Expr::And(Box::new(Expr::Not(Box::new(n(&rd.escape)))), Box::new(Expr::Not(Box::new(Expr::And(Box::new(esc), Box::new(x)))))))?)
            }
            None => None,
        };
        // Its stops: its quotes and its escape; its interpolation's braces too where it interpolates.
        let stops = self.val(&or(quotes.iter().map(|q| n(q)).chain([n(&rd.escape)]).collect()))?;
        let (ob, cb, stops_f, format_bit) = match &interp {
            Some(i) => (self.val(&n(&i.open))?, self.val(&n(&i.close))?, self.val(&or(quotes.iter().map(|q| n(q)).chain([n(&rd.escape), n(&i.open), n(&i.close)]).collect()))?, flag_bit(&i.flag).ok_or("tier 1 as Rust: the interpolation's flag")?),
            None => (String::new(), String::new(), stops.clone(), String::new()),
        };
        // The refusals at a place, as a test: `cond` the flag's or the triple's, `at` its byte or bytes.
        let when = |r: &Refusal| -> Option<String> {
            match &r.when {
                Some((true, f)) if f == "triple" => Some(format!("fl & {TRIPLE} != 0")),
                Some((false, f)) if f == "triple" => Some(format!("fl & {TRIPLE} == 0")),
                Some((true, f)) => flag_bit(f).map(|b| format!("fl & {b} != 0")),
                Some((false, f)) => flag_bit(f).map(|b| format!("fl & {b} == 0")),
                None => None,
            }
        };
        // A refusal by a mask: stage 1's blocks past the token's may change it, so where it has more to make it makes
        // them first, and the token is read again (decision 0619). One by a count is the file's.
        let refuse = |why: &str| format!("                    // {why}\n                    if made < nw {{\n                        {PAUSE}\n                    }}\n                    refused = true;\n                    break 'tokens;\n");
        let refuse_now = |why: &str| format!("                    // {why}\n                    refused = true;\n                    break 'tokens;\n");
        let at_place = |place: RefuseAt, byte: &str, range: Option<(&str, &str)>| -> String {
            let mut out = String::new();
            for (r, masks) in tests.iter().filter(|(r, _)| r.at == place) {
                let mask = match masks.as_slice() {
                    [] => None,
                    [m] => Some(m.clone()),
                    each => Some(format!("[{}][usize::from(qi)]", each.iter().map(|m| format!("&{m}[..]")).collect::<Vec<_>>().join(", "))),
                };
                let hold = match (mask, range) {
                    (None, _) => "true".to_owned(),
                    (Some(m), Some((a, z))) => format!("any_in({}{m}, {a}, {z})", if m.starts_with('[') { "" } else { "&" }),
                    (Some(m), None) => format!("bit({}{m}, {byte})", if m.starts_with('[') { "" } else { "&" }),
                };
                let test = match when(r) {
                    Some(c) => format!("{c} && {hold}"),
                    None => hold,
                };
                let _ = write!(out, "                if {test} {{\n{}                }}\n", refuse(&r.why));
            }
            out
        };
        let pause_at = |i: &str| format!("                if {i} >= 64 * made && made < nw {{\n                    {PAUSE}\n                }}\n");
        // A piece after the record: its kind alone, as the lean lexer's string's pieces are no tokens of code (no word,
        // no marks): a lookahead over the tokens counts no `{` of an interpolation and no `,` of a string's text.
        let put = |a: &str, z: &str, k: &str| format!("{{\n                        let (a, z) = ({a}, {z});\n                        // SAFETY: room for a record a byte: the piece's first byte is after the record before's last.\n                        unsafe {{ out.add(len + 1 + more).write(Rec {{ start: a as u32, last: z as u32, code: {k} }}) }};\n                        more += 1;\n                    }}");
        let quote_of = |each: &[String]| -> String {
            if each.len() == 1 { format!("&{}", each[0]) } else { format!("[{}][usize::from(qi)]", each.iter().map(|m| format!("&{m}[..]")).collect::<Vec<_>>().join(", ")) }
        };
        let (qm, q3cm) = (quote_of(&q_each), quote_of(&q3c_each));
        // The text from `t0`, its pieces to its closer or an interpolation's `{`: `(c, cl, opened)`, the closer's first
        // and last byte; `seg` the region's first byte (its fill's).
        let text = |k: &[String; 7]| -> String {
            let mut t = String::new();
            let _ = writeln!(t, "                // Its text, from stop to stop (the lean lexer's `string_stop`): its quote, its escape{}.", if interp.is_some() { ", an interpolation's braces" } else { "" });
            let _ = write!(t, "                let mut at = t0;\n                let (c, cl, opened) = loop {{\n");
            let stops_here = if interp.is_some() { format!("if fl & {format_bit} != 0 {{ &{stops_f} }} else {{ &{stops} }}") } else { format!("&{stops}") };
            let _ = write!(t, "                    let stops: &[u64] = {stops_here};\n                    let Some(p) = first_bit(&stops[..made], at) else {{\n                        if made < nw {{\n                            {PAUSE}\n                        }}\n                        // A string open at the file's end.\n                        refused = true;\n                        break 'tokens;\n                    }};\n                    if p + 1 >= 64 * made && made < nw {{\n                        {PAUSE}\n                    }}\n");
            // An escape.
            let _ = writeln!(t, "                    if bit(&{e}, p) {{");
            if let Some(rb) = &raw {
                let _ = write!(t, "                        if fl & {rb} != 0 {{\n                            // A raw string reads no escape: its backslash keeps a quote or a backslash after it from closing it or escaping.\n                            at = p + 1 + usize::from(bit(&{qs}, p + 1) || bit(&{e}, p + 1));\n                            continue;\n                        }}\n");
            }
            let _ = write!(t, "                        if t0 < p {{\n                            {}\n                        }}\n", put("t0", "p - 1", &k[0]));
            let esc = at_place(RefuseAt::Escape, "p", None);
            t.push_str(&indent8(&esc));
            let _ = write!(t, "                        // By its check, else a `{}` of one byte (the lean lexer's `backslash`): a quote after it is text.\n                        match check_{check}(&src[p..]) {{\n                            Some(k) => {{\n                                {}\n                                at = p + k;\n                                t0 = at;\n                            }}\n                            None => {{\n                                {}\n                                t0 = p + 1;\n                                at = p + 1 + usize::from(bit(&{qs}, p + 1));\n                            }}\n                        }}\n                        continue;\n                    }}\n", rd.escape_else, put("p", "p + k - 1", &k[1]), put("p", "p", &k[2]));
            if interp.is_some() {
                // `{{` and `}}`: one brace each, paired from a run's first; an odd run's last `{` is an interpolation's.
                let pair = if k[3].is_empty() { String::new() } else { format!("                        for i in 0..(r + 1 - p) / 2 {{\n                            {}\n                        }}\n", put("p + 2 * i", "p + 2 * i + 1", &k[3])) };
                let alone = tests.iter().find(|(r, _)| r.at == RefuseAt::BraceAlone).map_or(String::new(), |(r, _)| format!("                        if (r + 1 - p) % 2 == 1 {{\n    {}                        }}\n", refuse(&r.why).replace("\n                    ", "\n                        ")));
                let _ = write!(t, "                    if bit(&{ob}, p) || bit(&{cb}, p) {{\n                        let open = bit(&{ob}, p);\n                        let r = run_last(if open {{ &{ob} }} else {{ &{cb} }}, p);\n                        if r + 1 >= 64 * made && made < nw {{\n                            {PAUSE}\n                        }}\n                        if !open {{\n{alone}                        }}\n                        if t0 < p {{\n                            {}\n                        }}\n{pair}                        if open && (r + 1 - p) % 2 == 1 {{\n                            break (r, r, true);\n                        }}\n                        at = r + 1;\n                        t0 = at;\n                        continue;\n                    }}\n", put("t0", "p - 1", &k[0]));
            }
            let _ = write!(t, "                    // A quote: its own closes it (a triple's, the last three of a run); another is text.\n                    if !bit({qm}, p) {{\n                        at = p + 1;\n                        continue;\n                    }}\n                    if fl & {TRIPLE} != 0 && !bit({q3cm}, p) {{\n                        at = p + 1;\n                        continue;\n                    }}\n                    if t0 < p {{\n                        {}\n                    }}\n                    break (p, if fl & {TRIPLE} != 0 {{ p + 2 }} else {{ p }}, false);\n                }};\n", put("t0", "p - 1", &k[0]));
            // Its place's refusals: its fill, its text, a raw string's backslashes.
            t.push_str(&at_place(RefuseAt::Fill, "", Some(("seg", "c - 1"))));
            t.push_str(&at_place(RefuseAt::Text, "", Some(("o1", "c - 1"))));
            if let (Some(rb), Some(rp)) = (&raw, &raw_plain) {
                let why = tests.iter().find(|(r, _)| r.at == RefuseAt::RawBackslashes).map(|(r, _)| r.why.clone()).unwrap_or_default();
                let _ = write!(t, "                if fl & {rb} != 0 && o1 < c && !any_in(&{rp}, o1, c - 1) {{\n{}                }}\n", refuse(&why));
            }
            let _ = write!(t, "                if cl + 1 >= 64 * made && made < nw {{\n                    {PAUSE}\n                }}\n");
            let close = if interp.is_some() { format!("if opened {{\n                    {}\n                }} else {{\n                    {}\n                }}", put("c", "c", &k[5]), put("c", "cl", &k[4])) } else { put("c", "cl", &k[4]) };
            let _ = writeln!(t, "                {close}");
            t
        };
        // The counts at an interpolation's `{` and a tail's `}` (`i_at`, `o_t`: the reader's masks of them).
        let mut up = String::new();
        let mut down = String::new();
        for (name, u, d, max) in counts {
            let c = ident(name);
            if !super::reads_mask(u, "i_at", &self.defs) || !super::reads_mask(d, "o_t", &self.defs) {
                return Err(format!("tier 1 as Rust: count {name} reads a string reader's regions but not at its interpolation's `{{` and `}}`"));
            }
            let _ = write!(up, "                    if c_{c} == {max} {{\n                        refused = true;\n                        break 'tokens;\n                    }}\n                    c_{c} += 1;\n");
            let _ = writeln!(down, "                c_{c} = c_{c}.saturating_sub(1);");
        }
        let (nest, max, deep) = match (&interp, &self.reader_level) {
            (Some(_), Some(lv)) => (lv.nest.as_ref().map_or(1, |x| x.0), lv.max, lv.deep.clone()),
            (Some(_), None) => return Err("tier 1 as Rust: an interpolation with no level".into()),
            _ => (1, 0, String::new()),
        };
        let deep_why = tests.iter().find(|(r, _)| r.at == RefuseAt::Deep).map_or(deep.clone(), |(r, _)| r.why.clone());
        // An interpolation opened: the template around it waits (its nest), every template's frames (its bound); the
        // file is refused past either, before the loop's state changes.
        let r = refuse_now(&deep_why);
        let open = format!(
            "                if opened {{\n                    if rd_on && rd_depth + 1 >= {nest} || rd_total == {max} {{\n{r}                    }}\n                    if rd_on {{\n                        rd_around[rd_depth] = (rd_q, rd_fl, rd_level);\n                        rd_depth += 1;\n                    }}\n                    rd_total += 1;\n                    (rd_on, rd_q, rd_fl, rd_level, rd_open) = (true, qi, fl, 0, c);\n                    if rd_code == usize::MAX {{\n                        rd_code = c + 1;\n                    }}\n{up}                }}\n"
        );
        let mut rows_out = Vec::new();
        // The string's row.
        {
            let mut h = String::new();
            let _ = write!(h, "{{\n                more = 0;\n                // Its quote: the first after its prefix's letters; its flags by its prefix (at its quote); a triple's.\n                let Some(q) = first_bit(&{qs}[..made], s) else {{\n                    if made < nw {{\n                        {PAUSE}\n                    }}\n                    refused = true;\n                    break 'tokens;\n                }};\n");
            h.push_str(&pause_at("q + 2"));
            let qi = if q_each.len() == 1 { "0u8".to_owned() } else { q_each.iter().enumerate().skip(1).fold("0u8".to_owned(), |acc, (i, m)| format!("if bit(&{m}, q) {{ {i}u8 }} else {{ {acc} }}")) };
            let flags: Vec<String> = flag_masks.iter().enumerate().map(|(i, m)| format!("u8::from(bit(&{m}, q)) << {i}")).chain(std::iter::once(format!("u8::from({}) << 7", q3_each.iter().map(|m| format!("bit(&{m}, q)")).collect::<Vec<_>>().join(" || ")))).collect();
            let _ = write!(h, "                let qi = {qi};\n                let fl = {};\n                let seg = q;\n                let o = if fl & {TRIPLE} != 0 {{ q + 2 }} else {{ q }};\n                let o1 = o + 1;\n", flags.join(" | "));
            h.push_str(&at_place(RefuseAt::Open, "q", None));
            if !head_k[6].is_empty() {
                let _ = write!(h, "                // A triple's two more quotes.\n                if fl & {TRIPLE} != 0 {{\n                    {}\n                }}\n", put("q + 1", "q + 2", &head_k[6]));
            }
            h.push_str("                let mut t0 = o1;\n");
            h.push_str(&text(&head_k));
            h.push_str(&open);
            h.push_str("                rd_end = cl;\n                q\n            }");
            rows_out.push((head_row, format!("read {}, as the lean lexer's `string` reads one", rd.name), h));
        }
        let mut picked = Vec::new();
        let mut pick = String::new();
        let mut level = String::new();
        let mut level_bytes = [false; 256];
        if let Some(i) = &interp {
            // An end's refusals, the code's since its `{`.
            let end = |h: &mut String, with_empty: bool| {
                h.push_str(&at_place(RefuseAt::End, "s", None));
                if with_empty {
                    for (r, _) in tests.iter().filter(|(r, _)| r.at == RefuseAt::EndEmpty) {
                        let _ = write!(h, "                if s == rd_open + 1 {{\n{}                }}\n", refuse(&r.why));
                    }
                    for (r, m) in tests.iter().filter(|(r, _)| r.at == RefuseAt::Code) {
                        let hold = m.first().map_or("true".to_owned(), |m| format!("any_in(&{m}, rd_code, s - 1)"));
                        let _ = write!(h, "                if rd_code != usize::MAX && {hold} {{\n{}                }}\n", refuse(&r.why));
                    }
                }
            };
            if let (Some(row), Some(tk)) = (tail_row, &tail_k) {
                let mut h = String::new();
                let _ = write!(h, "{{\n                more = 0;\n                // Its `}}`, then the string's text as the template's string reads it.\n                let (qi, fl, seg, o1) = (rd_q, rd_fl, s, s + 1);\n");
                h.push_str(&pause_at("s + 1"));
                end(&mut h, true);
                h.push_str("                let mut t0 = o1;\n");
                h.push_str(&text(tk));
                let _ = write!(h, "{down}                // The interpolation's code ended at its `}}`; a `{{` after it begins the next's.\n                rd_code = usize::MAX;\n                if opened {{\n                    rd_open = c;\n                    rd_code = c + 1;\n{up}                }} else {{\n                    // The string ended: the template around it is open again.\n                    (rd_on, rd_q, rd_fl, rd_level) = if rd_depth > 0 {{\n                        rd_depth -= 1;\n                        let (q, f, v) = rd_around[rd_depth];\n                        (true, q, f, v)\n                    }} else {{\n                        (false, 0, 0, 0)\n                    }};\n                    rd_total = rd_total.saturating_sub(1);\n                }}\n                rd_end = cl;\n                rd_bare = true;\n                s\n            }}");
                rows_out.push((row, format!("`{}`, read {}'s, at its template's level 0", i.close, rd.name), h));
                picked.push(row);
            }
            if let (Some(row), Some((_, sp))) = (spec_row, &i.spec) {
                let mut h = String::new();
                h.push_str("{\n                more = 0;\n");
                h.push_str(&pause_at("s + 1"));
                end(&mut h, true);
                h.push_str(&at_place(RefuseAt::SpecStart, "s", None));
                let _ = write!(h, "                // To the byte before the interpolation's `{}` (none at its `{sp}`'s next).\n                let Some(e) = first_bit(&{cb}[..made], s + 2) else {{\n                    if made < nw {{\n                        {PAUSE}\n                    }}\n                    refused = true;\n                    break 'tokens;\n                }};\n                let c = e - 1;\n", i.close);
                h.push_str(&at_place(RefuseAt::Spec, "", Some(("s", "c - 1"))));
                h.push_str(&pause_at("c + 1"));
                h.push_str("                rd_code = usize::MAX;\n                c\n            }");
                rows_out.push((row, format!("`{sp}`, a format spec, at its template's level 0"), h));
                picked.push(row);
            }
            if let (Some(row), Some(_)) = (conv_row, &i.conversion) {
                let mut h = String::new();
                h.push_str("{\n                more = 0;\n");
                h.push_str(&pause_at("s + 2"));
                end(&mut h, false);
                h.push_str("                s + 1\n            }");
                rows_out.push((row, "a conversion, at its template's level 0".to_owned(), h));
                picked.push(row);
            }
            let (conv, spec_t) = (i.conversion.as_ref(), i.spec.as_ref());
            let arm = |r: Option<usize>| r.map_or(String::new(), arm_of);
            let mut tests_at: Vec<(String, String)> = vec![(format!("bit(&{cb}, s)"), arm(tail_row))];
            if let Some((_, sp)) = spec_t {
                tests_at.push((format!("bit(&{}, s)", self.val(&n(sp))?), arm(spec_row)));
            }
            if let Some((_, bang, l)) = conv {
                let ends = match spec_t {
                    Some((_, sp)) => format!("(bit(&{cb}, s + 2) || bit(&{}, s + 2))", self.val(&n(sp))?),
                    None => format!("bit(&{cb}, s + 2)"),
                };
                tests_at.push((format!("bit(&{}, s) && bit(&{}, s + 1) && {ends}", self.val(&n(bang))?, self.val(&n(l))?), arm(conv_row)));
            }
            let chain = tests_at.iter().map(|(t, r)| format!("if {t} {{\n                {r}\n            }}")).collect::<Vec<_>>().join(" else ");
            let _ = write!(pick, "        // A template open at its level 0 (the lean lexer's `interpolation`, at its count 0): its `{}`, its spec, its conversion are the string reader's.\n        let rd_pick = if rd_on && rd_level == 0 {{\n            if s + 2 >= 64 * made && made < nw {{\n                wait = (2 * made - b).min(nw);\n                continue 'pass;\n            }}\n            {chain} else {{\n                usize::MAX\n            }}\n        }} else {{\n            usize::MAX\n        }};\n", i.close);
            // The brackets of a template's code: the walk's level, every template's frames its bound.
            let Some(lv) = self.reader_level.clone() else { unreachable!("checked") };
            let brackets = Expr::Or(Box::new(lv.opens.clone()), Box::new(lv.closes.clone()));
            let both = self.val(&brackets)?;
            let (lo_v, lv_bytes) = (self.val(&lv.opens)?, self.bytes_at(&brackets, 0, &mut Default::default()).bools());
            level_bytes = lv_bytes;
            let _ = write!(level, "            // A template's code: its brackets, as the walk counts them (`{deep}`: past its bound, or a close at its level 0).\n            if rd_on {{\n                let mut q = s;\n                while let Some(i) = first_in(&{both}, q, l) {{\n                    if bit(&{lo_v}, i) {{\n                        if rd_total == {max} {{\n                            refused = true;\n                            break 'tokens;\n                        }}\n                        rd_level += 1;\n                        rd_total += 1;\n                    }} else if rd_level == 0 {{\n                        refused = true;\n                        break 'tokens;\n                    }} else {{\n                        rd_level -= 1;\n                        rd_total -= 1;\n                    }}\n                    q = i + 1;\n                }}\n            }}\n");
        }
        let decl = format!(
            "    // The string reader `{}` (decision 0698): the template open (its quote, its flags, its code's brackets, its `{{`), the\n    // templates around it, every template's frames, where the interpolation's code began; the pieces a string's arm\n    // writes after its record (its first), and the last byte of the last.\n    let (mut rd_on, mut rd_q, mut rd_fl, mut rd_level, mut rd_total, mut rd_open) = (false, 0u8, 0u8, 0u8, 0u8, 0usize);\n    let mut rd_around = [(0u8, 0u8, 0u8); {}];\n    let mut rd_depth = 0usize;\n    let mut rd_code = usize::MAX;\n    let (mut more, mut rd_end) = (0usize, usize::MAX);\n    // A tail's record (its `}}`): a piece of the string, no token of the records' pass (`stream::tokens`): no word, no\n    // marks, and the gap the next token's.\n    let mut rd_bare = false;\n",
            rd.name,
            nest.saturating_sub(1).max(1)
        );
        // At the file's end, an interpolation's code open to it: the code's refusals to the end.
        let mut end = String::new();
        for (r, m) in tests.iter().filter(|(r, _)| r.at == RefuseAt::Code) {
            if let Some(m) = m.first() {
                let _ = write!(end, "    // {}\n    if rd_code != usize::MAX && rd_code < n && any_in(&{m}, rd_code, n - 1) {{\n        refused = true;\n    }}\n", r.why);
            }
        }
        Ok(ReaderCode { decl, pick, rows: rows_out, picked, level, level_bytes, fns: new_fns, end, skip, head: (head_row, head_start) })
    }
}

/// Where row `row`'s arm's place goes, until the arms are numbered.
pub(super) fn arm_of(row: usize) -> String {
    format!("\u{0}arm {row}\u{0}")
}

/// Whether `e` names a quote's masks (`Q`, `Q3_c`: each quote's).
fn names_quote(e: &Expr) -> bool {
    match e {
        Expr::Name(m) => m.starts_with('Q'),
        Expr::Empty | Expr::Test { .. } => false,
        Expr::Not(a) => names_quote(a),
        Expr::And(a, b) | Expr::Or(a, b) | Expr::Xor(a, b) => names_quote(a) || names_quote(b),
        Expr::Call(_, args) => args.iter().any(|a| matches!(a, ::bitlex::syntax::Arg::E(x) if names_quote(x))),
    }
}

/// `e` with quote `q`'s masks for its `Q…`.
fn each_quote(e: &Expr, q: &str) -> Expr {
    let b = |x: &Expr| Box::new(each_quote(x, q));
    match e {
        Expr::Name(m) if m.starts_with('Q') => Expr::Name(format!("{q}{}", &m[1..])),
        Expr::Not(a) => Expr::Not(b(a)),
        Expr::And(x, y) => Expr::And(b(x), b(y)),
        Expr::Or(x, y) => Expr::Or(b(x), b(y)),
        Expr::Xor(x, y) => Expr::Xor(b(x), b(y)),
        Expr::Call(f, args) => Expr::Call(f.clone(), args.iter().map(|a| match a {
            ::bitlex::syntax::Arg::E(x) => ::bitlex::syntax::Arg::E(each_quote(x, q)),
            other => other.clone(),
        }).collect()),
        other => other.clone(),
    }
}

/// `s`'s lines four spaces deeper.
fn indent8(s: &str) -> String {
    s.lines().map(|l| if l.is_empty() { "\n".to_owned() } else { format!("    {l}\n") }).collect()
}
