//! `kinds.tsv` (`rerail-engine kind-names`): each tape tag's kind and field ids and names, as the engine's grammars
//! number them.

use std::collections::HashMap;

/// A kind: its name and whether it is named.
pub type Kind = (String, bool);

/// `kinds.tsv` by id: (tag, id) to (name, named) and its public symbol, (tag, field id) to name. A kind's line is
/// `tag, id, name, named, public`; a field's `tag, "field id", name, ""`.
pub struct Ids {
    pub kinds: HashMap<(u8, u16), Kind>,
    /// Each kind's public symbol: the id a node of its name and kind has (0: none, a hidden symbol).
    pub public: HashMap<(u8, u16), u16>,
    pub fields: HashMap<(u8, u8), String>,
}

impl Ids {
    /// Every line of `kinds.tsv` (`rerail-engine kind-names`): a name escaped as the command writes it (`\\`,
    /// `\n`, `\r`, `\t`, `\0`: C's and Go's tokens hold a newline and a NUL). A line of another shape stops the run:
    /// a name read wrong would leave its codes out of a table.
    pub fn load() -> Ids {
        let (mut kinds, mut public, mut fields) = (HashMap::new(), HashMap::new(), HashMap::new());
        for (n, line) in include_str!("../kinds.tsv").lines().enumerate() {
            let c: Vec<&str> = line.split('\t').collect();
            let (tag, id, name) = match c[..] {
                [tag, id, name, ""] if id.starts_with("field ") => (tag, id, name),
                [tag, id, name, _, _] => (tag, id, name),
                _ => bad_line(n + 1, line),
            };
            let tag: u8 = tag.parse().unwrap_or_else(|_| bad_line(n + 1, line));
            let name = unescape(name).unwrap_or_else(|| bad_line(n + 1, line));
            if let Some(id) = id.strip_prefix("field ") {
                fields.insert((tag, id.parse().unwrap_or_else(|_| bad_line(n + 1, line))), name);
            } else {
                let id: u16 = id.parse().unwrap_or_else(|_| bad_line(n + 1, line));
                kinds.insert((tag, id), (name, c[3] == "1"));
                public.insert((tag, id), c[4].parse().unwrap_or_else(|_| bad_line(n + 1, line)));
            }
        }
        Ids { kinds, public, fields }
    }
}

/// A line of `kinds.tsv` of no shape `load` reads: the run stops.
fn bad_line(n: usize, line: &str) -> ! {
    panic!("kinds.tsv line {n}: {line:?}")
}

/// A name as `kind-names` escapes it, read back; `None` for an escape it does not write.
fn unescape(s: &str) -> Option<String> {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        out.push(match chars.next()? {
            '\\' => '\\',
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            '0' => '\0',
            _ => return None,
        });
    }
    Some(out)
}
