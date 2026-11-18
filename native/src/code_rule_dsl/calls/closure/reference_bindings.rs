//! Stable inputs and outputs of declared reference lookup. The rule phase derives the
//! transitive dependencies and retains the links.

use super::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

const GLOBAL_CLASSES: &str = "<global-class>";

/// Current binding observations, with the other observations each lookup can read.
pub struct ReferenceBindings {
    pub observations: Vec<(String, String)>,
    pub dependencies: Vec<(String, String)>,
    pub requests: Vec<(String, String)>,
}

fn namespace(name: &str) -> &str {
    name.split('.').next().unwrap_or(name).trim_end_matches('^')
}

fn reads(name: &str) -> impl Iterator<Item = &str> {
    // A syntactic reference can carry another fully qualified name after an encoded fallback.
    name.split("=>")
        .enumerate()
        .filter(|(index, name)| *index == 0 || !name.is_empty())
        .map(|(_, name)| name)
        .flat_map(|name| {
            [
                Some(namespace(name)),
                name.contains(".<base>.").then_some(GLOBAL_CLASSES),
            ]
            .into_iter()
            .flatten()
        })
}

fn namespace_key(namespace: &str) -> String {
    format!("namespace:{namespace}")
}

type BindingPoint = (&'static str, String);

fn point_key(table: &str, name: &str) -> String {
    format!("binding:{table}:{name}")
}

#[derive(Default)]
struct Namespace {
    observations: BTreeMap<BindingPoint, Vec<u8>>,
    dependencies: BTreeSet<String>,
}

impl Namespace {
    fn input(&mut self, table: &'static str, name: &str, rows: impl Serialize) {
        self.observations.insert(
            (table, name.to_owned()),
            serde_json::to_vec(&rows).expect("binding records serialize"),
        );
    }

    fn follow(&mut self, name: &str) {
        self.dependencies.extend(reads(name).map(str::to_owned));
    }

    fn signature(&self) -> String {
        let mut hash = Sha256::new();
        for ((table, name), rows) in &self.observations {
            for input in [table.as_bytes(), name.as_bytes(), rows.as_slice()] {
                hash.update((input.len() as u64).to_le_bytes());
                hash.update(input);
            }
        }
        crate::content_store::observe::hex(&hash.finalize().into())
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct StableLink {
    targets: Vec<(String, u32, u32)>,
    open: bool,
    followed: Vec<String>,
}

impl Graph {
    /// Find the index entries for an ordinary lookup that ends without following another
    /// name. Include missing entries, so a new definition or alias can replace the result.
    fn plain_reference_points(&self, name: &str, points: &mut BTreeSet<BindingPoint>) -> bool {
        if name.contains(['<', '>']) || name.ends_with('^') {
            return false;
        }
        let mut record = |table, text: &str| {
            points.insert((table, text.to_owned()));
            Name::get(Family::Python, text)
        };
        if record("definition", name).is_some_and(|name| self.by_name.contains_key(&name)) {
            return true;
        }
        if let Some((module, member)) = name.rsplit_once('.')
            && record("alias", &format!("{module}.<imported>.{member}"))
                .is_some_and(|name| self.aliases.contains_key(&name))
        {
            return false;
        }
        if record("alias", name).is_some_and(|name| self.aliases.contains_key(&name)) {
            return false;
        }
        if record("opaque", name).is_some_and(|name| self.opaque.contains_key(&name)) {
            return true;
        }
        let mut rest = name;
        while let Some((prefix, _)) = rest.rsplit_once('.') {
            if record("alias", prefix).is_some_and(|name| self.aliases.contains_key(&name))
                || record("definition", prefix).is_some_and(|name| self.by_name.contains_key(&name))
            {
                return false;
            }
            if record("opaque", prefix).is_some_and(|name| self.opaque.contains_key(&name)) {
                return true;
            }
            if let Some(entries) = record("module", prefix).and_then(|name| self.modules.get(&name))
            {
                return entries.iter().all(|(_, (stars, _))| stars.is_empty());
            }
            rest = prefix;
        }
        true
    }

    fn reference_points(&self, name: &str) -> Option<BTreeSet<BindingPoint>> {
        let mut points = BTreeSet::new();
        if let Some((module, rest)) = name.split_once(".<bare>.")
            && let Some((member, fallback)) = rest.split_once("=>")
            // The same syntax must reach the bare-name branch in the linker.
            && !module.contains(['<', '>']) && !member.contains(['<', '>'])
            && !fallback.contains(['<', '>'])
        {
            for kind in ["declared", "imported"] {
                let binding = format!("{module}.<{kind}>.{member}");
                let entries =
                    Name::get(Family::Python, &binding).and_then(|name| self.aliases.get(&name));
                points.insert(("alias", binding));
                if let Some(entries) = entries {
                    return entries
                        .iter()
                        .all(|(_, target)| self.plain_reference_points(target.text(), &mut points))
                        .then_some(points);
                }
            }
            return (fallback.is_empty() || self.plain_reference_points(fallback, &mut points))
                .then_some(points);
        }
        self.plain_reference_points(name, &mut points)
            .then_some(points)
    }

    pub fn reference_bindings(&self, names: &[Name]) -> ReferenceBindings {
        let mut buckets = BTreeMap::<String, Namespace>::new();
        let at = |name: &Name| namespace(name.text()).to_owned();
        let path = |slot: u32| {
            self.slots[slot as usize]
                .as_ref()
                .expect("binding owner")
                .path
                .as_ref()
        };
        for (name, entries) in &self.by_name {
            let rows = entries
                .iter()
                .map(|(slot, (local, flags))| {
                    let summary = &self.slots[*slot as usize]
                        .as_ref()
                        .expect("definition owner")
                        .summary;
                    let def = summary.defs[*local as usize];
                    (
                        path(*slot),
                        def.start,
                        def.end,
                        *flags,
                        summary.name(def.name),
                    )
                })
                .collect::<Vec<_>>();
            buckets
                .entry(at(name))
                .or_default()
                .input("definition", name.text(), rows);
        }
        for (name, entries) in &self.aliases {
            let bucket = buckets.entry(at(name)).or_default();
            let rows = entries
                .iter()
                .map(|(slot, target)| {
                    bucket.follow(target.text());
                    (path(*slot), target.text())
                })
                .collect::<Vec<_>>();
            bucket.input("alias", name.text(), rows);
        }
        for (name, entries) in &self.opaque {
            let rows = entries
                .iter()
                .map(|(slot, ())| path(*slot))
                .collect::<Vec<_>>();
            buckets
                .entry(at(name))
                .or_default()
                .input("opaque", name.text(), rows);
        }
        for (name, entries) in &self.modules {
            let bucket = buckets.entry(at(name)).or_default();
            let rows = entries
                .iter()
                .map(|(slot, (stars, open))| {
                    let stars = stars
                        .iter()
                        .map(|target| {
                            bucket.follow(target.text());
                            target.text()
                        })
                        .collect::<Vec<_>>();
                    (path(*slot), stars, open)
                })
                .collect::<Vec<_>>();
            bucket.input("module", name.text(), rows);
        }
        for (name, entries) in &self.supers {
            let bucket = buckets.entry(at(name)).or_default();
            let rows = entries
                .iter()
                .map(|(slot, (target, kind))| {
                    bucket.follow(target.text());
                    (path(*slot), target.text(), kind)
                })
                .collect::<Vec<_>>();
            bucket.input("base", name.text(), rows);
        }
        for (name, entries) in &self.subclasses {
            let bucket = buckets.entry(at(name)).or_default();
            let rows = entries
                .iter()
                .map(|child| {
                    bucket.follow(child.text());
                    child.text()
                })
                .collect::<Vec<_>>();
            bucket.input("subclass", name.text(), rows);
        }
        for (short, classes) in &self.reference_classes {
            let bucket = buckets.entry(GLOBAL_CLASSES.to_owned()).or_default();
            let rows = classes
                .iter()
                .map(|class| {
                    bucket.follow(class.text());
                    class.text()
                })
                .collect::<Vec<_>>();
            bucket.input("global class", short, rows);
        }
        let mut bindings = ReferenceBindings {
            observations: Vec::with_capacity(buckets.len()),
            dependencies: Vec::new(),
            requests: Vec::new(),
        };
        let mut points = BTreeSet::new();
        let mut use_namespaces = false;
        for name in names {
            if let Some(reads) = self.reference_points(name.text()) {
                bindings.requests.extend(
                    reads
                        .iter()
                        .map(|(table, entry)| (name.text().to_owned(), point_key(table, entry))),
                );
                points.extend(reads);
            } else {
                use_namespaces = true;
                bindings.requests.extend(
                    reads(name.text())
                        .map(|target| (name.text().to_owned(), namespace_key(target))),
                );
            }
        }
        for point in points {
            let (table, entry) = (point.0, &point.1);
            if let Some(rows) = buckets
                .get(namespace(entry))
                .and_then(|bucket| bucket.observations.get(&point))
            {
                let digest = crate::content_store::observe::hex(&Sha256::digest(rows).into());
                bindings
                    .observations
                    .push((point_key(table, entry), digest));
            }
        }
        if !use_namespaces {
            return bindings;
        }
        for (name, bucket) in buckets {
            let key = namespace_key(&name);
            bindings
                .observations
                .push((key.clone(), bucket.signature()));
            bindings.dependencies.extend(
                bucket
                    .dependencies
                    .into_iter()
                    .filter(|target| target != &name)
                    .map(|target| (key.clone(), namespace_key(&target))),
            );
        }
        bindings
    }

    pub fn resolve_reference_link(&self, name: &str) -> String {
        let (link, followed) = self.link_followed(&Name::new(Family::Python, name));
        let targets = link
            .defs
            .into_iter()
            .map(|target| {
                let file = self.slots[target.file as usize]
                    .as_ref()
                    .expect("linked definition owner");
                let def = file.summary.defs[target.local as usize];
                (file.path.to_string(), def.start, def.end)
            })
            .collect();
        let followed = followed
            .into_iter()
            .map(|name| name.text().to_owned())
            .collect();
        serde_json::to_string(&StableLink {
            targets,
            open: link.open,
            followed,
        })
        .expect("links serialize")
    }

    pub fn restore_reference_link(&self, text: &str) -> Option<(Link, Vec<Name>)> {
        let link: StableLink = serde_json::from_str(text).ok()?;
        let defs = link
            .targets
            .into_iter()
            .map(|(path, start, end)| {
                let &slot = self.paths.get(path.as_str())?;
                let file = self.slots[slot as usize].as_ref()?;
                let first = file.summary.defs.partition_point(|def| def.start < start);
                let local = file.summary.defs[first..]
                    .iter()
                    .take_while(|def| def.start == start)
                    .position(|def| def.end == end)?
                    + first;
                Some(DefRef {
                    file: slot,
                    local: local as u32,
                })
            })
            .collect::<Option<_>>()?;
        let followed = link
            .followed
            .iter()
            .map(|name| Name::new(Family::Python, name))
            .collect();
        Some((
            Link {
                defs,
                open: link.open,
            },
            followed,
        ))
    }
}
