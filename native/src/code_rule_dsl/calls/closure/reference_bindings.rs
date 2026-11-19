//! The saved reference answer records the binding lookups that produced it.

use super::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::cell::RefCell;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
enum Table {
    Definition,
    Alias,
    Opaque,
    Module,
    Base,
    Subclass,
    ClassCandidates,
    ConstructorParameters,
}

type Read = (Table, String);

thread_local! {
    static READS: RefCell<Option<BTreeSet<Read>>> = const { RefCell::new(None) };
}

struct Trace;

impl Trace {
    fn begin() -> Self {
        READS.with_borrow_mut(|reads| *reads = Some(BTreeSet::new()));
        Self
    }

    fn take(&self) -> BTreeSet<Read> {
        READS.with_borrow_mut(|reads| reads.take().unwrap_or_default())
    }
}

impl Drop for Trace {
    fn drop(&mut self) {
        READS.with_borrow_mut(|reads| *reads = None);
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct StableLink {
    targets: Vec<(String, u32, u32)>,
    open: bool,
    followed: Vec<String>,
    reads: Vec<(Read, Option<[u8; 32]>)>,
}

impl Graph {
    fn note_binding(&self, table: Table, name: &str) {
        if self.references {
            READS.with_borrow_mut(|reads| {
                if let Some(reads) = reads {
                    reads.insert((table, name.to_owned()));
                }
            });
        }
    }

    pub fn lookup_definitions(&self, name: &Name) -> Option<&Vec<(u32, (u32, u8))>> {
        self.note_binding(Table::Definition, name.text());
        self.by_name.get(name)
    }

    pub fn lookup_aliases(&self, name: &Name) -> Option<&Vec<(u32, Name)>> {
        self.note_binding(Table::Alias, name.text());
        self.aliases.get(name)
    }

    pub fn lookup_opaque(&self, name: &Name) -> bool {
        self.note_binding(Table::Opaque, name.text());
        self.opaque.contains_key(name)
    }

    pub fn lookup_module(&self, name: &Name) -> Option<&Vec<(u32, Module)>> {
        self.note_binding(Table::Module, name.text());
        self.modules.get(name)
    }

    pub fn lookup_bases(&self, name: &Name) -> Option<&Vec<(u32, (Name, u8))>> {
        self.note_binding(Table::Base, name.text());
        self.supers.get(name)
    }

    pub fn lookup_subclasses(&self, name: &Name) -> Option<&Vec<Name>> {
        self.note_binding(Table::Subclass, name.text());
        self.subclasses.get(name)
    }

    pub fn lookup_class_candidates(&self, short: &str) -> Option<&Vec<Name>> {
        self.note_binding(Table::ClassCandidates, short);
        self.reference_classes.get(short)
    }

    pub fn constructor_parameters(&self, field: &str) -> Option<Vec<String>> {
        self.note_binding(Table::ConstructorParameters, field);
        crate::code_rule_dsl::calls::contracts::get(self.reference_contracts).constructor_names(field)
    }

    fn binding_stamp(&self, (table, text): &Read) -> Option<[u8; 32]> {
        fn digest(rows: impl Serialize) -> [u8; 32] {
            Sha256::digest(serde_json::to_vec(&rows).expect("binding rows serialize")).into()
        }

        if *table == Table::ConstructorParameters {
            return crate::code_rule_dsl::calls::contracts::get(self.reference_contracts).constructor_names(text).map(digest);
        }
        if *table == Table::ClassCandidates {
            return self.reference_classes.get(text).map(|classes| {
                digest(classes.iter().map(Name::text).collect::<Vec<_>>())
            });
        }
        let name = Name::get(Family::Python, text)?;
        Some(match table {
            Table::Definition => digest(self.by_name.get(&name)?.iter().map(|(slot, (local, flags))| {
                let file = self.slots[*slot as usize].as_ref().expect("binding owner");
                let def = file.summary.defs[*local as usize];
                (file.path.as_ref(), def.start, def.end, flags)
            }).collect::<Vec<_>>()),
            Table::Alias => digest(self.aliases.get(&name)?.iter().map(|(_, name)| name.text()).collect::<Vec<_>>()),
            Table::Opaque => self.opaque.contains_key(&name).then(|| digest(true))?,
            Table::Module => digest(self.modules.get(&name)?.iter().map(|(_, (stars, open))| {
                (stars.iter().map(Name::text).collect::<Vec<_>>(), open)
            }).collect::<Vec<_>>()),
            Table::Base => digest(self.supers.get(&name)?.iter().map(|(_, (target, kind))| {
                (target.text(), kind)
            }).collect::<Vec<_>>()),
            Table::Subclass => digest(self.subclasses.get(&name)?.iter().map(Name::text).collect::<Vec<_>>()),
            Table::ClassCandidates | Table::ConstructorParameters => unreachable!("lookups by unqualified text handled above"),
        })
    }

    pub fn resolve_reference_link(&self, name: &str) -> String {
        let trace = Trace::begin();
        let (link, followed) = self.link_followed(&Name::new(Family::Python, name));
        let reads = trace.take().into_iter().map(|read| {
            let stamp = self.binding_stamp(&read);
            (read, stamp)
        }).collect();
        let targets = link.defs.into_iter().map(|target| {
            let file = self.slots[target.file as usize].as_ref().expect("linked definition owner");
            let def = file.summary.defs[target.local as usize];
            (file.path.to_string(), def.start, def.end)
        }).collect();
        let followed = followed.into_iter().map(|name| name.text().to_owned()).collect();
        serde_json::to_string(&StableLink { targets, open: link.open, followed, reads }).expect("links serialize")
    }

    pub fn reference_link_is_current(&self, answer: &str) -> bool {
        serde_json::from_str::<StableLink>(answer).is_ok_and(|link| {
            link.reads.iter().all(|(read, stamp)| self.binding_stamp(read) == *stamp)
        })
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
