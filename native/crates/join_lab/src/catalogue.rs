//! The shape catalogue (`shapes.yaml`, the profile's: `RERAIL_PROFILE`), read back: each shape's stages (their kind,
//! mode and input), the rows into and out of each, the joins it ran for. The file is the profile's own form
//! (`plan_base::profile::report`, `Shape::yaml`), read by its lines; nothing else writes it.

use std::collections::BTreeMap;

/// A stage as the catalogue gives it: its kind, its mode, its input's kind (with its view or arity).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stage {
    pub kind: String,
    pub mode: String,
    /// The input's form as written (`{kind: view, view: 14, parent: false}`), empty for none.
    pub input: String,
    /// Whether it is probed by a key (`keys:`).
    pub keyed: bool,
}

impl Stage {
    /// Its input's kind (`view`, `derived`, `own`, `axis`, `words`, `none`).
    pub fn input_kind(&self) -> &str {
        self.input.strip_prefix("{kind: ").and_then(|s| s.split([',', '}']).next()).unwrap_or("none")
    }

    /// The part it needs (`rerail_join_shapes::parts::part`).
    pub fn part(&self) -> Option<String> {
        rerail_join_shapes::parts::part(&self.kind, &self.mode, self.input_kind(), self.input_field("view").and_then(|v| v.parse().ok()), self.keyed)
    }

    /// Its input's field `name`, where it has one.
    pub fn input_field(&self, name: &str) -> Option<&str> {
        let at = self.input.find(&format!("{name}: "))? + name.len() + 2;
        self.input[at..].split([',', '}']).next().map(str::trim)
    }
}

/// A shape's entry.
#[derive(Clone, Debug, Default)]
pub struct Entry {
    pub name: String,
    pub runs: u64,
    pub head: String,
    pub stages: Vec<Stage>,
    /// Per stage, its rows in and out, over every run.
    pub rows: Vec<[u64; 2]>,
    /// The joins it ran for: each its head (`rule <key>`) and its runs.
    pub instances: Vec<(String, u64)>,
}

impl Entry {
    /// The parts its class needs: each stage's, then `Emit`.
    pub fn parts(&self) -> Vec<String> {
        self.stages.iter().filter_map(Stage::part).chain(std::iter::once("Emit".to_owned())).collect()
    }

    /// Its class's name, or the first part it lacks.
    pub fn class(&self) -> Result<&'static str, String> {
        match rerail_join_shapes::parts::key_of(&self.name).and_then(rerail_ukl_rust::join_program::classes::class_of) {
            Some(c) => Ok(c.name),
            // Its parts built, its class not generated from a capture of it (`join-lab classes`).
            None => Err(rerail_join_shapes::parts::missing(&self.parts()).unwrap_or("its class (join-lab classes)").to_owned()),
        }
    }
}

/// Every entry of `text`, by name.
pub fn parse(text: &str) -> BTreeMap<String, Entry> {
    let mut out = BTreeMap::new();
    let mut e: Option<Entry> = None;
    let mut in_instances = false;
    for line in text.lines() {
        if let Some(name) = line.strip_prefix("- shape: ") {
            if let Some(done) = e.take() {
                out.insert(done.name.clone(), done);
            }
            e = Some(Entry { name: name.trim().to_owned(), ..Entry::default() });
            in_instances = false;
            continue;
        }
        let Some(cur) = e.as_mut() else { continue };
        let t = line.trim_start();
        let indent = line.len() - t.len();
        if indent == 2 {
            in_instances = t == "instances:";
            if let Some(v) = t.strip_prefix("runs: ") {
                cur.runs = v.trim().parse().unwrap_or(0);
            } else if let Some(v) = t.strip_prefix("head: ") {
                cur.head = v.trim().to_owned();
            } else if let Some(v) = t.strip_prefix("rows_in_out: ") {
                cur.rows = pairs(v);
            }
        } else if in_instances && t.starts_with("- {") {
            let head = field(t, "head").unwrap_or_default();
            let runs = field(t, "runs").and_then(|r| r.parse().ok()).unwrap_or(0);
            cur.instances.push((head, runs));
        } else if let Some(k) = t.strip_prefix("- kind: ") {
            cur.stages.push(Stage { kind: k.trim().to_owned(), ..Stage::default() });
        } else if let Some(s) = cur.stages.last_mut() {
            if let Some(m) = t.strip_prefix("mode: ") {
                s.mode = m.trim().to_owned();
            } else if let Some(i) = t.strip_prefix("input: ") {
                s.input = i.trim().to_owned();
            } else if t.starts_with("keys: ") {
                s.keyed = true;
            }
        }
    }
    if let Some(done) = e {
        out.insert(done.name.clone(), done);
    }
    out
}

/// `[[a, b], [c, d]]`'s pairs.
fn pairs(v: &str) -> Vec<[u64; 2]> {
    let nums: Vec<u64> = v.split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).filter_map(|s| s.parse().ok()).collect();
    nums.chunks_exact(2).map(|p| [p[0], p[1]]).collect()
}

/// A flow map's field `name` (`{span: "…", head: "rule 1f", runs: 64}`), its quotes taken off.
fn field(t: &str, name: &str) -> Option<String> {
    let at = t.find(&format!("{name}: "))? + name.len() + 2;
    let rest = &t[at..];
    if let Some(q) = rest.strip_prefix('"') {
        let end = q.find('"')?;
        return Some(q[..end].to_owned());
    }
    Some(rest.split([',', '}']).next()?.trim().to_owned())
}

#[cfg(test)]
mod tests {
    #[test]
    fn an_entry_reads_back_as_the_profile_wrote_it() {
        let y = "# shapes\n- shape: s-28c54b9d365aab6f\n  runs: 128\n  slots: 12\n  head: [v3, v11:text]\n  stages:\n    - kind: atom\n      mode: join\n      span: 1\n      input: {kind: view, view: 14, parent: false}\n      binds: [[0, 2, 0]]\n    - kind: map\n      mode: join\n      input: {kind: none}\n  rows_in_out: [[128, 5824], [5824, 5824]]\n  instances:\n    - {span: \"wait/a; b\", head: \"rule 44b0f1b068d64aad\", runs: 64}\n- shape: s-1\n  runs: 2\n";
        let c = super::parse(y);
        let e = &c["s-28c54b9d365aab6f"];
        assert_eq!((e.runs, e.head.as_str(), e.stages.len()), (128, "[v3, v11:text]", 2));
        assert_eq!((e.stages[0].kind.as_str(), e.stages[0].mode.as_str(), e.stages[0].input_kind(), e.stages[0].input_field("view")), ("atom", "join", "view", Some("14")));
        assert_eq!(e.stages[1].input_kind(), "none");
        assert_eq!(e.rows, vec![[128, 5824], [5824, 5824]]);
        assert_eq!(e.instances, vec![("rule 44b0f1b068d64aad".to_owned(), 64)]);
        assert_eq!(c["s-1"].runs, 2);
        assert_eq!(e.parts(), vec!["ScanKind<doc>".to_owned(), "Arith".into(), "Emit".into()]);
        // Its parts built, its class not generated.
        assert_eq!(e.class(), Err("its class (join-lab classes)".to_owned()));
    }
}
