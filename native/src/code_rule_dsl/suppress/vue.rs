//! `eslint-plugin-vue` 10.11.1 template comment directives, and the ESLint directives of a Vue
//! file's scripts.

use super::EslintDirectives;
use crate::code_rule_dsl::lines::LineTable;
use ast_grep_core::{Doc, Node};
use std::collections::HashSet;

/// The ESLint directives of the Vue file `node` belongs to: those of the comments in its
/// `<script>` blocks, the only ones vue-eslint-parser gives ESLint core (`getAllComments`); the
/// template's comments are eslint-plugin-vue's (`VueDirectives`), and its expressions' comments
/// neither's.
pub(crate) fn script_directives<D: Doc>(
    node: &Node<'_, D>,
    source: &str,
    lines: &LineTable,
) -> EslintDirectives {
    let root = super::root(node);
    let scripts = root
        .children()
        .filter(|child| child.kind() == "script_element")
        .map(|script| script.range())
        .collect::<Vec<_>>();
    let program = crate::code_rule_dsl::vue::program(&root);
    let comments = program
        .iter()
        .flat_map(|program| program.dfs())
        .filter(|node| {
            node.kind() == "comment"
                && scripts
                    .iter()
                    .any(|script| script.contains(&node.range().start))
        });
    EslintDirectives::from_comments(comments, source, lines)
}

/// A sorted event replayed by [`VueDirectives::suppresses`].
#[derive(Debug)]
struct Event {
    /// Byte at which the event takes effect.
    at: usize,
    /// Event operation.
    kind: EventKind,
}

/// The operations maintained by eslint-plugin-vue's processor states.
#[derive(Debug)]
enum EventKind {
    /// Disable all rules, or the listed rules, in the block state.
    BlockDisable(Vec<String>),
    /// Enable all rules, or the listed rules, in the block state.
    BlockEnable(Vec<String>),
    /// Disable all rules, or the listed rules, in the line state.
    LineDisable(Vec<String>),
    /// Enable all rules, or the listed rules, in the line state.
    LineEnable(Vec<String>),
    /// Clear both processor states at an element boundary.
    Clear,
}

/// eslint-plugin-vue 10.11.1's template comment directives and element boundaries.
#[derive(Debug, Default)]
pub(crate) struct VueDirectives {
    /// Directive and element-boundary events in source order.
    events: Vec<Event>,
}

impl VueDirectives {
    /// Reads HTML comment directives in the template and top-level document comments.
    pub(crate) fn read<D: Doc>(node: &Node<'_, D>, source: &str, lines: &LineTable) -> Self {
        let root = super::root(node);
        let mut events = Vec::new();
        for child in root.children() {
            let kind = child.kind();
            if kind == "comment" {
                add_comment(&mut events, child.range(), source, lines);
            } else if kind.ends_with("_element") || kind == "element" {
                events.push(Event {
                    at: child.range().end,
                    kind: EventKind::Clear,
                });
                if kind == "template_element" {
                    for descendant in child
                        .dfs()
                        .filter(|descendant| descendant.kind() == "comment")
                    {
                        add_comment(&mut events, descendant.range(), source, lines);
                    }
                }
            }
        }
        events.sort_by_key(|event| event.at);
        Self { events }
    }

    /// Whether a problem of one of `rules` at byte `at` is suppressed.
    pub(crate) fn suppresses(&self, rules: &[String], at: usize, _lines: &LineTable) -> bool {
        let mut block = State::default();
        let mut line = State::default();
        for event in self.events.iter().take_while(|event| event.at <= at) {
            match &event.kind {
                EventKind::BlockDisable(ids) => block.disable(ids),
                EventKind::BlockEnable(ids) => block.enable(ids),
                EventKind::LineDisable(ids) => line.disable(ids),
                EventKind::LineEnable(ids) => line.enable(ids),
                EventKind::Clear => {
                    block.clear();
                    line.clear();
                }
            }
        }
        block.matches(rules) || line.matches(rules)
    }
}

