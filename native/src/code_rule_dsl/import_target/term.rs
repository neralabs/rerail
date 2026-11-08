//! `importTarget`: the matcher term reading a specifier node's import or require target, as
//! eslint-plugin-n reads it.

use crate::code_rule_dsl::import_target::{self, ModuleType, Style, Target, fs::View};
use crate::code_rule_dsl::matcher::{Reading, matched_file_path};
use crate::code_rule_dsl::value::Value;
use ast_grep_core::{Doc, Node};
use std::sync::Arc;

/// `importTarget`'s conditions; each one set must hold.
#[derive(Clone)]
pub(crate) struct ImportTargetTest {
    /// Reads the node's constant value (getStringIfConstant, with no scope).
    pub(crate) reading: Reading,
    /// The style a target gets where no ancestor decides it: `import` for visitImport's targets,
    /// `require` for visitRequire's.
    pub(crate) via: Style,
    pub(crate) module_types: Option<Vec<ModuleType>>,
    pub(crate) builtin: Option<bool>,
    pub(crate) resolved: Option<bool>,
    pub(crate) failed: Option<bool>,
    pub(crate) extension_spelled: Option<bool>,
    pub(crate) published: Option<bool>,
    pub(crate) dev_dependency: Option<bool>,
    pub(crate) declared: Option<bool>,
    pub(crate) aliased: Option<bool>,
}

/// `ImportTarget#getModuleStyle`: the first ancestor that decides it (a call of an identifier
/// `require`; an import declaration, `type` for `import type`; an `export type … from`), else
/// `via`.
fn style<D: Doc>(node: &Node<'_, D>, via: Style) -> Style {
    let mut current = node.parent();
    while let Some(ancestor) = current {
        match ancestor.kind().as_ref() {
            "call_expression" => {
                let mut callee = ancestor.field("function");
                while let Some(inner) = callee
                    .as_ref()
                    .filter(|callee| callee.kind() == "parenthesized_expression")
                    .and_then(|callee| callee.child(1))
                {
                    callee = Some(inner);
                }
                if callee.is_some_and(|callee| {
                    callee.kind() == "identifier" && callee.text() == "require"
                }) {
                    return Style::Require;
                }
            }
            "import_statement" => {
                return if ancestor.children().any(|child| child.kind() == "type") {
                    Style::Type
                } else {
                    Style::Import
                };
            }
            "export_statement" if ancestor.children().any(|child| child.kind() == "type") => {
                return Style::Type;
            }
            _ => {}
        }
        current = ancestor.parent();
    }
    via
}

/// JavaScript's `String(value)` of a known primitive, as getStringIfConstant converts it.
fn constant_string(value: &Value) -> Option<String> {
    crate::code_rule_dsl::value::ecma::ops::to_string(value)
        .map(|units| String::from_utf16_lossy(&units))
}

impl ImportTargetTest {
    /// Whether the node is a target whose facts meet every condition: a node with no constant
    /// value names no target.
    pub(crate) fn holds<D: Doc>(&self, node: &Node<'_, D>) -> bool {
        if !self.reading.has_value(node) {
            return false;
        }
        let Some(specifier) = constant_string(&self.reading.value(node)) else {
            return false;
        };
        let name = import_target::strip_params(&specifier);
        if self
            .builtin
            .is_some_and(|builtin| import_target::is_builtin(name) != builtin)
        {
            return false;
        }
        let view = import_target::current_view().unwrap_or_else(empty_view);
        let Some(importer) = matched_file_path() else {
            return false;
        };
        let target = import_target::target(&view, &importer, &specifier, style(node, self.via));
        self.meets(&target, &view)
    }

    fn meets(&self, target: &Target, view: &View) -> bool {
        let fact = |condition: Option<bool>, fact: &dyn Fn() -> Option<bool>| {
            condition.is_none_or(|expected| fact() == Some(expected))
        };
        self.module_types
            .as_ref()
            .is_none_or(|types| types.contains(&target.module_type))
            && fact(self.resolved, &|| Some(target.file_path.is_some()))
            && fact(self.failed, &|| Some(target.error.is_some()))
            && fact(self.extension_spelled, &|| target.extension_spelled(view))
            && fact(self.published, &|| target.published(view))
            && fact(self.dev_dependency, &|| target.dev_dependency(view))
            && fact(self.declared, &|| target.declared(view))
            && fact(self.aliased, &|| Some(target.aliased(view)))
    }
}

/// The view of an evaluation that installed none: no files, no root.
fn empty_view() -> Arc<View> {
    Arc::new(View::new(Arc::default(), None))
}
