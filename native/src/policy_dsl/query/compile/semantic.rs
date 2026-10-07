use super::*;
use crate::policy_dsl::language::GrammarDialect;
use crate::policy_dsl::matcher::facts::slot;
use crate::policy_dsl::matcher::{
    AstMatcher, BindingTest, Binds, FormatArgs, HostTest, ParseTest, Semantic,
};
/// A `parse` whose tree's rule is being compiled: the SQL expressions of the host document, node
/// and environment as the `parse` began, and whether its `host` rules can match.
#[derive(Clone)]
pub(super) struct Host {
    file: String,
    node: String,
    env: String,
    open: bool,
}
/// A Rerail predicate at one node: its term, and the SQL of the document, node and environment
/// its facts are read at.
pub(super) struct Leaf {
    term: usize,
    file: String,
    node: String,
    env: String,
}

impl Leaf {
    fn new(term: usize, file: &str, node: &str, env: &str) -> Self {
        Self {
            term,
            file: file.to_owned(),
            node: node.to_owned(),
            env: env.to_owned(),
        }
    }

    /// The `WHERE` clause selecting this predicate's facts, as `alias`.
    fn at(&self, alias: &str) -> String {
        format!(
            "{alias}.batch = ?1 AND {alias}.term = {} AND {alias}.file = {} AND {alias}.of = {} \
             AND {alias}.env = {}",
            self.term, self.file, self.node, self.env
        )
    }

