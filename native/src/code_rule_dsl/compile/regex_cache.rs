//! Compilation-scoped regex programs. Compiled matchers own their returned handles.
use regex::Regex;
use std::{cell::RefCell, collections::HashMap};
thread_local! {static PROGRAMS:RefCell<Option<HashMap<String,Regex>>>=const{RefCell::new(None)};}
pub(super) struct Scope(Option<HashMap<String, Regex>>);
impl Scope {
    pub(super) fn enter() -> Self {
        Self(PROGRAMS.replace(Some(HashMap::new())))
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        PROGRAMS.replace(self.0.take());
    }
}
pub(super) fn compile(source: &str) -> Result<Regex, regex::Error> {
    if let Some(regex) = PROGRAMS.with_borrow(|programs| {
        programs
            .as_ref()
            .and_then(|cache| cache.get(source))
            .cloned()
    }) {
        return Ok(regex);
    }
    #[cfg(test)]
    COMPILES.set(COMPILES.get() + 1);
    let regex = Regex::new(source)?;
    PROGRAMS.with_borrow_mut(|programs| {
        if let Some(cache) = programs {
            cache.insert(source.into(), regex.clone());
        }
    });
    Ok(regex)
}
#[cfg(test)]
thread_local! {static COMPILES:std::cell::Cell<usize>=const{std::cell::Cell::new(0)};}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_patterns_compile_once_without_retention_after_the_scope() {
        COMPILES.set(0);
        {
            let _scope = Scope::enter();
            let a = compile("^target$").unwrap();
            let b = compile("^target$").unwrap();
            assert!(a.is_match("target") && b.is_match("target"));
            assert_eq!(
                COMPILES.get(),
                1,
                "validation and matching must share one immutable program"
            );
        }
        assert!(PROGRAMS.with_borrow(Option::is_none));
        {
            let _scope = Scope::enter();
            compile("^target$").unwrap();
        }
        assert_eq!(COMPILES.get(), 2, "a later compilation owns its own cache");
    }
}
