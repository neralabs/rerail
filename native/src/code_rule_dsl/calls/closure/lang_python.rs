//! Python's linking of a call graph's names: attributes, bases and the method resolution order

use super::*;

/// Python's linking of the call graph (`CallRules::link`).
pub(crate) trait PythonLinks {
    /// Links a Python callee name: what the name itself names (`member`), else the member of the
    /// longest prefix something names: an alias to follow, a class whose method resolution order
    /// gives the attribute, a function or value whose attribute is set at run time (unknown), or a
    /// module whose member this is. A late-bound name (`pkg.C.m^`, `pkg.C.^` for `cls()`) is the
    /// class's and every workspace subclass's.
    fn link_python(&self, name: &Name, hops: u32, followed: &mut Vec<Name>) -> Link;
    fn reference_field(
        &self,
        class: &Name,
        field: &str,
        member: &str,
        hops: u32,
        followed: &mut Vec<Name>,
    ) -> Link;
    fn reference_type_members(
        &self,
        caller: &Name,
        aliases: &[(u32, Name)],
        member: &str,
        hops: u32,
        followed: &mut Vec<Name>,
    ) -> Link;
    /// A late-bound name, `pkg.C.m^` (`self.m()`, `cls.m()`) or `pkg.C.^` (`cls()`): the class's
    /// method (instantiation) and every workspace subclass's. Unknown where a class whose base
    /// no name denotes, which may derive from `C`, defines the method.
    fn link_late_python(&self, name: &Name, hops: u32, followed: &mut Vec<Name>) -> Link;
    /// Attribute `attribute` of class `class`, as its method resolution order gives it; unknown
    /// where no workspace class of it defines the attribute (an instance's attribute, one set at
    /// run time, another module's class's).
    fn class_member(
        &self,
        class: &Name,
        attribute: &str,
        hops: u32,
        followed: &mut Vec<Name>,
    ) -> Link;
    fn reference_bases(&self, class: &Name, followed: &mut Vec<Name>) -> Vec<Name>;
    fn reference_order(&self, class: &Name, followed: &mut Vec<Name>) -> Vec<Name>;
    fn reference_method(
        &self,
        order: &[Name],
        attribute: &str,
        followed: &mut Vec<Name>,
    ) -> Option<Link>;
    fn reference_fq_class(&self, name: &Name, hops: u32, followed: &mut Vec<Name>) -> Option<Name>;
    fn reference_type(
        &self,
        module: &str,
        short: &str,
        family: Family,
        hops: u32,
        followed: &mut Vec<Name>,
    ) -> Option<Name>;
}