    /// The `WHERE` clause selecting this predicate's facts of `slot` and `key`, as `alias`.
    fn slot(&self, alias: &str, slot: u16, key: &str) -> String {
        format!(
            "{} AND {alias}.slot = {slot} AND {alias}.key = {key}",
            self.at(alias)
        )
    }
}
impl Compiler<'_> {
    pub(super) fn semantic(
        &mut self,
        semantic: &Semantic,
        matcher: &AstMatcher,
        file: &str,
        node: &str,
        env: &str,
    ) -> Compiled {
        match semantic {
            Semantic::Binding(test) => self.binding(matcher, test, file, node, env),
            Semantic::Binds(test) => self.binds(matcher, test, file, node, env),
            Semantic::FormatArgs(test) => self.format_args(matcher, test, file, node, env),
            Semantic::Parse(test) => self.parse(matcher, test, file, node, env),
            Semantic::Host(test) => self.host(test, file, env),
            // Rerail's predicates without sub-rules (scopes, values, comparisons, layout, call
            // arguments, positions, captures): their own code tests the node the statement holds.
            Semantic::Resolves(_)
            | Semantic::Calls(_)
            | Semantic::Value(_)
            | Semantic::Arguments(_)
            | Semantic::Capture(_)
            | Semantic::Layout(_) => {
                let term = self.catalog.term(Term::Predicate(matcher.clone()));
                let three = i64::from(self.three);
                Ok(format!(
                    "tq_pred(?1, {term}, {file}, {node}, {env}, {three})"
                ))
            }
        }
    }

    /// A binding's nodes matched by the listed sub-rules (`slots`, `facts::slot::SITE` and the
    /// like), one after another from `start` in one trial environment: none is matched where the
    /// binding lacks a listed node; each spends a step, and fails where its node is missing or
    /// its sub-rule is already being matched there (`tq_follow`), as the matcher's
    /// `match_binding_nodes` does.
    pub(super) fn binding_nodes(
        &mut self,
        leaf: &Leaf,
        slots: [(u16, &Option<AstMatcher>); 3],
        key: &str,
        start: &str,
    ) -> Compiled {
        let listed = slots
            .into_iter()
            .filter_map(|(slot, rule)| rule.as_ref().map(|rule| (slot, rule)))
            .collect::<Vec<_>>();
        if listed.is_empty() {
            return Ok(start.to_owned());
        }
        let lacking = self.alias();
        let slot_list = listed
            .iter()
            .map(|(slot, _)| slot.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let mut current = format!(
            "CASE WHEN EXISTS (SELECT 1 FROM rerail_facts {lacking} WHERE {} AND {lacking}.key = \
             {key} AND {lacking}.slot IN ({slot_list}) AND {lacking}.flag = 0) THEN NULL \
             ELSE {start} END",
            leaf.at(&lacking)
        );
        for (slot, rule) in listed {
            let follow = (leaf.term as u64) << 8 | u64::from(slot);
            current = self.bind(&current, |this, before| {
                let target = this.alias();
                let followed = this.alias();
                let test = this.expr(
                    rule,
                    &leaf.file,
                    &format!("{target}.node"),
                    &format!("{followed}.e"),
                )?;
                Ok(format!(
                    "CASE WHEN {before} IS NULL OR {before} < 0 THEN {before} ELSE \
                     (SELECT CASE WHEN {target}.node IS NULL THEN NULL WHEN tq_step() THEN \
                     (SELECT tq_unfollow(?1, {test}, {before}) FROM (SELECT tq_follow(?1, \
                     {target}.node, {follow}, {before}) AS e) AS {followed}) END \
                     FROM rerail_facts {target} WHERE {}) END",
                    leaf.slot(&target, slot, key)
                ))
            })?;
        }
        Ok(current)
    }

    /// A binding's occurrences: the first where `rule` matches from `start`, each spending a
    /// step, as the matcher's `match_occurrence` does.
    pub(super) fn occurrences(
        &mut self,
        leaf: &Leaf,
        rule: &AstMatcher,
        key: &str,
        start: &str,
    ) -> Compiled {
        self.bind(start, |this, before| {
            let occurrence = this.alias();
            let test = this.expr(rule, &leaf.file, &format!("{occurrence}.node"), before)?;
            let test = this.fresh(&leaf.file, test);
            let candidate = format!(
                "CASE WHEN tq_step() THEN (CASE WHEN {occurrence}.node IS NULL THEN NULL \
                 ELSE {test} END) END"
            );
            let from = format!(
                "rerail_facts {occurrence} WHERE {} ORDER BY {occurrence}.pos",
                leaf.slot(&occurrence, slot::OCCURRENCE, key)
            );
            let first = this.first(&candidate, &from, false);
            Ok(format!(
                "CASE WHEN {before} IS NULL OR {before} < 0 THEN {before} ELSE {first} END"
            ))
        })
    }

    /// The predicate at `leaf`: `body` over its gate row (alias given), unknown where the gate
    /// is; no match without one.
    pub(super) fn gated(
        &mut self,
        leaf: &Leaf,
        body: impl FnOnce(&mut Self, &str) -> Compiled,
    ) -> Compiled {
        let gate = self.alias();
        let body = body(self, &gate)?;
        let env = &leaf.env;
        Ok(format!(
            "CASE WHEN {env} IS NULL OR {env} < 0 THEN {env} ELSE (SELECT CASE WHEN {gate}.flag < 0 \
             THEN -1 ELSE {body} END FROM rerail_facts {gate} WHERE {}) END",
            leaf.slot(&gate, slot::GATE, "0")
        ))
    }

    /// `resolves.binding`: the gate (the reference's binding passes the filters), the binding's
    /// nodes, then its occurrences.
    pub(super) fn binding(
        &mut self,
        matcher: &AstMatcher,
        test: &BindingTest,
        file: &str,
        node: &str,
        env: &str,
    ) -> Compiled {
        let term = self.catalog.term(Term::Semantic(matcher.clone()));
        self.bind(env, |this, env| {
            let leaf = Leaf::new(term, file, node, env);
            this.gated(&leaf, |this, _| {
                let nodes = this.binding_nodes(
                    &leaf,
                    [
                        (slot::SITE, &test.site),
                        (slot::VALUE, &test.value),
                        (slot::ANNOTATION, &test.annotation),
                    ],
                    "0",
                    env,
                )?;
                match &test.occurrence {
                    Some(rule) => this.occurrences(&leaf, rule, "0", &nodes),
                    None => Ok(nodes),
                }
            })
        })
    }

    /// `binds`: the first binding the node names (gate rows, in order) whose nodes, occurrences
    /// and `redefines` all match; unknown where none did and one was unknown.
    pub(super) fn binds(
        &mut self,
        matcher: &AstMatcher,
        test: &Binds,
        file: &str,
        node: &str,
        env: &str,
    ) -> Compiled {
        let term = self.catalog.term(Term::Semantic(matcher.clone()));
        let python = test.language == GrammarDialect::Python;
        self.bind(env, |this, env| {
            let leaf = Leaf::new(term, file, node, env);
            let binding = this.alias();
            let key = format!("{binding}.pos");
            let mut current = this.binding_nodes(
                &leaf,
                [
                    (slot::SITE, &test.site),
                    (slot::VALUE, &test.value),
                    (slot::ANNOTATION, &test.annotation),
                ],
                &key,
                env,
            )?;
            if let Some(rule) = &test.occurrence {
                current = this.occurrences(&leaf, rule, &key, &current)?;
            }
            if let Some(redefines) = test.redefines.as_deref() {
                if python {
                    current = this.binding_nodes(
                        &leaf,
                        [
                            (slot::SHADOWED_SITE, &redefines.site),
                            (slot::SHADOWED_SITE + 1, &redefines.value),
                            (slot::SHADOWED_SITE + 2, &redefines.annotation),
                        ],
                        &key,
                        &current,
                    )?;
                } else if redefines.same_scope == Some(false)
                    && !crate::policy_dsl::language::Family::of(test.language).reads_php()
                {
                    current = this.bind(&current, |_, before| {
                        Ok(format!(
                            "CASE WHEN {before} IS NULL OR {before} < 0 THEN {before} END"
                        ))
                    })?;
                } else {
                    current = this.redefines(&leaf, redefines, &key, &current)?;
                }
            }
            let candidate = format!("CASE WHEN {binding}.flag < 0 THEN -1 ELSE {current} END");
            let from = format!(
                "rerail_facts {binding} WHERE {} ORDER BY {binding}.pos",
                leaf.slot(&binding, slot::GATE, "0")
            );
            let first = this.first(&candidate, &from, false);
            Ok(format!(
                "CASE WHEN {env} IS NULL OR {env} < 0 THEN {env} ELSE {first} END"
            ))
        })
    }

    /// TypeScript's and a private name's `redefines` over the other definitions (`slot::OTHERS`)
    /// of the binding `key`: with `every`, each one passes and its nodes match, the environment
    /// threaded through them in order; otherwise the first that does.
    pub(super) fn redefines(
        &mut self,
        leaf: &Leaf,
        redefines: &crate::policy_dsl::matcher::Redefines,
        key: &str,
        start: &str,
    ) -> Compiled {
        let slots = [
            (slot::OTHER_SITE, &redefines.site),
            (slot::OTHER_SITE + 1, &redefines.value),
            (slot::OTHER_SITE + 2, &redefines.annotation),
        ];
        let shift = 1_i64 << slot::OTHER_BITS;
        self.bind(start, |this, before| {
            let other = this.alias();
            let other_key = format!("({key} * {shift} + {other}.pos)");
            if !redefines.every {
                let nodes = this.binding_nodes(leaf, slots, &other_key, before)?;
                let nodes = this.fresh(&leaf.file, nodes);
                let candidate = format!(
                    "CASE WHEN {other}.flag = -2 THEN -2 WHEN {other}.flag = -1 THEN -1 \
                     WHEN {other}.flag = 0 THEN NULL ELSE {nodes} END"
                );
                let from = format!(
                    "rerail_facts {other} WHERE {} ORDER BY {other}.pos",
                    leaf.slot(&other, slot::OTHERS, key)
                );
                let first = this.first(&candidate, &from, true);
                return Ok(format!(
                    "CASE WHEN {before} IS NULL OR {before} < 0 THEN {before} ELSE {first} END"
                ));
            }
            let fold = this.alias();
            let previous = format!("{fold}.e");
            let nodes = this.binding_nodes(leaf, slots, &other_key, &previous)?;
            let matched = this.alias();
            // A private name's other definition that binds the enumerated capture to a reported
            // range is passed over, the environment left as it was.
            let passed = if this.excluding {
                format!(
                    "CASE WHEN {other}.flag = 2 AND {matched}.r >= 0 THEN \
                     COALESCE(tq_fresh(?1, {}, {matched}.r), {previous}) ELSE {matched}.r END",
                    leaf.file
                )
            } else {
                format!("{matched}.r")
            };
            let step = format!(
                "CASE WHEN {other}.flag < 0 THEN -1 WHEN {other}.flag = 0 THEN NULL ELSE \
                 (SELECT {passed} FROM (SELECT {nodes} AS r) AS {matched}) END"
            );
            Ok(format!(
                "CASE WHEN {before} IS NULL OR {before} < 0 THEN {before} ELSE \
                 (WITH RECURSIVE {fold}(i, e) AS (SELECT -1, {before} UNION ALL \
                 SELECT {other}.pos, {step} FROM {fold}, rerail_facts {other} \
                 WHERE {fold}.e >= 0 AND {} AND {other}.pos = {fold}.i + 1) \
                 SELECT {fold}.e FROM {fold} ORDER BY {fold}.i DESC LIMIT 1) END",
                leaf.slot(&other, slot::OTHERS, key)
            ))
        })
    }

    /// `formatArgs`: the first unused argument `unused` matches, then the first missing field
    /// `missing` matches in a fresh environment (its captures exported), each unknown where only
    /// an argument or field not known to be unused (or missing) matched, or more may exist than
    /// the call shows; the verdicts combine as a three-valued `and`.
    pub(super) fn format_args(
        &mut self,
        matcher: &AstMatcher,
        test: &FormatArgs,
        file: &str,
        node: &str,
        env: &str,
    ) -> Compiled {
        let term = self.catalog.term(Term::Semantic(matcher.clone()));
        self.bind(env, |this, env| {
            let leaf = Leaf::new(term, file, node, env);
            this.gated(&leaf, |this, gate| {
                let incomplete = |this: &mut Self, pos: u8| {
                    let flag = this.alias();
                    format!(
                        "(SELECT {flag}.flag FROM rerail_facts {flag} WHERE {} AND {flag}.pos = {pos}) = 1",
                        leaf.slot(&flag, slot::INCOMPLETE, "0")
                    )
                };
                let unused = match &test.unused {
                    None => env.to_owned(),
                    Some(rule) => {
                        let found = this.search_known(&leaf, rule, slot::UNUSED, None, env)?;
                        let open = incomplete(this, 0);
                        let searched = this.alias();
                        format!(
                            "(SELECT CASE WHEN {searched}.v IS NULL AND {open} THEN -1 ELSE \
                             {searched}.v END FROM (SELECT {found} AS v) AS {searched})"
                        )
                    }
                };
                let first = this.alias();
                let second = this.alias();
                let after = format!("CASE WHEN {first}.u >= 0 THEN {first}.u ELSE {env} END");
                let (missing, exported) = match &test.missing {
                    None => (after.clone(), after.clone()),
                    Some(rule) => {
                        let fresh = format!("tq_following(?1, {env})");
                        let found = this.search_known(
                            &leaf,
                            rule,
                            slot::MISSING,
                            Some(&format!("{gate}.doc")),
                            &fresh,
                        )?;
                        let open = incomplete(this, 1);
                        let searched = this.alias();
                        (
                            format!(
                                "(SELECT CASE WHEN {searched}.v IS NULL AND {open} THEN -1 ELSE \
                                 {searched}.v END FROM (SELECT {found} AS v) AS {searched})"
                            ),
                            format!(
                                "CASE WHEN {second}.m >= 0 THEN tq_export(?1, {term}, {file}, \
                                 {after}, {gate}.doc, {second}.m) ELSE {after} END"
                            ),
                        )
                    }
                };
                Ok(format!(
                    "(SELECT CASE WHEN {first}.u IS NULL THEN NULL ELSE (SELECT CASE WHEN \
                     {second}.m IS NULL THEN NULL WHEN {first}.u < 0 OR {second}.m < 0 THEN -1 \
                     ELSE {exported} END FROM (SELECT {missing} AS m) AS {second}) END \
                     FROM (SELECT {unused} AS u) AS {first})"
                ))
            })
        })
    }

    /// `formatArgs`' search over its unused arguments or missing fields (`slot`): the first
    /// candidate `rule` matches, from `env`; unknown where a candidate not known to qualify (or an
    /// unknown verdict) was all that matched. `doc` is the candidates' document (the parsed
    /// template's), else the statement's.
    pub(super) fn search_known(
        &mut self,
        leaf: &Leaf,
        rule: &AstMatcher,
        slot: u16,
        doc: Option<&str>,
        env: &str,
    ) -> Compiled {
        let candidate = self.alias();
        let file = doc.map_or_else(|| leaf.file.clone(), |_| format!("{candidate}.doc"));
        let test = self.expr(rule, &file, &format!("{candidate}.node"), env)?;
        let test = self.fresh(&file, test);
        let verdict = self.alias();
        let known = format!(
            "(SELECT CASE WHEN {verdict}.r IS NULL THEN NULL WHEN {verdict}.r < 0 THEN -1 \
             WHEN {candidate}.flag = 1 THEN {verdict}.r ELSE -1 END FROM (SELECT {test} AS r) AS {verdict})"
        );
        let from = format!(
            "rerail_facts {candidate} WHERE {} ORDER BY {candidate}.pos",
            leaf.slot(&candidate, slot, "0")
        );
        let three = std::mem::replace(&mut self.three, true);
        let found = self.first(&known, &from, false);
        self.three = three;
        Ok(found)
    }

    /// `parse`: the tree's rule matched on its root (a derived document) in a fresh environment,
    /// its captures exported to the host.
    pub(super) fn parse(
        &mut self,
        matcher: &AstMatcher,
        test: &ParseTest,
        file: &str,
        node: &str,
        env: &str,
    ) -> Compiled {
        let term = self.catalog.term(Term::Semantic(matcher.clone()));
        self.bind(env, |this, env| {
            let leaf = Leaf::new(term, file, node, env);
            this.gated(&leaf, |this, gate| {
                let host = this.host.replace(Host {
                    file: file.to_owned(),
                    node: node.to_owned(),
                    env: env.to_owned(),
                    open: test.host_sound.is_some(),
                });
                let rule = this.expr(
                    &test.rule,
                    &format!("{gate}.doc"),
                    &format!("{gate}.node"),
                    &format!("tq_following(?1, {env})"),
                );
                this.host = host;
                Ok(format!(
                    "tq_export(?1, {term}, {file}, {env}, {gate}.doc, {})",
                    rule?
                ))
            })
        })
    }

    /// `host` (inside a `parse`'s rule): the host rule on the host node, in the host's
    /// environment as the `parse` began with the tree's captures so far exported; binds nothing
    /// and excludes nothing.
    pub(super) fn host(&mut self, test: &HostTest, file: &str, env: &str) -> Compiled {
        let Some(host) = self.host.clone() else {
            return Err(Unsupported("`host` outside a `parse`"));
        };
        if !host.open {
            return Ok("NULL".to_owned());
        }
        self.bind(env, |this, env| {
            let hosted = format!(
                "tq_host_env(?1, {file}, {env}, {}, {})",
                host.file, host.env
            );
            let saved = this.host.take();
            let rule =
                this.suspended(|this| this.expr(&test.rule, &host.file, &host.node, &hosted));
            this.host = saved;
            let verdict = this.alias();
            Ok(format!(
                "CASE WHEN {env} IS NULL OR {env} < 0 THEN {env} ELSE (SELECT CASE WHEN \
                 {verdict}.v IS NULL THEN NULL WHEN {verdict}.v < 0 THEN -1 ELSE {env} END \
                 FROM (SELECT {} AS v) AS {verdict}) END",
                rule?
            ))
        })
    }
}
