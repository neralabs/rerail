//! One fresh line-filter state per file evaluation. No linter-specific syntax or names.
use crate::code_rule_dsl::model::RegexLines;

pub(super) struct State {
    state: usize,
    indentation: Option<usize>,
}

impl State {
    pub(super) fn new(lines: Option<&RegexLines>) -> Self {
        Self {
            state: lines
                .and_then(|l| l.state.as_ref())
                .map_or(0, |s| s.initial),
            indentation: None,
        }
    }

    /// Whether the current line emits, or an incomplete result after work exhaustion.
    pub(super) fn allows(&mut self, lines: &RegexLines, text: &str) -> Result<bool, ()> {
        let mut emit = true;
        if let Some(indentation) = &lines.indentation {
            let width = text.chars().take_while(|ch| ch.is_whitespace()).count();
            emit = self.indentation.is_none_or(|prior| width <= prior);
            if !indentation.ignore.iter().any(|r| r.is_match(text)) {
                if indentation.reset.is_match(text) {
                    self.indentation = None;
                }
                if indentation.exclude_under.is_match(text) {
                    self.indentation =
                        Some(self.indentation.map_or(width, |prior| prior.min(width)));
                }
            }
        }
        if let Some(state) = &lines.state {
            emit &= state.emit[self.state];
            for transition in &state.transitions {
                if transition.from != self.state {
                    continue;
                }
                let mut count = 0;
                for _ in transition.pattern.find_iter(text) {
                    if crate::code_rule_dsl::budget::matcher_budget_exhausted() {
                        return Err(());
                    }
                    count += 1;
                }
                if count >= transition.min
                    && transition.max.is_none_or(|max| count <= max)
                    && transition.odd.is_none_or(|odd| (count % 2 == 1) == odd)
                {
                    self.state = transition.to;
                    emit &= !transition.skip;
                    break;
                }
            }
        }
        Ok(emit)
    }
}
