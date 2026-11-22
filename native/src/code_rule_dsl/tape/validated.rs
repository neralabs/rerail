//! One admission contract for restored matcher trees and direct relational columns.
//! The sink sees validated preorder nodes. No partially admitted result is published.
use super::*;

pub struct Node {
    pub id: u32,
    pub kind: u16,
    pub field: u16,
    pub flags: u8,
    pub alias: u16,
    pub start: u32,
    pub end: u32,
    pub size: u32,
    pub parent: u32,
    pub first_field: bool,
}

struct Open {
    id: u32,
    until: usize,
    start: u32,
    end: u32,
    fields: u64,
    child_end: Option<u32>,
}

/// Checks `image` against `source` and `grammar`, and gives `sink` each node in preorder with its classes, read
/// through the tape API in whichever format the image is held (decision 0293). Returns the source's digest and the
/// contextual aliases the nodes' `alias` numbers.
pub fn visit(
    source: &str,
    grammar: &Grammar,
    image: &TapeImage,
    budget: &impl Fn() -> bool,
    sink: impl FnMut(Node, &[u16]),
) -> Result<([u8; 32], Vec<(String, bool)>), String> {
    checked(budget)?;
    // The source's digest, returned: a tape decoded from the wire holds none of its own (`TapeImage::source_digest`).
    let digest = source_digest(source, budget)?;
    if image.language != grammar.lang
        || image.source_len != source.len()
        || image.source_digest.is_some_and(|stored| stored != digest)
    {
        return Err("tape grammar or source revision mismatch".into());
    }
    if let Some(c) = image.f15() {
        let count = c.node_count();
        let widths_valid = [c.widths[0], c.widths[1], c.widths[2] & !TAIL_FROM_START].iter().all(|width| matches!(width, 2 | 4));
        if !widths_valid
            || c.code().len() != count * 2
            || c.size().len() != count * c.size_width()
            || c.base().len() != count.div_ceil(CHECKPOINT_STRIDE) * 4
            || c.off().len() != count * c.off_width()
            || c.tail().len() != count * c.tail_width()
        {
            return Err("malformed v9np tape".into());
        }
        // Format 15's code slots: none past the count, and the unused ones zero.
        if c.codes.iter().any(|code| {
            usize::from(code.class_count) > MAX_NODE_SUPERTYPES || code.classes[usize::from(code.class_count)..].iter().any(|&unused| unused != 0)
        }) {
            return Err("invalid abstract node class entry".into());
        }
    }
    // A 16z tape is not walked: its reads hold every offset to its tape, its block and its source, whatever its bytes
    // state (decision 0463; `rerail_clif::reads`, and the host's reads written directly), so a malicious tape reads
    // or writes no memory it should not, and a walk here would decode every block of it (the user, 2026-11-21: "The
    // requirement is not that we validate every tape on every decompression, it's to stop a malicious tape from
    // reading or writing memory it shouldn't"). Its header, regions, pieces and presence are checked where it is read
    // in (`Tape16Z::from_bytes`); its codes are its language's table.
    if image.format() == super::formats::Format::F16Z {
        checked(budget)?;
        return Ok((digest, Vec::new()));
    }
    let aliases = rerail_tape_format::with_tape!(image.tape(), |t| visit_tape(&t, source, grammar, budget, sink))?;
    checked(budget)?;
    Ok((digest, aliases))
}

