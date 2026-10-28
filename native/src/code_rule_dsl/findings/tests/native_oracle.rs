//! Native-only oracle on the reference's existing tapes. Index publication is
//! construction; candidate selection and matching are execution. Scope/memo slots
//! span every clause of a file, as in a complete production file evaluation.
use crate::code_rule_dsl::{
    CompiledCodeRuleDsl,
    language::GrammarDialect,
    matcher::{self, FileAnchorIndex, FileGates, FileScopeSlot, Selection, Stopped},
    model::{Anchor, ClauseKind, FileInput, Query},
    module_path::ModulePath,
    origin::InFile,
    phase_clock,
    soundness::SyntaxErrors,
    tape::TapeDoc,
};
use ast_grep_core::{AstGrep, Doc};
use std::{
    collections::{BTreeMap, BTreeSet},
    convert::Infallible,
    sync::{Arc, atomic::Ordering},
    time::Instant,
};

const CLAUSE_WORK: usize = 100_000_000;

pub(super) struct Native<'a> {
    parsed: &'a [AstGrep<TapeDoc>],
    compiled: &'a CompiledCodeRuleDsl,
    paths: &'a [String],
    modules: &'a BTreeMap<u32, ModulePath>,
    /// CPU time publishing the input tapes into one reusable production index. W's anchor index
    /// is built per file while matching (`FileAnchorIndex::build`), so this is zero and `run`'s
    /// `prepare_ns` counts the index.
    pub index_ns: u64,
    pub index_wall_ns: u64,
}

pub(super) struct NativeRun {
    pub keys: BTreeSet<(String, u32, usize)>,
    /// Selection + matching CPU, excluding the separately reported SQL compilation.
    pub cpu_ns: u64,
    /// Actual elapsed run time, including any SQL compilation in this run.
    pub wall_ns: u64,
    /// Candidate preparation CPU excluding its nested SQL compilation; part of cpu_ns.
    pub prepare_ns: u64,
    pub compile_ns: u64,
    pub roots_matched: usize,
}

impl<'a> Native<'a> {
    pub(super) fn new(
        parsed: &'a [AstGrep<TapeDoc>],
        compiled: &'a CompiledCodeRuleDsl,
        paths: &'a [String],
        modules: &'a BTreeMap<u32, ModulePath>,
    ) -> Self {
        assert_eq!(parsed.len(), paths.len());
        assert!(parsed.len() <= compiled.inner.capabilities.limits.max_files);
        assert!(
            parsed
                .iter()
                .map(|root| root.root().get_doc().file_source().len())
                .sum::<usize>()
                <= compiled.inner.capabilities.limits.max_source_bytes
        );

        assert_eq!(
            paths.iter().collect::<BTreeSet<_>>().len(),
            paths.len(),
            "native oracle paths must be unique"
        );
        for root in parsed {
            assert_eq!(
                *root.root().get_doc().get_lang(),
                GrammarDialect::Python,
                "native oracle supports Python tapes only"
            );
        }
        let (index_ns, index_wall_ns) = (0, 0);
        Self {
            parsed,
            compiled,
            paths,
            modules,
            index_ns,
            index_wall_ns,
        }
    }

