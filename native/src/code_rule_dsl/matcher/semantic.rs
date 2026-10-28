use super::{
    Arguments, BindingTest, Binds, EmbeddedTest, FormatArgs, HostTest, Layout, ParseTest, Resolves,
    ValueTest,
};

/// Rerail's semantic predicates: the rule terms that read a file's scopes, bindings, values,
/// parsed literals, layout or call graph rather than its syntax alone. The structural rule
/// language holds them as one term (`AstMatcher::Semantic`) and delegates to their families.
#[derive(Clone)]
pub(in crate::code_rule_dsl) enum Semantic {
    /// `resolves`: what a name or attribute resolves to through the file's scopes.
    Resolves(Box<Resolves>),
    /// `resolves.binding`: sub-rules over the site, value and annotation of the node's binding.
    Binding(Box<BindingTest>),
    /// `binds`: filters and sub-rules over the binding whose name node the node is.
    Binds(Box<Binds>),
    /// `value`: predicates over the node's own value.
    Value(Box<ValueTest>),
    /// `arguments`: a call's arguments bound to signature slots.
    Arguments(Box<Arguments>),
    /// `parse`: a rule over the tree a sub-grammar parses from the node's literal.
    Parse(Box<ParseTest>),
    /// `host`: inside a parsed tree's rule, a host rule on the node the `parse` holds; on an
    /// embedded program's node, a rule of the file's own tree on its node spanning the bytes the
    /// program's node stands for (`embedded`).
    Host(Box<HostTest>),
    /// `embedded`: a rule over the nodes of the file's embedded program that stand for bytes the
    /// node spans.
    Embedded(Box<EmbeddedTest>),
    /// `capture` (inside `parse`): binds the node under this name.
    Capture(String),
    /// `formatArgs`: a format call's arguments no field consumes, or its fields no argument fills.
    FormatArgs(Box<FormatArgs>),
    /// `layout`: the node's lines, its first byte, and the lines to a neighbour.
    Layout(Box<Layout>),
    /// `calls` (Python): the call's callee reaches a definition the term's inner rule matches,
    /// through the workspace call graph's closure.
    Calls(Box<crate::code_rule_dsl::calls::CallsTest>),
}

/// Emit predicate-local rows without executing a nested user rule. For N's catalog backend
/// (`demand::driver::catalog::sql_backend`), which only tests run.
#[cfg(test)]
pub(crate) fn producer_facts<'tree, D: ast_grep_core::Doc>(
    source: &crate::code_rule_dsl::demand::driver::catalog::sql_backend::SemanticFactSource,
    node: ast_grep_core::Node<'tree, D>,
    env: &ast_grep_core::meta_var::MetaVarEnv<'tree, D>,
) -> crate::code_rule_dsl::transducer::semantic_facts::Facts<'tree, D> {
    use crate::code_rule_dsl::transducer::semantic_facts::{Facts, Target, slot};
    use crate::code_rule_dsl::demand::driver::catalog::sql_backend::SemanticFactSource;
    let mut facts = Facts::new();
    match source {
        SemanticFactSource::Binding { test, request } => {
            test.facts(node, env, &mut facts, *request)
        }
        SemanticFactSource::Binds { test, request } => test.facts(node, env, &mut facts, *request),
        SemanticFactSource::FormatArgs(test) => test.facts(env, &mut facts),
        SemanticFactSource::Arguments(arguments) => {
            let mut trial = std::borrow::Cow::Borrowed(env);
            super::UNKNOWN.set(false);
            if arguments.match_node(node, &mut trial).is_some() {
                facts.gate(1, Target::None);
                for (position, (name, _)) in arguments.bind.iter().enumerate() {
                    if let Some(target) = trial.get_match(name) {
                        facts.push(
                            slot::ARGUMENT,
                            position as i64,
                            1,
                            Target::Node(target.clone()),
                        );
                    }
                }
            } else if super::UNKNOWN.replace(false) {
                facts.gate(-1, Target::None);
            }
        }
        SemanticFactSource::Resolves(resolves) => {
            if resolves.holds(&node) {
                facts.gate(1, Target::None);
            }
        }
        SemanticFactSource::Layout(layout) => {
            let mut trial = std::borrow::Cow::Borrowed(env);
            super::UNKNOWN.set(false);
            if layout.match_node(node, &mut trial).is_some() {
                facts.gate(1, Target::None);
            } else if super::UNKNOWN.replace(false) {
                facts.gate(-1, Target::None);
            }
        }
        SemanticFactSource::Calls(_) => {}
    }
    facts
}