/// One of the two processor states.
#[derive(Debug, Default)]
struct State {
    /// Whether the state disables every rule.
    disable_all: bool,
    /// Rules disabled individually by this state.
    rules: HashSet<String>,
}

impl State {
    /// Applies a disable operation.
    fn disable(&mut self, ids: &[String]) {
        if ids.is_empty() {
            self.disable_all = true;
        } else {
            self.rules.extend(ids.iter().cloned());
        }
    }

    /// Applies an enable operation.
    fn enable(&mut self, ids: &[String]) {
        if ids.is_empty() {
            self.disable_all = false;
        } else {
            for id in ids {
                self.rules.remove(id);
            }
        }
    }

    /// Resets this state at an element boundary.
    fn clear(&mut self) {
        self.disable_all = false;
        self.rules.clear();
    }

    /// Checks whether this state suppresses any named rule.
    fn matches(&self, rules: &[String]) -> bool {
        self.disable_all || self.rules.iter().any(|rule| rules.contains(rule))
    }
}

/// Adds one HTML comment's directive events.
fn add_comment(
    events: &mut Vec<Event>,
    range: std::ops::Range<usize>,
    source: &str,
    lines: &LineTable,
) {
    let text = &source[range.clone()];
    if !text.starts_with("<!--") {
        return;
    }
    let Some(value) = text
        .strip_prefix("<!--")
        .and_then(|value| value.strip_suffix("-->"))
    else {
        return;
    };
    let Some((kind, ids)) = parse(value) else {
        return;
    };
    let first = lines.line(range.start);
    let last = lines.line(range.end);
    match kind {
        Kind::Disable => events.push(Event {
            at: range.start,
            kind: EventKind::BlockDisable(ids),
        }),
        Kind::Enable => events.push(Event {
            at: range.start,
            kind: EventKind::BlockEnable(ids),
        }),
        Kind::DisableLine if first == last => {
            let next = lines.full_end(first);
            events.push(Event {
                at: lines.start(first),
                kind: EventKind::LineDisable(ids.clone()),
            });
            events.push(Event {
                at: next,
                kind: EventKind::LineEnable(ids),
            });
        }
        Kind::DisableNextLine if first == last => {
            let target = lines.full_end(last);
            let after = if target == source.len() {
                target
            } else {
                lines.full_end(lines.line(target))
            };
            events.push(Event {
                at: target,
                kind: EventKind::LineDisable(ids.clone()),
            });
            events.push(Event {
                at: after,
                kind: EventKind::LineEnable(ids),
            });
        }
        Kind::DisableLine | Kind::DisableNextLine => {}
    }
}

/// The four directive forms recognized by eslint-plugin-vue 10.11.1.
#[derive(Clone, Copy)]
enum Kind {
    Disable,
    Enable,
    DisableLine,
    DisableNextLine,
}

