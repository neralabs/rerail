use super::*;
use crate::code_rule_dsl::typing::terms::Ctor;

/// A name interner standing in for the store's.
#[derive(Default)]
struct Names(Vec<String>);
impl Names {
    fn id(&mut self, s: &str) -> Sym {
        match self.0.iter().position(|x| x == s) {
            Some(i) => i as Sym,
            None => {
                self.0.push(s.to_owned());
                (self.0.len() - 1) as Sym
            }
        }
    }
}

const FILE: &str = r#"# rerail.types/v1
# module: vendor.client
from typing import Callable, Optional, TypeVar
T = TypeVar("T", bound=Hashable)

def get(key: str, default: T | None = None) -> T | None: ...
def each(f: Callable[[int], str], xs: list[int]) -> tuple[str, ...]: ...
def mode() -> Literal["r", "w"]: ...

class Client:
    def fetch(self, url: str, timeout: Optional[float] = ...) -> bytes: ...
    async def close(self) -> None: ...

def after() -> "Client": ...
"#;

#[test]
fn declarations_read_stub_syntax_as_terms() {
    let (mut terms, mut names) = (Terms::default(), Names::default());
    let d = parse(FILE, &mut terms, &mut |s| names.id(s)).unwrap();
    assert_eq!(d.module, "vendor.client");
    let keys: Vec<&str> = d.signatures.keys().map(String::as_str).collect();
    assert_eq!(keys, ["vendor.client.Client.close", "vendor.client.Client.fetch", "vendor.client.after", "vendor.client.each", "vendor.client.get", "vendor.client.mode"]);
    let mut n = |s: &str| names.id(s);
    // get(key: str, default: T | None) -> T | None, T bounded by Hashable.
    let (s, none, h) = (terms.named(n("str"), &[]), terms.named(n("None"), &[]), terms.named(n("Hashable"), &[]));
    let tv = terms.var(n("T"), Some(h));
    let opt_t = terms.union(&[tv, none], false);
    assert_eq!(d.signatures["vendor.client.get"], terms.callable(&[s, opt_t], opt_t));
    // fetch(self, url: str, timeout: Optional[float]) -> bytes; self unannotated is ⊤.
    let (f, b) = (terms.named(n("float"), &[]), terms.named(n("bytes"), &[]));
    let opt_f = terms.union(&[f, none], false);
    let top = terms.top();
    assert_eq!(d.signatures["vendor.client.Client.fetch"], terms.callable(&[top, s, opt_f], b));
    // Callable[[int], str] and a literal union.
    let i = terms.named(n("int"), &[]);
    let cb = terms.callable(&[i], s);
    let each = terms.get(d.signatures["vendor.client.each"]).clone();
    assert_eq!(each.args[0], cb);
    let mode = terms.get(d.signatures["vendor.client.mode"]).clone();
    assert_eq!(terms.get(*mode.args.last().unwrap()).ctor, Ctor::Union);
    // A quoted name is a forward reference.
    let after = terms.get(d.signatures["vendor.client.after"]).clone();
    assert_eq!(terms.get(after.args[0]).ctor, Ctor::Reference);
}

#[test]
fn declarations_fill_gaps_only_and_report_conflicts() {
    let (mut terms, mut names) = (Terms::default(), Names::default());
    let d = parse(FILE, &mut terms, &mut |s| names.id(s)).unwrap();
    let b = terms.named(names.id("bytes"), &[]);
    let other = terms.callable(&[], b);
    let source = BTreeMap::from([
        ("vendor.client.get".to_owned(), d.signatures["vendor.client.get"]), // the same: no conflict
        ("vendor.client.mode".to_owned(), other),                           // differs: kept, reported
    ]);
    let (filled, conflicts) = fill(&source, &d);
    assert_eq!(filled["vendor.client.mode"], other, "the source's annotation wins");
    assert_eq!(filled["vendor.client.Client.fetch"], d.signatures["vendor.client.Client.fetch"], "a gap is filled");
    assert_eq!(filled.len(), d.signatures.len());
    assert_eq!(conflicts, vec![Conflict { name: "vendor.client.mode".into(), source: other, declared: d.signatures["vendor.client.mode"] }]);
}

#[test]
fn a_file_without_the_header_or_with_other_code_is_refused() {
    let (mut terms, mut names) = (Terms::default(), Names::default());
    assert_eq!(parse("def f() -> int: ...", &mut terms, &mut |s| names.id(s)).unwrap_err().line, 1);
    let e = parse("# rerail.types/v1\nx = compute()\n", &mut terms, &mut |s| names.id(s)).unwrap_err();
    assert_eq!(e.line, 2);
}
