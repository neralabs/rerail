//! PHP's linking of a call graph's names: classes, traits and late static binding

use super::*;

/// PHP's linking of the call graph (`CallRules::link`).
pub(crate) trait PhpLinks {
    /// Links a PHP callee name. A namespace fallback (`\ns\f|\f`) is the namespaced function
    /// where the workspace declares one, else the global one. A method `\c.m` is, as PHP looks
    /// it up, `c`'s own, then an alias its trait adaptations give, then a used trait's (but those
    /// `insteadof` excludes), then its parent's, then an interface's (an abstract class's); an
    /// abstract method (a trait's) yields to a later step's concrete one. A
    /// late-bound method `\c.m^` is `\c.m` and, unless no override can replace that for a call in
    /// `c` (`sealed_in`), the method every class the workspace derives from `c` looks up. A name
    /// no definition gives (a builtin, a vendor class, an undefined method only `__call` could
    /// answer) has no edge.
    fn link_php(&self, name: &Name, hops: u32, followed: &mut Vec<Name>) -> Link;
    /// `method` (`\c.m`) late-bound: what `c` and every class derived from it look up.
    fn link_late(&self, method: &Name, hops: u32, followed: &mut Vec<Name>) -> Link;
    /// Whether no override replaces `def`, which `method` (`\c.m`) links to, for a call in `c`.
    /// A final method no derived class declares. A private one binds a call in the class that
    /// has it: `c`'s own, or one `c` imports from a trait (the copy is `c`'s), private as `c`'s
    /// adaptations leave it (`m as private`, `m as public`); not one `c` inherits, which PHP
    /// passes over for an override, nor a constructor, which PHP looks up on the created class.
    fn sealed_in(&self, class: &Name, method: &Name, def: DefRef) -> bool;
}

impl PhpLinks for Graph {
    /// Links a PHP callee name. A namespace fallback (`\ns\f|\f`) is the namespaced function
    /// where the workspace declares one, else the global one. A method `\c.m` is, as PHP looks
    /// it up, `c`'s own, then an alias its trait adaptations give, then a used trait's (but those
    /// `insteadof` excludes), then its parent's, then an interface's (an abstract class's); an
    /// abstract method (a trait's) yields to a later step's concrete one. A
    /// late-bound method `\c.m^` is `\c.m` and, unless no override can replace that for a call in
    /// `c` (`sealed_in`), the method every class the workspace derives from `c` looks up. A name
    /// no definition gives (a builtin, a vendor class, an undefined method only `__call` could
    /// answer) has no edge.
    fn link_php(&self, name: &Name, hops: u32, followed: &mut Vec<Name>) -> Link {
        let text = name.text();
        if let Some(base) = text.strip_suffix('^') {
            return self.link_late(&Name::new(name.family(), base), hops, followed);
        }
        if let Some((namespaced, global)) = text.split_once('|') {
            for candidate in [namespaced, global] {
                let candidate = Name::new(name.family(), candidate);
                let link = self.link_name(&candidate, hops + 1, followed);
                followed.push(candidate);
                if link.open || !link.defs.is_empty() {
                    return link;
                }
            }
            return Link::default();
        }
        if let Some(entries) = self.by_name.get(name) {
            let mut defs = entries
                .iter()
                .filter(|(_, (_, flags))| flags & CLASS == 0)
                .map(|(file, (local, _))| DefRef {
                    file: *file,
                    local: *local,
                })
                .collect::<Vec<_>>();
            if !defs.is_empty() {
                defs.sort_unstable_by_key(|def| self.path_order(*def));
                return Link { defs, open: false };
            }
        }
        let Some(class) = name.parent() else {
            return Link::default();
        };
        let method = &text[class.text().len() + 1..];
        let supers = |key: &Name, kind: u8| {
            self.supers
                .get(key)
                .into_iter()
                .flatten()
                .filter(move |(_, (_, of))| *of == kind)
                .map(|(_, (target, _))| target)
        };
        let member =
            |class: &Name| Name::new(class.family(), &format!("{}.{method}", class.text()));
        // Trait adaptations: `T::m as n`, `m as n`.
        let mut names = supers(name, php::ALIAS).cloned().collect::<Vec<_>>();
        for aliased in supers(name, php::ALIAS_TRAITS) {
            names.extend(supers(class, php::TRAIT).map(|used| {
                Name::new(
                    used.family(),
                    &format!("{}.{}", used.text(), aliased.text()),
                )
            }));
        }
        let excluded = supers(name, php::EXCLUDE).collect::<Vec<_>>();
        let mut steps = vec![names];
        steps.push(
            supers(class, php::TRAIT)
                .filter(|used| !excluded.contains(used))
                .map(member)
                .collect(),
        );
        steps.push(supers(class, php::PARENT).map(member).collect());
        steps.push(supers(class, php::INTERFACE).map(member).collect());
        // An abstract method (a trait's, an interface's) is what a later step's concrete one
        // implements: it is the answer only where none is.
        let mut declared = None;
        for names in steps {
            if names.is_empty() {
                continue;
            }
            let link = self.union(names, hops, followed);
            if link.open {
                return link;
            }
            if link.defs.is_empty() {
                continue;
            }
            if !link
                .defs
                .iter()
                .all(|def| self.flags(*def) & php::ABSTRACT != 0)
            {
                return link;
            }
            declared.get_or_insert(link);
        }
        declared.unwrap_or_default()
    }