impl PythonLinks for Graph {
    /// Links a Python callee name: what the name itself names (`member`), else the member of the
    /// longest prefix something names: an alias to follow, a class whose method resolution order
    /// gives the attribute, a function or value whose attribute is set at run time (unknown), or a
    /// module whose member this is. A late-bound name (`pkg.C.m^`, `pkg.C.^` for `cls()`) is the
    /// class's and every workspace subclass's.
    fn link_python(&self, name: &Name, hops: u32, followed: &mut Vec<Name>) -> Link {
        if self.references
            && let Some((module, rest)) = name.text().split_once(".<type>.")
            && let Some((short, attribute)) = rest.split_once('.')
        {
            return self
                .reference_type(module, short, name.family(), hops, followed)
                .and_then(|class| {
                    self.reference_method(
                        &self.reference_order(&class, followed),
                        attribute,
                        followed,
                    )
                })
                .unwrap_or_default();
        }
        if self.references
            && let Some((class, rest)) = name.text().split_once(".<field>.")
            && let Some((field, member)) = rest.split_once('.')
        {
            return self.reference_field(
                &Name::new(name.family(), class),
                field,
                member,
                hops + 1,
                followed,
            );
        }
        if self.references
            && !name.text().contains(".<unqualified>.")
            && let Some((module, rest)) = name.text().split_once(".<bare>.")
            && let Some((short, fallback)) = rest.split_once("=>")
        {
            let declared = Name::new(name.family(), &format!("{module}.<declared>.{short}"));
            followed.push(declared.clone());
            if let Some(aliases) = self.aliases.get(&declared) {
                return self.union(
                    aliases.iter().map(|(_, target)| target.clone()).collect(),
                    hops + 1,
                    followed,
                );
            }
            let imported = Name::new(name.family(), &format!("{module}.<imported>.{short}"));
            followed.push(imported.clone());
            if let Some(aliases) = self.aliases.get(&imported) {
                return self.union(
                    aliases.iter().map(|(_, target)| target.clone()).collect(),
                    hops + 1,
                    followed,
                );
            }
            return if fallback.is_empty() {
                Link::default()
            } else {
                let fallback = Name::new(name.family(), fallback);
                followed.push(fallback.clone());
                self.link_python(&fallback, hops + 1, followed)
            };
        }
        if self.references
            && let Some((class, rest)) = name.text().split_once(".<unqualified>.")
            && let Some((method, fallback)) = rest.split_once("=>")
        {
            let own = Name::new(name.family(), &format!("{class}.{method}"));
            followed.push(own.clone());
            if self.by_name.get(&own).is_some_and(|definitions| {
                definitions.iter().any(|(_, (_, flags))| flags & CLASS == 0)
            }) {
                return self.member(&own, hops + 1, followed).unwrap_or_default();
            }
            return if fallback.is_empty() {
                Link::default()
            } else {
                let fallback = Name::new(name.family(), fallback);
                followed.push(fallback.clone());
                self.link_python(&fallback, hops + 1, followed)
            };
        }
        if self.references
            && let Some((class, attribute)) = name.text().split_once(".<self>.")
        {
            let class = Name::new(name.family(), class);
            let order = self.reference_order(&class, followed);
            if let Some(found) = self.reference_method(&order, attribute, followed) {
                return found;
            }
            // Composition can supply a method to a mixin through a sibling base.
            for child in self.subclasses.get(&class).into_iter().flatten() {
                followed.push(child.clone());
                for co_base in self.reference_bases(child, followed) {
                    if co_base != class {
                        let order = self.reference_order(&co_base, followed);
                        if let Some(found) = self.reference_method(&order, attribute, followed) {
                            return found;
                        }
                    }
                }
            }
            return Link::default();
        }
        if self.references
            && let Some((class, attribute)) = name.text().split_once(".<super>.")
        {
            let class = Name::new(name.family(), class);
            let order = self.reference_order(&class, followed);
            return self
                .reference_method(&order[1..], attribute, followed)
                .unwrap_or_default();
        }
        if name.text().ends_with('^') {
            if self.references {
                let declared = name.text().trim_end_matches('^').trim_end_matches('.');
                return self.link_python(&Name::new(name.family(), declared), hops + 1, followed);
            }
            return self.link_late_python(name, hops, followed);
        }
        if self.references {
            if let Some(entries) = self.by_name.get(name) {
                return Link {
                    defs: entries
                        .iter()
                        .map(|(file, (local, _))| DefRef {
                            file: *file,
                            local: *local,
                        })
                        .collect(),
                    open: false,
                };
            }
            if let Some((module, short)) = name.text().rsplit_once('.') {
                let imported = Name::new(name.family(), &format!("{module}.<imported>.{short}"));
                followed.push(imported.clone());
                if let Some(aliases) = self.aliases.get(&imported) {
                    return self.union(
                        aliases.iter().map(|(_, target)| target.clone()).collect(),
                        hops + 1,
                        followed,
                    );
                }
            }
        }
        if let Some(link) = self.member(name, hops, followed) {
            return link;
        }
        for prefix in name.lineage().skip(1) {
            let rest = &name.text()[prefix.text().len()..];
            if let Some(entries) = self.aliases.get(prefix) {
                let targets = entries
                    .iter()
                    .filter(|(_, target)| target != prefix)
                    // Declared reexports resolve symbols, not module-valued
                    // attributes. Direct module imports already name their
                    // target before linking; following this prefix would add
                    // a runtime alias relation absent from the reference graph.
                    .filter(|(_, target)| {
                        !self.references
                            || !self.modules.contains_key(target)
                            || self.by_name.contains_key(target)
                    })
                    .map(|(_, target)| {
                        Name::new(target.family(), &format!("{}{rest}", target.text()))
                    })
                    .collect::<Vec<_>>();
                if !targets.is_empty() {
                    return self.union(targets, hops, followed);
                }
            }
            if let Some(entries) = self.by_name.get(prefix) {
                let attribute = &rest[1..];
                if self.references
                    && entries.iter().all(|(_, (_, flags))| flags & CLASS != 0)
                    && let Some((field, member)) = attribute.split_once('.')
                {
                    return self.reference_field(prefix, field, member, hops + 1, followed);
                }
                if !attribute.contains('.')
                    && entries.iter().all(|(_, (_, flags))| flags & CLASS != 0)
                {
                    return self.class_member(prefix, attribute, hops, followed);
                }
                return OPEN_LINK;
            }
            if self.opaque.contains_key(prefix) {
                return OPEN_LINK;
            }
            if let Some(entries) = self.modules.get(prefix) {
                // A member of a workspace module that neither a definition, an alias nor another
                // binding gives: a submodule's. Only a star import can bind it to a definition,
                // and only an open summary can hide one.
                let stars = entries
                    .iter()
                    .flat_map(|(_, (stars, _))| stars.iter())
                    .map(|star| Name::new(star.family(), &format!("{}{rest}", star.text())))
                    .collect::<Vec<_>>();
                let starred = !stars.is_empty();
                let mut link = self.union(stars, hops, followed);
                let open = entries.iter().any(|(_, (_, open))| *open);
                link.open |= open || (starred && link.defs.is_empty());
                return link;
            }
        }
        // Outside the workspace: the standard library, installed packages, builtins.
        Link::default()
    }

