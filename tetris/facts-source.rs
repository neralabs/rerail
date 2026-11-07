
// Source attribution for the standalone fact runner, captured while the native lowerer
// expands each source clause. Generated aggregate helpers retain their parent clause.
thread_local! {
    static RULE_SOURCE_LINES: std::cell::RefCell<std::collections::BTreeMap<String, std::collections::VecDeque<usize>>> = const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
}

pub(in crate::code_rule_dsl) fn reset_rule_sources() {
    RULE_SOURCE_LINES.with(|sources| sources.borrow_mut().clear());
}

pub(super) fn record_rule_sources(rules: &[super::front::Rule], line: usize) {
    RULE_SOURCE_LINES.with(|sources| {
        let mut sources = sources.borrow_mut();
        for rule in rules { sources.entry(rule.to_string()).or_default().push_back(line); }
    });
}

pub(in crate::code_rule_dsl) fn take_rule_sources() -> std::collections::BTreeMap<String, std::collections::VecDeque<usize>> {
    RULE_SOURCE_LINES.with(|sources| std::mem::take(&mut *sources.borrow_mut()))
}
