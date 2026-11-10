//! Native-only oracle on the reference's existing tapes. Index publication is
//! construction; candidate selection and matching are execution. Scope/memo slots
//! span every clause of a file, as in a complete production file evaluation.
use crate::code_rule_dsl::{
    CompiledCodeRuleDsl,
    language::GrammarDialect,
    matcher::{self, Corpus, CorpusSlot, FileAnchorIndex, FileGates, FileScopeSlot, Stopped},
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
    corpus: Arc<Corpus>,
    /// CPU time publishing the input tapes into one reusable production index.
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
        let started = super::cpu_ns();
        let wall = Instant::now();
        let corpus = Arc::new(Corpus::default());
        for (root, path) in parsed.iter().zip(paths) {
            corpus
                .put(path, root.root().get_doc(), &|| false)
                .expect("publish native oracle tape");
        }
        let index_ns = super::cpu_ns().saturating_sub(started);
        let index_wall_ns = wall.elapsed().as_nanos() as u64;
        Self {
            parsed,
            compiled,
            paths,
            modules,
            corpus,
            index_ns,
            index_wall_ns,
        }
    }

    pub(super) fn run(&self) -> NativeRun {
        let wall = Instant::now();
        let started = super::cpu_ns();
        let roots_before = matcher::roots_matched();
        let compile_before = phase_clock::ANCHOR_COMPILE.load(Ordering::Relaxed);
        let preparing = super::cpu_ns();
        let plans = matcher::anchor_plans([self.compiled.inner.as_ref()]);
        self.corpus
            .prepare(&plans)
            .expect("prepare native oracle anchors");
        let prepare_total = super::cpu_ns().saturating_sub(preparing);
        let compile_ns = phase_clock::ANCHOR_COMPILE
            .load(Ordering::Relaxed)
            .saturating_sub(compile_before);
        let prepare_ns = prepare_total.saturating_sub(compile_ns);
        let _corpus = CorpusSlot::install(self.corpus.clone());
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
            let file_plans: Vec<_> = plans.values().flatten().map(Arc::as_ref).collect();
            let index = FileAnchorIndex::build_at(root, &file_plans, CLAUSE_WORK, || false, path);
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
                            || !grammar.admits_source(
                                &file.source,
                                &errors,
                                &mut rustc_hash::FxHashMap::default(),
                            )
                        {
                            continue;
                        }
                        assert!(
                            !errors.incomplete(grammar),
                            "native oracle requires complete clauses; syntax-error input is incomplete"
                        );
                        let selected = if grammar.anchors.needs_index() {
                            let selected = index.selected(&grammar.anchors);
                            assert!(
                                selected.is_some(),
                                "native oracle requires prepared candidate roots; refusing silent full scan"
                            );
                            selected
                        } else {
                            None
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
                                if grammar.invalid_syntax == Some(true)
                                    || !errors.meets(&place.place(&node))
                                {
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