    fn reference_field(
        &self,
        class: &Name,
        field: &str,
        member: &str,
        hops: u32,
        followed: &mut Vec<Name>,
    ) -> Link {
        let order = self.reference_order(class, followed);
        let names = crate::code_rule_dsl::calls::contracts::get(self.reference_contracts).constructor_names(field);
        for base in order {
            if let Some(names) = &names {
                for parameter in names {
                    let key = Name::new(
                        base.family(),
                        &crate::code_rule_dsl::calls::contracts::parameter_key(base.text(), parameter),
                    );
                    followed.push(key.clone());
                    if let Some(aliases) = self.aliases.get(&key) {
                        if aliases
                            .iter()
                            .all(|(_, target)| target.text() == crate::code_rule_dsl::calls::contracts::UNANNOTATED)
                        {
                            break;
                        }
                        return self.reference_type_members(class, aliases, member, hops, followed);
                    }
                }
            }
            let key = Name::new(base.family(), &format!("{}.{field}", base.text()));
            followed.push(key.clone());
            if let Some(aliases) = self.aliases.get(&key) {
                return self.reference_type_members(class, aliases, member, hops, followed);
            }
        }
        Link::default()
    }

    fn reference_type_members(
        &self,
        caller: &Name,
        aliases: &[(u32, Name)],
        member: &str,
        hops: u32,
        followed: &mut Vec<Name>,
    ) -> Link {
        let mut targets = Vec::new();
        for (_, target) in aliases {
            let class = if let Some(annotation) = target.text().strip_prefix("<annotation>.") {
                caller.parent().and_then(|module| {
                    self.reference_type(module.text(), annotation, target.family(), hops, followed)
                })
            } else {
                match self.class_of(target, hops + 1, followed) {
                    Some(Mro::Workspace(class)) => Some(class),
                    _ => None,
                }
            };
            if let Some(class) = class {
                targets.push(Name::new(
                    class.family(),
                    &format!("{}.{member}", class.text()),
                ));
            }
        }
        self.union(targets, hops + 1, followed)
    }

    /// A late-bound name, `pkg.C.m^` (`self.m()`, `cls.m()`) or `pkg.C.^` (`cls()`): the class's
    /// method (instantiation) and every workspace subclass's. Unknown where a class whose base
    /// no name denotes, which may derive from `C`, defines the method.
    fn link_late_python(&self, name: &Name, hops: u32, followed: &mut Vec<Name>) -> Link {
        let Some(class) = name.parent() else {
            return OPEN_LINK;
        };
        let text = name.text();
        let attribute = &text[class.text().len() + 1..text.len() - 1];
        let lookup = |class: &Name, followed: &mut Vec<Name>| {
            if attribute.is_empty() {
                self.instantiate(class, hops + 1, followed)
            } else {
                self.class_member(class, attribute, hops + 1, followed)
            }
        };
        let mut link = lookup(class, followed);
        let mut seen = HashSet::from([class.clone()]);
        let mut pending = vec![class.clone()];
        while let Some(next) = pending.pop() {
            for child in self.subclasses.get(&next).into_iter().flatten() {
                if seen.insert(child.clone()) {
                    link.extend(lookup(child, followed));
                    pending.push(child.clone());
                }
            }
        }
        let methods: &[&str] = if attribute.is_empty() {
            &["__new__", "__init__"]
        } else {
            &[attribute]
        };
        for orphan in &self.orphans {
            followed.push(orphan.clone());
            link.open |= methods.iter().any(|method| {
                Name::get(orphan.family(), &format!("{}.{method}", orphan.text())).is_some_and(
                    |member| {
                        self.by_name.contains_key(&member)
                            || self.aliases.contains_key(&member)
                            || self.opaque.contains_key(&member)
                    },
                )
            });
        }
        link.defs.sort_unstable_by_key(|def| self.path_order(*def));
        link.defs.dedup();
        link
    }