    /// `method` (`\c.m`) late-bound: what `c` and every class derived from it look up.
    fn link_late(&self, method: &Name, hops: u32, followed: &mut Vec<Name>) -> Link {
        let mut link = self.link_name(method, hops + 1, followed);
        followed.push(method.clone());
        let Some(class) = method.parent() else {
            return link;
        };
        let sealed = !link.defs.is_empty()
            && link
                .defs
                .iter()
                .all(|def| self.sealed_in(class, method, *def));
        if link.open || sealed {
            return link;
        }
        let suffix = &method.text()[class.text().len()..];
        let mut seen = HashSet::from([class.clone()]);
        let mut pending = vec![class.clone()];
        while let Some(class) = pending.pop() {
            for (_, child) in self.children.get(&class).into_iter().flatten() {
                if !seen.insert(child.clone()) {
                    continue;
                }
                let name = Name::new(child.family(), &format!("{}{suffix}", child.text()));
                let found = self.link_name(&name, hops + 1, followed);
                followed.push(name);
                link.defs.extend(found.defs);
                link.open |= found.open;
                pending.push(child.clone());
            }
        }
        link.defs.sort_unstable_by_key(|def| self.path_order(*def));
        link.defs.dedup();
        link
    }

    /// Whether no override replaces `def`, which `method` (`\c.m`) links to, for a call in `c`.
    /// A final method no derived class declares. A private one binds a call in the class that
    /// has it: `c`'s own, or one `c` imports from a trait (the copy is `c`'s), private as `c`'s
    /// adaptations leave it (`m as private`, `m as public`); not one `c` inherits, which PHP
    /// passes over for an override, nor a constructor, which PHP looks up on the created class.
    fn sealed_in(&self, class: &Name, method: &Name, def: DefRef) -> bool {
        let file = self.slots[def.file as usize]
            .as_ref()
            .expect("a linked definition's file is linked");
        let declared = &file.summary.defs[def.local as usize];
        let name = file.summary.name(declared.name).to_ascii_lowercase();
        let owner = name.rsplit_once('.').map_or("", |(owner, _)| owner);
        let mut private = declared.flags & php::PRIVATE != 0;
        let mut final_method = declared.flags & php::FINAL != 0;
        let supers = |key: &Name| {
            self.supers
                .get(key)
                .into_iter()
                .flatten()
                .map(|(_, (target, kind))| (target, *kind))
        };
        let imported = owner != class.text()
            && supers(class).any(|(used, kind)| kind == php::TRAIT && used.text() == owner);
        if imported {
            for (target, kind) in supers(method) {
                // `T::m as private` adapts `T`'s `m`; `m as private` that of whichever trait has it.
                if target.text().contains('.') && target.text() != name {
                    continue;
                }
                match kind {
                    php::PRIVATE_AS => private = true,
                    php::VISIBLE_AS => private = false,
                    php::FINAL_AS => final_method = true,
                    _ => {}
                }
            }
        }
        if method.text().ends_with(".__construct") {
            final_method
        } else if imported || owner == class.text() {
            final_method || private
        } else {
            final_method && !private
        }
    }
}
