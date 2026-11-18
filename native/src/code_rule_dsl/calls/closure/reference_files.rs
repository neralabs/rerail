//! A placed file's reference summary and its rows for the binding index.

use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};

pub struct ReferenceFile {
    pub(super) summary: Arc<Summary>,
    pub(super) bindings: Contribution,
}

#[derive(Serialize, Deserialize)]
struct SavedFile {
    summary: String,
    definitions: BTreeMap<String, Vec<(u32, u8)>>,
    aliases: BTreeMap<String, Vec<String>>,
    opaque: BTreeMap<String, Vec<()>>,
    modules: BTreeMap<String, Vec<(Vec<String>, bool)>>,
    bases: BTreeMap<String, Vec<(String, u8)>>,
    children: BTreeMap<String, Vec<String>>,
}

fn save<T, U>(rows: HashMap<Name, Vec<T>>, convert: impl Fn(T) -> U) -> BTreeMap<String, Vec<U>> {
    rows.into_iter().map(|(name, values)| {
        (name.text().to_owned(), values.into_iter().map(&convert).collect())
    }).collect()
}

fn restore<T, U>(rows: BTreeMap<String, Vec<T>>, convert: impl Fn(T) -> U) -> HashMap<Name, Vec<U>> {
    rows.into_iter().map(|(name, values)| {
        (Name::new(Family::Python, &name), values.into_iter().map(&convert).collect())
    }).collect()
}

impl ReferenceFile {
    pub fn prepare(path: &str, source: &Summary, contracts: super::super::contracts::Key) -> String {
        rerail_prof::count("reference.binding_contributions", 1);
        let summary = source.as_references(contracts);
        let contribution = Contribution::of(path, &summary);
        let text = |name: Name| name.text().to_owned();
        let saved = SavedFile {
            summary: STANDARD.encode(summary.encode()),
            definitions: save(contribution.defs, |def| def),
            aliases: save(contribution.aliases, text),
            opaque: save(contribution.opaque, |()| ()),
            modules: save(contribution.modules, |(stars, open)| {
                (stars.iter().map(|name| name.text().to_owned()).collect(), open)
            }),
            bases: save(contribution.supers, |(name, kind)| (text(name), kind)),
            children: save(contribution.children, text),
        };
        serde_json::to_string(&saved).expect("reference file facts serialize")
    }

    pub fn decode(value: &str) -> Option<Self> {
        let saved: SavedFile = serde_json::from_str(value).ok()?;
        let summary = Summary::decode(&STANDARD.decode(saved.summary).ok()?, Family::Python)?;
        let name = |text: String| Name::new(Family::Python, &text);
        let bindings = Contribution {
            defs: restore(saved.definitions, |def| def),
            aliases: restore(saved.aliases, name),
            opaque: restore(saved.opaque, |()| ()),
            modules: restore(saved.modules, |(stars, open)| {
                (stars.into_iter().map(name).collect::<Arc<[_]>>(), open)
            }),
            supers: restore(saved.bases, |(base, kind)| (name(base), kind)),
            children: restore(saved.children, name),
        };
        Some(Self { summary: Arc::new(summary), bindings })
    }
}