impl Semantic {
    pub(super) fn binds_nothing(&self) -> bool {
        match self {
            Self::Resolves(_) | Self::Value(_) | Self::Layout(_) | Self::Host(_) => true,
            Self::Binding(test) => test.sub_rules().all(super::AstMatcher::binds_nothing),
            Self::Binds(test) => test.sub_rules().all(super::AstMatcher::binds_nothing),
            Self::Arguments(arguments) => arguments.bind.is_empty(),
            Self::Parse(test) => test.exports.is_empty(),
            Self::Embedded(test) => test.exports.is_empty(),
            Self::Calls(test) => test.chain.is_none(),
            Self::FormatArgs(test) => {
                test.exports.is_empty()
                    && test
                        .unused
                        .as_ref()
                        .is_none_or(super::AstMatcher::binds_nothing)
            }
            Self::Capture(_) => false,
        }
    }

    pub(super) fn reads_values(&self, seen: &mut std::collections::BTreeSet<usize>) -> bool {
        match self {
            Self::Value(_)
            | Self::Arguments(_)
            | Self::Parse(_)
            | Self::FormatArgs(_)
            | Self::Calls(_)
            | Self::Binds(_) => true,
            Self::Host(test) => super::reads_values(&test.rule, seen),
            Self::Embedded(test) => test.rules().any(|rule| super::reads_values(rule, seen)),
            // PHP's run-time fallback of a function or constant name may be unknown.
            Self::Resolves(resolves) => flow(resolves.language),
            Self::Capture(_) | Self::Layout(_) => false,
            // PHP's flow may leave a read's binding, `latest` and `outer` unknown.
            Self::Binding(test) => {
                test.filter.reads_usage()
                    || flow(test.language)
                    || test.sub_rules().any(|rule| super::reads_values(rule, seen))
            }
        }
    }

    pub(super) fn match_node<'tree, D: ast_grep_core::Doc>(
        &self,
        node: ast_grep_core::Node<'tree, D>,
        env: &mut std::borrow::Cow<ast_grep_core::meta_var::MetaVarEnv<'tree, D>>,
    ) -> Option<ast_grep_core::Node<'tree, D>> {
        match self {
            Self::Calls(test) => match super::call::calls(test, &node, env.to_mut()) {
                Some(true) => Some(node),
                Some(false) => None,
                None => {
                    super::UNKNOWN.set(true);
                    None
                }
            },
            Self::Resolves(resolves) => resolves.holds(&node).then_some(node),
            Self::Binding(test) => test.match_node(node, env),
            Self::Binds(test) => test.match_node(node, env),
            Self::Arguments(arguments) => arguments.match_node(node, env),
            Self::Parse(test) => test.match_node(node, env),
            Self::Host(test) => test.match_node(node, env),
            Self::Embedded(test) => test.match_node(node, env),
            Self::FormatArgs(test) => test.match_node(node, env),
            Self::Capture(name) => {
                env.to_mut().insert(name, node.clone())?;
                Some(node)
            }
            Self::Layout(layout) => layout.match_node(node, env),
            Self::Value(test) => match test.holds(&node) {
                Some(true) => Some(node),
                Some(false) => None,
                None => {
                    super::UNKNOWN.set(true);
                    None
                }
            },
        }
    }
}

/// Whether `language`'s family reads the file's flow, which may leave a read unknown
/// (`ScopeRules::FLOW`).
fn flow(language: crate::code_rule_dsl::language::GrammarDialect) -> bool {
    use crate::code_rule_dsl::language::adapter::{Adapter, ScopeRules, with_adapter};
    with_adapter!(crate::code_rule_dsl::language::Family::of(language), A => <<A as Adapter>::Scope as ScopeRules>::FLOW)
}
