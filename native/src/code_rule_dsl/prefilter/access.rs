//! Value predicates operate on distinct vocabulary terms before locating packed postings.
use super::{Lower, Set};

impl Lower {
    pub(super) fn text(&mut self, scope: &Set, expression: &str, literal: bool) -> Result<Set, ()> {
        let domain = self.reuse.kinds.get(&scope.name).cloned();
        let bare = scope.name == "all_nodes"
            || domain.as_ref().is_some_and(|kinds| {
                !kinds.contains(&0)
                    && !kinds.contains(&u16::MAX)
                    && self
                        .roots
                        .get(kinds)
                        .is_some_and(|set| set.name == scope.name)
            });
        let key = (expression.to_owned(), literal, domain.clone());
        let indexed = if let Some(set) = self.reuse.values.get(&key) {
            set.clone()
        } else {
            let term_key = (expression.to_owned(), literal);
            let terms = if let Some(name) = self.reuse.terms.get(&term_key) {
                name.clone()
            } else {
                let expression = self.parameter(expression.to_owned());
                let lang = self.parameter(i64::from(self.lang));
                let name = format!("terms{}", self.ctes.len());
                let body = if literal {
                    format!("SELECT term FROM term_freq WHERE lang={lang} AND term={expression}")
                } else {
                    self.predicates
                        .insert(name.clone(), (lang.clone(), expression.clone()));
                    format!(
                        "SELECT term FROM vocabulary_term WHERE lang={lang} AND expression={expression}"
                    )
                };
                self.ctes
                    .push(format!("{name}(term) AS NOT MATERIALIZED ({body})"));
                self.reuse.terms.insert(term_key, name.clone());
                name
            };
            let lang = self.parameter(i64::from(self.lang));
            let guard = self.value_kind_guard(domain.as_ref(), "v");
            let set = self.relation(format!("SELECT v.tape_id,p.pre FROM {terms} t CROSS JOIN cf CROSS JOIN term_postings v ON v.tape_id=cf.fid CROSS JOIN nera_postings(v.postings) p WHERE v.lang={lang} AND v.term=t.term{guard}"), true)?;
            self.indexed.insert(set.name.clone());
            self.reuse.values.insert(key, set.clone());
            set
        };
        // Bare kind domains are answered directly by value postings. Other scopes use indexed
        // membership, with the value hits outside, rather than scanning the whole kind set.
        let selected = self.value_within(scope, &indexed, bare)?;
        let guard = self.value_kind_guard(domain.as_ref(), "c");
        let uncovered = self.relation(format!("SELECT c.tape_id,p.pre FROM cf CROSS JOIN term_coverage c ON c.tape_id=cf.fid CROSS JOIN nera_postings(c.missing) p WHERE length(c.missing)>0{guard}"), true)?;
        self.indexed.insert(uncovered.name.clone());
        // Uncovered nodes are often every node of a kind (any node with an interior), so the scope,
        // within the same kinds, is the smaller side: iterate it and probe the indexed postings.
        let fallback = if bare {
            self.value_within(scope, &uncovered, bare)?
        } else {
            self.relation(format!("SELECT s.fid,s.pre FROM {} s WHERE EXISTS(SELECT 1 FROM {} u WHERE u.fid=s.fid AND u.pre=s.pre)", scope.name, uncovered.name), scope.exact)?
        };
        // P confirmed uncovered nodes against their decoded source spans. Unconfirmed, they are
        // candidates: the union is a superset.
        let confirmed = Set {
            exact: false,
            ..fallback
        };
        let selected = self.combine(&selected, &confirmed, "UNION", false)?;
        Ok(self.narrowed(selected, scope))
    }
    fn value_kind_guard(
        &mut self,
        domain: Option<&std::collections::BTreeSet<u16>>,
        alias: &str,
    ) -> String {
        domain.map_or(String::new(), |kinds| {
            let parameters = kinds
                .iter()
                .map(|&kind| {
                    let atom = self.atom(kind);
                    self.parameter(atom)
                })
                .collect::<Vec<_>>();
            format!(" AND {alias}.kind IN ({})", parameters.join(","))
        })
    }
    fn value_within(&mut self, scope: &Set, points: &Set, bare: bool) -> Result<Set, ()> {
        if bare {
            return Ok(Set {
                name: points.name.clone(),
                exact: scope.exact,
            });
        }
        self.indexed.insert(scope.name.clone());
        self.relation(format!("SELECT p.fid,p.pre FROM {} p WHERE EXISTS(SELECT 1 FROM {} s WHERE s.fid=p.fid AND s.pre=p.pre)", points.name, scope.name), scope.exact)
    }
}