    /// Attribute `attribute` of class `class`, as its method resolution order gives it; unknown
    /// where no workspace class of it defines the attribute (an instance's attribute, one set at
    /// run time, another module's class's).
    fn class_member(
        &self,
        class: &Name,
        attribute: &str,
        hops: u32,
        followed: &mut Vec<Name>,
    ) -> Link {
        if self.references {
            let order = self.reference_order(class, followed);
            return self
                .reference_method(&order, attribute, followed)
                .unwrap_or_default();
        }
        self.mro(class, hops, followed)
            .and_then(|order| self.lookup(&order, attribute, hops, followed))
            .unwrap_or(OPEN_LINK)
    }

    // Declared references use a visited breadth-first hierarchy walk. Alias limits
    // constrain a single name resolution, independently of inheritance depth.
    fn reference_bases(&self, class: &Name, followed: &mut Vec<Name>) -> Vec<Name> {
        followed.push(class.clone());
        self.supers
            .get(class)
            .into_iter()
            .flatten()
            .filter(|(_, (_, kind))| *kind == BASE)
            .filter_map(
                |(_, (target, _))| match self.class_of(target, 0, followed) {
                    Some(Mro::Workspace(base)) => Some(base),
                    _ => None,
                },
            )
            .collect()
    }

    fn reference_order(&self, class: &Name, followed: &mut Vec<Name>) -> Vec<Name> {
        let mut order = Vec::new();
        let mut seen = HashSet::new();
        let mut pending = VecDeque::from([class.clone()]);
        while let Some(class) = pending.pop_front() {
            if !seen.insert(class.clone()) {
                continue;
            }
            let bases = self.reference_bases(&class, followed);
            order.push(class);
            pending.extend(bases);
        }
        order
    }

    fn reference_method(
        &self,
        order: &[Name],
        attribute: &str,
        followed: &mut Vec<Name>,
    ) -> Option<Link> {
        for class in order {
            let member = Name::new(class.family(), &format!("{}.{attribute}", class.text()));
            followed.push(member.clone());
            if let Some(defs) = self.by_name.get(&member) {
                let defs: Vec<_> = defs
                    .iter()
                    .filter(|(_, (_, flags))| flags & CLASS == 0)
                    .map(|(file, (local, _))| DefRef {
                        file: *file,
                        local: *local,
                    })
                    .collect();
                if !defs.is_empty() {
                    return Some(Link { defs, open: false });
                }
            }
        }
        None
    }

    fn reference_fq_class(&self, name: &Name, hops: u32, followed: &mut Vec<Name>) -> Option<Name> {
        if hops > MAX_HOPS {
            return None;
        }
        followed.push(name.clone());
        if let Some(entries) = self.by_name.get(name) {
            return entries
                .iter()
                .all(|(_, (_, flags))| flags & CLASS != 0)
                .then(|| name.clone());
        }
        let (module, short) = name.text().rsplit_once('.')?;
        let key = Name::new(name.family(), &format!("{module}.<imported>.{short}"));
        followed.push(key.clone());
        self.aliases
            .get(&key)?
            .iter()
            .find_map(|(_, target)| self.reference_fq_class(target, hops + 1, followed))
    }

    fn reference_type(
        &self,
        module: &str,
        short: &str,
        family: Family,
        hops: u32,
        followed: &mut Vec<Name>,
    ) -> Option<Name> {
        for kind in ["imported", "class"] {
            let key = Name::new(family, &format!("{module}.<{kind}>.{short}"));
            followed.push(key.clone());
            if let Some(aliases) = self.aliases.get(&key)
                && let Some(class) = aliases
                    .iter()
                    .find_map(|(_, target)| self.reference_fq_class(target, hops + 1, followed))
            {
                return Some(class);
            }
        }
        None
    }
}