    pub(super) fn run(&self) -> NativeRun {
        let wall = Instant::now();
        let started = super::cpu_ns();
        let roots_before = matcher::roots_matched();
        let compile_before = phase_clock::ANCHOR_COMPILE.load(Ordering::Relaxed);
        let compile_ns = phase_clock::ANCHOR_COMPILE
            .load(Ordering::Relaxed)
            .saturating_sub(compile_before);
        let mut prepare_ns = 0;
        let file_plans: Vec<_> = self
            .compiled
            .inner
            .clauses
            .iter()
            .filter_map(|clause| match &clause.kind {
                ClauseKind::Forbid(Query::Ast { matchers, .. }) => Some(matchers),
                _ => None,
            })
            .flatten()
            .map(|grammar| grammar.anchors.as_ref())
            .collect();
        let mut keys = BTreeSet::new();
        for (doc, root) in self.parsed.iter().enumerate() {
            let path = &self.paths[doc];
            if !self.compiled.inner.scope.matches(path) {
                continue;
            }
            let tape = root.root().get_doc();
            assert!(
                tape.transducer_len() <= CLAUSE_WORK,
                "native oracle file exceeds the 100m-node allowance"
            );
            let file = FileInput {
                path: path.clone(),
                language: "python".into(),
                source: tape.file_source().clone(),
                module: self
                    .modules
                    .get(&(doc as u32))
                    .cloned()
                    .or_else(|| ModulePath::stem(path))
                    .map(Arc::new),
                package: None,
            };
            let errors = SyntaxErrors::of_doc(tape);
            let _scopes =
                FileScopeSlot::install(Vec::new(), file.module.clone(), errors.hide_bindings())
                    .file(&file);
            let gates = FileGates::new(&file, root.root());
            let preparing = super::cpu_ns();
            let index = FileAnchorIndex::build(root, &file_plans, CLAUSE_WORK, || false);
            prepare_ns += super::cpu_ns().saturating_sub(preparing);
            assert!(
                index.complete(),
                "native oracle requires a complete anchor index; refusing silent full scan"
            );
            let place = InFile::tape(tape);
            matcher::with_file_memo(root, || {
                for clause in &self.compiled.inner.clauses {
                    let ClauseKind::Forbid(Query::Ast { matchers, calls }) = &clause.kind else {
                        panic!("native oracle accepts forbid AST clauses only")
                    };
                    assert!(
                        calls.is_empty(),
                        "native oracle does not install call closure seeds"
                    );
                    assert!(
                        clause.violation.each.is_none(),
                        "native oracle does not enumerate violation.each"
                    );
                    assert!(
                        matches!(clause.violation.anchor, Anchor::Span),
                        "native oracle reports witness.span only"
                    );
                    for grammar in matchers
                        .iter()
                        .filter(|g| g.language == GrammarDialect::Python && g.document.is_none())
                    {
                        assert!(
                            grammar.transform.is_none(),
                            "native oracle does not render transformed witnesses"
                        );
                        if !grammar.applies
                            || !grammar.file.as_ref().is_none_or(|condition| {
                                condition.holds(path, file.module.as_deref(), None)
                            })
                            || !gates.admit(grammar)
                            || !grammar.text.as_ref().is_none_or(|text| {
                                text.holds_with(&file.source, &mut |source: &str, literal| {
                                    memchr::memmem::find(source.as_bytes(), literal.as_bytes())
                                        .is_some()
                                })
                            })
                        {
                            continue;
                        }
                        assert!(
                            !errors.incomplete(grammar),
                            "native oracle requires complete clauses; syntax-error input is incomplete"
                        );
                        let selected: Option<Arc<[u32]>> =
                            match index.select(&grammar.anchors) {
                                Selection::All => None,
                                Selection::Roots(roots) => Some(
                                    roots
                                        .iter()
                                        .map(|&ordinal| index.node(ordinal).node_id() as u32)
                                        .collect(),
                                ),
                            };
                        let mut work = CLAUSE_WORK;
                        let result = matcher::run_clause(
                            root,
                            grammar,
                            selected.as_deref(),
                            CLAUSE_WORK,
                            &mut work,
                            |ordinal, _env, _work| -> Result<bool, Infallible> {
                                let node = matcher::node_at(root, ordinal);
                                if !errors.meets(&place.place(&node)) {
                                    keys.insert((
                                        clause.violation.code.clone(),
                                        doc as u32,
                                        node.start_pos().line() + 1,
                                    ));
                                }
                                Ok(true)
                            },
                        )
                        .unwrap();
                        match result {
                            Ok(()) => {}
                            Err(Stopped::Limit(limit)) => {
                                panic!("native oracle utility limit: {limit:?}")
                            }
                            Err(Stopped::Deadline) => panic!("native oracle deadline"),
                        }
                    }
                }
            });
        }
        NativeRun {
            keys,
            cpu_ns: super::cpu_ns()
                .saturating_sub(started)
                .saturating_sub(compile_ns),
            wall_ns: wall.elapsed().as_nanos() as u64,
            prepare_ns,
            compile_ns,
            roots_matched: matcher::roots_matched().saturating_sub(roots_before),
        }
    }
}
