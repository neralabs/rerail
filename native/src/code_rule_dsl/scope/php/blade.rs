//! The variables Blade's directives define in a template's embedded program (`blade::view`): each
//! marker (an empty statement over the directive) does to the flow what the compiled directive
//! does (`blade::marker`).

use super::{Flowing, Outer, Var, Visitor};
use crate::code_rule_dsl::scope::{Binding, Form, Site, VALUE};
use crate::code_rule_dsl::value::NodeRef;
use ast_grep_core::{Doc, Node};

impl Visitor<'_, '_> {
    /// The state after a marker; any other empty statement leaves it as it is.
    pub(super) fn blade_marker<D: Doc>(
        &mut self,
        node: &Node<'_, D>,
        mut state: Flowing,
    ) -> Flowing {
        // A marker's text is the program's `;`; the directive is the file bytes it stands for.
        let Some(file) = crate::code_rule_dsl::origin::InFile::of(node) else {
            return state;
        };
        let Some(marker) = crate::code_rule_dsl::blade::marker(&file.text(node)) else {
            return state;
        };
        if marker.reads_all {
            self.flow.reads_all.insert(self.scope);
        }
        if marker.dynamic {
            Self::dynamic(&mut state);
        }
        if let (Some(name), Some(state)) = (marker.forget, &mut state) {
            // Blade unsets the variable and restores what it held before, if anything did.
            state
                .vars
                .entry(name.into())
                .or_insert_with(Var::undefined)
                .unknown = true;
        }
        if let Some(name) = &marker.define {
            self.bind_implicit(name, node, &mut state);
        }
        state
    }

    /// Binds `name` at a marker: Blade assigns it there, by a name the template does not spell.
    fn bind_implicit<D: Doc>(&mut self, name: &str, marker: &Node<'_, D>, state: &mut Flowing) {
        let range = marker.range();
        let id = match self.sites.get(&(range.start, range.end)) {
            Some(id) => *id,
            None => {
                let binding = Binding::new(
                    range.start,
                    VALUE,
                    Site {
                        form: Form::Assignment,
                        node: Some(NodeRef::of(marker)),
                        value: None,
                        annotation: None,
                    },
                );
                let scope = self.scope;
                let index = self.builder.table.scopes[scope as usize]
                    .bindings
                    .get(name)
                    .map_or(0, Vec::len);
                self.builder.bind(scope, name, binding);
                let id = self.bindings.len() as u32;
                self.bindings
                    .push((scope, name.into(), index, range.start as u32));
                self.sites.insert((range.start, range.end), id);
                id
            }
        };
        // The flow keys the binding by the marker's start, as `Binding::offset` holds it.
        self.assign(id, name.to_owned(), range.start, state, false, Outer::Keep);
    }
}