fn visit_tape<T: rerail_tape_format::tape::Tape>(
    t: &T,
    source: &str,
    grammar: &Grammar,
    budget: &impl Fn() -> bool,
    mut sink: impl FnMut(Node, &[u16]),
) -> Result<Vec<(String, bool)>, String> {
    let table = t.table();
    // Each code the tape uses, once, at its first node: its grammar entry checked, and its alias numbered among the
    // tape's. (A language's table lists every code its grammar allows; a tape uses a few.)
    let mut aliases: Vec<(String, bool)> = Vec::new();
    let mut alias_of: Vec<Option<u16>> = vec![None; table.len()];
    let mut admit = |number: usize| -> Result<u16, String> {
        if let Some(alias) = alias_of[number] {
            return Ok(alias);
        }
        let code = table.entry(number as u32);
        let (kind, field) = (code.kind, code.field);
        let error = code.flags & CODE_ERROR != 0;
        if code.flags & !(CODE_EXTRA | CODE_ERROR | CODE_MISSING) != 0
            || (error && (kind != ERROR_CODE || code.flags & CODE_MISSING != 0))
            || (!error
                && (usize::from(kind) >= grammar.kinds.kind_count()
                    || !grammar.kinds.kind_is_visible(kind)))
            || usize::from(field) > grammar.kinds.field_count()
            || (field != 0 && grammar.kinds.field_name(field).is_none())
            || code.alias.as_ref().is_some_and(|alias| alias.0.is_empty())
        {
            return Err("unknown v9np grammar code".into());
        }
        if code.classes.len() > MAX_NODE_SUPERTYPES
            || code.classes.windows(2).any(|pair| pair[0] >= pair[1])
            || code.classes.iter().any(|&class| {
                usize::from(class) >= grammar.kinds.kind_count()
                    || !grammar.kinds.kind_is_supertype(class)
            })
        {
            return Err("invalid abstract node class entry".into());
        }
        let alias = match code.alias {
            None => SENTINEL,
            Some(alias) => {
                let at = aliases.iter().position(|a| (a.0.as_str(), a.1) == alias).unwrap_or_else(|| {
                    aliases.push((alias.0.to_owned(), alias.1));
                    aliases.len() - 1
                });
                u16::try_from(at).ok().filter(|&at| at != SENTINEL).ok_or("too many contextual grammar aliases")?
            }
        };
        alias_of[number] = Some(alias);
        Ok(alias)
    };

    let count = t.len();
    if count == 0 || count > u32::MAX as usize {
        return Err("malformed v9np tape".into());
    }
    let mut open: Vec<Open> = Vec::new();
    let mut unwound = 0usize;
    let mut s = 0u64;
    for index in 0..count {
        if index % 256 == 0 {
            checked(budget)?;
        }
        let number = t.code(index) as usize;
        if number >= table.len() {
            return Err("unknown v9np code".into());
        }
        let alias = admit(number)?;
        let code = table.entry(number as u32);
        let field = code.field;
        let error = code.flags & CODE_ERROR != 0;
        let missing = code.flags & CODE_MISSING != 0;
        let kind = if error { ERROR_SYMBOL } else { code.kind };
        let until = t.subtree_end(index);
        if until <= index || until > count {
            return Err("invalid subtree boundary".into());
        }
        let size = (until - index) as u32;
        s = t.start_next(index, s);
        let e = t.end_with_start(index, s);
        let (start, end) = (u32::try_from(s).map_err(|_| "span outside source")?, u32::try_from(e).map_err(|_| "span end overflow")?);
        if end < start
            || (missing && (start != end || size != 1))
            || end as usize > source.len()
            || !source.is_char_boundary(start as usize)
            || !source.is_char_boundary(end as usize)
        {
            return Err("span outside source or UTF-8 boundary".into());
        }
        while open.last().is_some_and(|p| index >= p.until) {
            if unwound.is_multiple_of(256) {
                checked(budget)?;
            }
            unwound += 1;
            open.pop();
        }
        let mut first_field = false;
        let parent = if index == 0 {
            if field != 0 || size as usize != count {
                return Err("invalid tape root".into());
            }
            NONE
        } else {
            let p = open.last_mut().ok_or("orphan tape node")?;
            if start < p.start || end > p.end || until > p.until {
                return Err("invalid tape parent or span".into());
            }
            if p.child_end.is_some_and(|previous| previous > end) {
                return Err("tape siblings out of byte order".into());
            }
            p.child_end = Some(end);
            if field != 0 {
                let bit = 1_u64 << field;
                first_field = p.fields & bit == 0;
                p.fields |= bit;
            }
            p.id
        };
        let named = match &code.alias {
            None => grammar.kind_is_named(kind),
            Some(alias) => alias.1,
        };
        sink(
            Node {
                id: index as u32,
                kind,
                field,
                start,
                end,
                size,
                parent,
                first_field,
                alias,
                flags: u8::from(named)
                    | (u8::from(code.flags & CODE_EXTRA != 0) << 1)
                    | (u8::from(missing) << 2)
                    | (u8::from(error) << 3),
            },
            &code.classes,
        );
        open.push(Open {
            id: index as u32,
            until,
            start,
            end,
            fields: 0,
            child_end: None,
        });
    }
    Ok(aliases)
}
