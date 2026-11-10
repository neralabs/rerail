//! One admission contract for restored matcher trees and direct relational columns.
//! The sink sees validated preorder nodes. No partially admitted result is published.
use super::*;

pub(in crate::code_rule_dsl) struct Node {
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

pub(in crate::code_rule_dsl) fn visit(
    source: &str,
    grammar: &Grammar,
    image: &TapeImage,
    budget: &impl Fn() -> bool,
    mut sink: impl FnMut(Node, &[u16]),
) -> Result<(), String> {
    checked(budget)?;
    if image.language != grammar.lang
        || image.source_len != source.len()
        || image.source_digest != source_digest(&source, budget)?
    {
        return Err("tape grammar or source revision mismatch".into());
    }
    let count = image.node_count();
    let widths_valid = [
        image.widths[0],
        image.widths[1],
        image.widths[2] & !TAIL_FROM_START,
    ]
    .iter()
    .all(|width| matches!(width, 2 | 4));
    if count == 0
        || count > u32::MAX as usize
        || !widths_valid
        || image.code.len() != count * 2
        || image.size.len() != count * image.size_width()
        || image.base.len() != count.div_ceil(CHECKPOINT_STRIDE) * 4
        || image.off.len() != count * image.off_width()
        || image.tail.len() != count * image.tail_width()
    {
        return Err("malformed v9np tape".into());
    }
    for code in &image.codes {
        let kind = code.kf & 1023;
        let field = code.kf >> 10;
        let error = code.flags & CODE_ERROR != 0;
        if code.flags & !(CODE_EXTRA | CODE_ERROR | CODE_MISSING) != 0
            || (error && (kind != ERROR_CODE || code.flags & CODE_MISSING != 0))
            || (!error
                && (usize::from(kind) >= grammar.kinds.kind_count()
                    || !grammar.kinds.kind_is_visible(kind)))
            || usize::from(field) > grammar.kinds.field_count()
            || (field != 0 && grammar.kinds.field_name(field).is_none())
            || (code.alias != SENTINEL
                && image
                    .kind_aliases
                    .get(usize::from(code.alias))
                    .is_none_or(|alias| alias.0.is_empty()))
        {
            return Err("unknown v9np grammar code".into());
        }
        if usize::from(code.class_count) > MAX_NODE_SUPERTYPES
            || code.classes().windows(2).any(|pair| pair[0] >= pair[1])
            || code.classes[usize::from(code.class_count).min(MAX_NODE_SUPERTYPES)..]
                .iter()
                .any(|&unused| unused != 0)
            || code.classes().iter().any(|&class| {
                usize::from(class) >= grammar.kinds.kind_count()
                    || !grammar.kinds.kind_is_supertype(class)
            })
        {
            return Err("invalid abstract node class entry".into());
        }
    }

    let from_start = image.widths[2] & TAIL_FROM_START != 0;
    let mut open: Vec<Open> = Vec::new();
    let mut unwound = 0usize;
    for index in 0..count {
        if index % 256 == 0 {
            checked(budget)?;
        }
        let code = image
            .codes
            .get(usize::from(image.code_at(index)))
            .ok_or("unknown v9np code")?;
        let field = code.kf >> 10;
        let error = code.flags & CODE_ERROR != 0;
        let missing = code.flags & CODE_MISSING != 0;
        let kind = if error { ERROR_SYMBOL } else { code.kf & 1023 };
        let size = image.size_at(index);
        let until = index
            .checked_add(size as usize)
            .filter(|&end| size != 0 && end <= count)
            .ok_or("invalid subtree boundary")?;
        let start = image.start_at(index);
        let from = if from_start {
            start
        } else {
            image.start_at(until - 1)
        };
        let end = from
            .checked_add(image.tail_at(index))
            .ok_or("span end overflow")?;
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
        let named = if code.alias == SENTINEL {
            grammar.kind_is_named(kind)
        } else {
            image.kind_aliases[code.alias as usize].1
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
                alias: code.alias,
                flags: u8::from(named)
                    | (u8::from(code.flags & CODE_EXTRA != 0) << 1)
                    | (u8::from(missing) << 2)
                    | (u8::from(error) << 3),
            },
            code.classes(),
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
    checked(budget)
}