/// Parses a directive value using eslint-plugin-vue's directive grammar.
fn parse(value: &str) -> Option<(Kind, Vec<String>)> {
    let text = strip_description(value).trim_start();
    let (kind, rest) = [
        ("eslint-disable-next-line", Kind::DisableNextLine),
        ("eslint-disable-line", Kind::DisableLine),
        ("eslint-disable", Kind::Disable),
        ("eslint-enable", Kind::Enable),
    ]
    .into_iter()
    .find_map(|(name, kind)| {
        text.strip_prefix(name)
            .filter(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
            .map(|rest| (kind, rest))
    })?;
    let mut ids = Vec::new();
    for token in rest
        .split(|ch: char| ch.is_whitespace() || ch == ',')
        .filter(|token| !token.is_empty())
    {
        ids.push(token.to_owned());
    }
    Some((kind, ids))
}

/// Removes eslint-plugin-vue's ` -- description` suffix from a directive.
fn strip_description(value: &str) -> &str {
    let bytes = value.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at].is_ascii_whitespace() {
            let start = at;
            while at < bytes.len() && bytes[at].is_ascii_whitespace() {
                at += 1;
            }
            let hyphens = at;
            while at < bytes.len() && bytes[at] == b'-' {
                at += 1;
            }
            if at - hyphens >= 2 && at < bytes.len() && bytes[at].is_ascii_whitespace() {
                return &value[..start];
            }
        } else {
            at += 1;
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::lines::Terminators;
    use ast_grep_core::AstGrep;

    fn suppressed(source: &str, rule: &str, marker: &str) -> bool {
        let tree = AstGrep::new(source, GrammarDialect::Vue);
        let root = tree.root();
        let lines = LineTable::new(source, Terminators::Eslint);
        let directives = VueDirectives::read(&root, source, &lines);
        let at = source.rfind(marker).expect("marker");
        directives.suppresses(&[rule.to_owned()], at, &lines)
    }

    #[test]
    fn block_disable_all() {
        assert!(suppressed(
            "<template><!-- eslint-disable --><p v-html=\"x\" /></template>",
            "vue/no-v-html",
            "v-html"
        ));
    }
    #[test]
    fn block_named_only_matching() {
        assert!(suppressed(
            "<template><!-- eslint-disable vue/no-v-html --><p v-html=\"x\" /></template>",
            "vue/no-v-html",
            "v-html"
        ));
    }
    #[test]
    fn block_named_does_not_match_other_rule() {
        assert!(!suppressed(
            "<template><!-- eslint-disable vue/no-v-html --><p v-html=\"x\" /></template>",
            "vue/no-unused-vars",
            "v-html"
        ));
    }
    #[test]
    // Verified with eslint-plugin-vue 10.11.1: Linter.verify returns no messages after
    // eslint-enable without rules, so the named disable remains active.
    fn enable_all_does_not_reenable_named() {
        assert!(suppressed(
            "<template><!-- eslint-disable vue/no-v-html --><!-- eslint-enable --><p v-html=\"x\" /></template>",
            "vue/no-v-html",
            "v-html"
        ));
    }
    #[test]
    fn enable_named_reenables() {
        assert!(!suppressed(
            "<template><!-- eslint-disable vue/no-v-html --><!-- eslint-enable vue/no-v-html --><p v-html=\"x\" /></template>",
            "vue/no-v-html",
            "v-html"
        ));
    }
    #[test]
    fn disable_line() {
        assert!(suppressed(
            "<template><p v-html=\"x\" /><!-- eslint-disable-line vue/no-v-html --></template>",
            "vue/no-v-html",
            "v-html"
        ));
    }
    #[test]
    fn disable_next_line() {
        assert!(suppressed(
            "<template><!-- eslint-disable-next-line vue/no-v-html -->\n<p v-html=\"x\" /></template>",
            "vue/no-v-html",
            "v-html"
        ));
    }
    #[test]
    fn multiline_line_comment_does_nothing() {
        assert!(!suppressed(
            "<template><!-- eslint-disable-line\n --><p v-html=\"x\" /></template>",
            "vue/no-v-html",
            "v-html"
        ));
    }
    #[test]
    fn template_disable_does_not_reach_script() {
        assert!(!suppressed(
            "<template><!-- eslint-disable --><p /></template><script>const vhtml = 1</script>",
            "vue/no-v-html",
            "vhtml"
        ));
    }
    #[test]
    // Verified with eslint-plugin-vue 10.11.1: Linter.verify returns no messages for this
    // top-level disable before the template.
    fn top_level_comment_reaches_template() {
        assert!(suppressed(
            "<!-- eslint-disable --><template><p v-html=\"x\" /></template>",
            "vue/no-v-html",
            "v-html"
        ));
    }
    #[test]
    fn description_is_stripped() {
        assert!(suppressed(
            "<template><!-- eslint-disable vue/no-v-html -- because --><p v-html=\"x\" /></template>",
            "vue/no-v-html",
            "v-html"
        ));
    }
    #[test]
    fn nested_template_comment_is_seen() {
        assert!(suppressed(
            "<template><div><!-- eslint-disable vue/no-v-html --><p v-html=\"x\" /></div></template>",
            "vue/no-v-html",
            "v-html"
        ));
    }
}
