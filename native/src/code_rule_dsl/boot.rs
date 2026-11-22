//! The engine's composition (docs/final-shape/proposals/crate-split/incremental-plan.md, step 1): every provider the
//! build selects, registered into the core's registries in one order, once a process, before any run. The engine
//! (napi, the CLI) calls `install` as it loads; so does any test binary that needs a booted program.
//!
//! The core never names a provider: the language packs, Metal, and the engine's glue (the prefilter, the functors,
//! the `ast` wave compiler) are named here alone.
use super::*;

/// The build's identities (`build_ids`): the engine's build and its grammars' build. The engine's build script makes
/// them (`RERAIL_ENGINE_BUILD_ID`, `RERAIL_GRAMMAR_BUILD_ID`); the engine passes them in.
#[derive(Clone, Copy, Debug)]
pub struct BuildIds {
    pub engine: &'static str,
    pub grammar: &'static str,
}

/// Boots the program, once a process: the build's identities, the devices, the language packs and the glue, the
/// git inventory's readers, and the SQL functions of every connection the store opens. A second call does nothing.
pub fn install(ids: BuildIds) {
    install_with(Some(ids));
}

/// Boots a test binary that is not the engine's: as `install`, with the binary's own build identities, which
/// `build_ids` takes on its first read when none was set.
#[cfg(any(test, feature = "test-hooks"))]
pub fn install_for_tests() {
    install_with(None);
}

/// The literal grammars only tests parse with: none outside a test build.
fn test_grammars(tests: bool) -> Vec<literal::Grammar> {
    #[cfg(any(test, feature = "test-hooks"))]
    if tests {
        return vec![literal::CHARS];
    }
    let _ = tests;
    Vec::new()
}

fn install_with(ids: Option<BuildIds>) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        if let Some(ids) = ids {
            build_ids::set(ids.engine, ids.grammar);
        }
        install_devices();
        install_languages(ids.is_none());
        // The retained traces, which the SQL executor's parts below them read (`maintenance::TEMPORAL`).
        sql_executor::temporal::register();
        logic::plan::corpus::install_git_inventory();
        logic::run_state::CARRIERS.register("boot", logic::CARRIERS);
        logic::plan::corpus::RUNNER.register("boot", &logic::plan::corpus::Corpus);
        // A workspace index task's run (decision 0441) and what writes its profile (0442).
        workspace::index::TASK_RUN.register("boot", workspace::index::TaskRun {
            scope: || Box::new(logic::run::scope(logic::native::width::Settings::from_env)),
            report: logic::profile::report::write,
        });
        residency::admission::relational::ADMISSION.register("boot", &residency::admission::relational::Relational);
        logic::plan::keeps::CORE_NAMES.register("boot", logic::rr::core_names);
        #[cfg(target_os = "macos")]
        logic::costs::join_steps::register_install(logic::plan::joins::model::install);
        // What the store hands the observer and git below it: the observer's kept facts, a reused inventory's
        // manifests owned again.
        crate::content_store::register_below();
        crate::content_store::set_connection_functions(|db| {
            text::install(db)?;
            prefilter::tapes::install(db)
        });
        sql_hazards::strict_on_every_connection();
    });
}

/// The engine's language packs, into the core's registry (`language::adapter::pack`). Called as the library
/// loads, before any run: the core asks a family's questions of the pack registered here.
/// The families' relations' columns, for the facts' base (`facts::core::FAMILY_COLUMNS`), which reads them as an
/// input's (`input_columns`) and cannot name the syntax table that registers the families.
fn register_family_columns() {
    facts::core::FAMILY_COLUMNS.register("rerail-facts (the syntax table's families)", facts::vtab::syntax::family_columns);
}

pub fn install_languages(tests: bool) {
    use language::Family;
    use language::adapter::pack::{Pack, pack, register};
    use language::adapter::{blade::Blade, c::C, css::Css, csharp::Csharp, go::Go, ecma::Ecma, format::Format, html::Html, java::Java, json::Json, markdown::Markdown, php::Php, python::Python, rr::Rr, rust::Rust, sql::Sql, vue::Vue, kotlin::Kotlin, scala::Scala, swift::Swift};
    register(Family::Python, &Pack::<Python>::NEW);
    language::register_base(Family::Python, &Pack::<Python>::NEW);
    register(Family::Ecma, &Pack::<Ecma>::NEW);
    language::register_base(Family::Ecma, &Pack::<Ecma>::NEW);
    register(Family::Php, &Pack::<Php>::NEW);
    language::register_base(Family::Php, &Pack::<Php>::NEW);
    register(Family::Blade, &Pack::<Blade>::NEW);
    language::register_base(Family::Blade, &Pack::<Blade>::NEW);
    register(Family::Rust, &Pack::<Rust>::NEW);
    language::register_base(Family::Rust, &Pack::<Rust>::NEW);
    register(Family::Sql, &Pack::<Sql>::NEW);
    language::register_base(Family::Sql, &Pack::<Sql>::NEW);
    register(Family::Vue, &Pack::<Vue>::NEW);
    language::register_base(Family::Vue, &Pack::<Vue>::NEW);
    register(Family::Rr, &Pack::<Rr>::NEW);
    language::register_base(Family::Rr, &Pack::<Rr>::NEW);
    register(Family::Json, &Pack::<Json>::NEW);
    language::register_base(Family::Json, &Pack::<Json>::NEW);
    register(Family::Format, &Pack::<Format>::NEW);
    language::register_base(Family::Format, &Pack::<Format>::NEW);
    register(Family::Java, &Pack::<Java>::NEW);
    language::register_base(Family::Java, &Pack::<Java>::NEW);
    register(Family::Csharp, &Pack::<Csharp>::NEW);
    language::register_base(Family::Csharp, &Pack::<Csharp>::NEW);
    register(Family::Go, &Pack::<Go>::NEW);
    language::register_base(Family::Go, &Pack::<Go>::NEW);
    register(Family::C, &Pack::<C>::NEW);
    language::register_base(Family::C, &Pack::<C>::NEW);
    register(Family::Kotlin, &Pack::<Kotlin>::NEW);
    language::register_base(Family::Kotlin, &Pack::<Kotlin>::NEW);
    register(Family::Swift, &Pack::<Swift>::NEW);
    language::register_base(Family::Swift, &Pack::<Swift>::NEW);
    register(Family::Scala, &Pack::<Scala>::NEW);
    language::register_base(Family::Scala, &Pack::<Scala>::NEW);
    // Answers inside the fixpoint (`answers`): C's directives by file (0327 mode (a)).
    language::install_answerers(vec![&language::adapter::c_pp::Directives, &language::adapter::c_pp::Units, &language::adapter::c_pp::CompileCommands]);
    register(Family::Html, &Pack::<Html>::NEW);
    language::register_base(Family::Html, &Pack::<Html>::NEW);
    register(Family::Css, &Pack::<Css>::NEW);
    language::register_base(Family::Css, &Pack::<Css>::NEW);
    register(Family::Markdown, &Pack::<Markdown>::NEW);
    language::register_base(Family::Markdown, &Pack::<Markdown>::NEW);
    // The linters' suppressions, in the order `suppress` is parsed and checked in.
    {
        use suppress::{eslint, noqa, nosec, oxlint, biome, phpstan, psalm, phpcs, clippy, sqlfluff, kind};
        suppress::install(vec![
            kind::<eslint::Suppressions>(),
            kind::<noqa::Suppressions>(),
            kind::<nosec::Suppressions>(),
            kind::<oxlint::Suppressions>(),
            kind::<biome::Suppressions>(),
            kind::<phpstan::Suppressions>(),
            kind::<psalm::Suppressions>(),
            kind::<phpcs::Suppressions>(),
            kind::<clippy::Suppressions>(),
            kind::<sqlfluff::Suppressions>(),
        ]);
    }
    // The embedders, by `Via` (`embed::Embedder`), and the program scopes an embedding gives a meaning of its own.
    embed::install_embedders(vec![
        #[cfg(feature = "lang-blade")]
        (language::Via::CompiledView, &embed::compiled_view::CompiledView),
        #[cfg(feature = "lang-html")]
        (language::Via::RawText, &embed::raw_text::RawText),
        #[cfg(all(feature = "lang-html", feature = "lang-python"))]
        (language::Via::CallArgument, &embed::call_argument::CallArgument),
        #[cfg(feature = "lang-markdown")]
        (language::Via::InlineContent, &embed::inline_content::InlineContent),
        #[cfg(all(feature = "lang-rr", feature = "lang-json"))]
        (language::Via::JsonObject, &embed::json_object::JsonObject),
        #[cfg(feature = "lang-rr")]
        (language::Via::MatchRegex, &embed::match_regex::MatchRegex),
        #[cfg(feature = "lang-rr")]
        (language::Via::Pattern, &embed::rule_pattern::RulePattern),
        #[cfg(feature = "lang-vue")]
        (language::Via::ScriptBlock, &embed::script_block::ScriptBlock),
        #[cfg(any(test, feature = "test-hooks"))]
        (language::Via::TestScript, &embed::TestScript),
    ]);
    scope::install_program_scopes(vec![
        #[cfg(feature = "lang-blade")]
        (language::Via::CompiledView, embed::compiled_view::scope),
    ]);
    // What reads W's values off a file's tape for facts' `value` relations.
    facts::core::install_value_reader(&matcher::Values);
    // The families' producers of a batch's relations: Python's f-string trees, CPython's syntax, string annotations.
    facts::vtab::syntax::install_producers(vec![
        &facts::vtab::syntax::fstring::Producer,
        &facts::vtab::syntax::cpython::Producer,
        &facts::vtab::syntax::annotation::Producer,
    ]);
    // The families' lazy relations over a batch (Python's `py.*`).
    facts::vtab::syntax::install_families(vec![&facts::vtab::syntax::py_families::Families]);
    register_family_columns();
    // The families' tables over a bound `node` relation (Python's semantic producers).
    facts::vtab::install_node_tables(vec![facts::vtab::python::register]);
    // The semantic facts a program's native steps demand (Python's).
    facts::vtab::syntax::install_semantic_producer(&facts::vtab::syntax::python_demand::Semantic);
    // What each family reads of its own core.rerail facts.
    {
        use crate::code_rule_dsl::facts::vtab::syntax::core_rr::{self, Family};
        use crate::code_rule_dsl::facts::vtab::syntax::{ecma_core, python_core, rr_core};
        core_rr::register_dialect_facts(Family::Python, &python_core::PythonFacts);
        core_rr::register_dialect_facts(Family::Ecma, &ecma_core::EcmaFacts);
        core_rr::register_dialect_facts(Family::Rr, &rr_core::RrFacts);
        core_rr::register_dialect_facts(core_rr::Family::Format, &literal::format_tape::FormatFacts);
    }
    // A workspace's project context of ECMAScript's files (manifests, project configurations, server graphs).
    workspace::ingest::install_project(|| Box::new(workspace::ingest::ecma::EcmaProject::default()));
    // A snapshot's context of ECMAScript's files (its manifests).
    eval::install_snapshot_context(eval::ecma::derive);
    // The numbers `toPrecision` reads (ECMAScript's).
    matcher::install_number_text(&value::ecma::Numbers);
    // `@to_precision`'s formatting (ECMAScript's), for the string functions below the languages.
    logic::strings::functions::install_to_precision(value::ecma::to_precision);
    // `@py_repr`'s formatting (Python's), likewise.
    logic::strings::functions::install_py_repr(|s| cpython::value::str_repr(&cpython::value::PyStr::Text(s.to_owned())));
    // The canonical AST stage's producer (Python's CPython `ast`).
    residency::install_ast_producer(&residency::admission::relational::python_ast::PythonAst);
    // What ECMAScript's import targets resolve among (`resolution`).
    resolution::install(import_target::held::RESOLUTIONS);
    // A repository's context as its index reads it (ECMAScript's `package.json` files).
    workspace::ingest::install_repository_context(workspace::ingest::ecma::repository_context);
    // The language terms (ECMAScript's `importTarget`).
    matcher::install_terms(vec![compile::import_target::TERM]);
    // Each format style's preparer (`formatArgs`): Python's, PHP's and Rust's.
    {
        use matcher::format_args::{Style, printf, python, rust, scanf};
        matcher::format_args::install_format_preparers(vec![
            (Style::Brace, &python::BRACE),
            (Style::Percent, &python::PERCENT),
            (Style::Printf, &printf::Printf),
            (Style::Scanf, &scanf::Scanf),
            (Style::Rust, &rust::Rust),
        ]);
    }
    // What compiles `resolves` over a flow (PHP's).
    compile::resolve::install_flow_resolves(compile::resolve::php::compile_php_resolves);
    // The containment relations' `nearest` solve (the file-local FIX's).
    facts::nearest::install_nearest(typing::file_local::nearest);
    // What a block of relations asks about the device: the run's binding's (decision 0342, Q58).
    facts::rows::install_block_device(logic::plan::placement::block_device);
    // What a chunk's jobs allocated less freed (Q203 item 3): the allocator's own per-thread counters.
    #[cfg(not(any(windows, target_family = "wasm", feature = "system-allocator")))]
    facts::rows::install_heap_net(heap_net);
    // And each chunk scope's heap at its most (decision 0469): jemalloc's per-thread peak.
    #[cfg(not(any(windows, target_family = "wasm", feature = "system-allocator")))]
    facts::rows::install_heap_peak(heap_peak, heap_peak_reset);
    // `core.rerail`'s names, as the rule language reads them.
    facts::core::install_core_names(logic::rr::core_shape);
    // What the evaluator keeps per connection, dropped with it.
    facts::vtab::install_forget(facts::vtab::Forget { scope: logic::native::forget_scope, engine: logic::native::forget_engine });
    // core.rerail's functors whose meaning is W's or a language's.
    logic::strings::install_functors(functors::register_functors);
    // The run's executors, by binding.
    facts::core::install_executors(logic::lanes::executor_for);
    // A tape's prefilter rows, which the push path derives.
    facts::families::install_prefilter_image(|tape, doc, interrupted| prefilter::image(tape, doc, &interrupted));
    // An owned connection's demand module.
    waves::owner::database::install_connection_setup(demand::vtab::set_up_connection);
    workspace::index::install_source_classifier(|root, store, owner, paths, registry| {
        logic::plan::corpus::source_languages::classify(root, store, owner, paths, registry)
            .map_err(|error| error.to_string())
    });
    // A snapshot's files, as an SQL session binds them.
    sql_executor::documents::install_snapshot_files(|json| {
        let (snapshot, _) = eval::parse_snapshot(json, false)?;
        Ok(snapshot.files.into_iter().map(|f| sql_executor::documents::File { path: f.path, language: f.language, source: f.source }).collect())
    });
    // The candidate compiler's audit of a compiled rule.
    let _ = rule_api::FRONTIER_AUDIT.set(prefilter::candidate_frontier_audit);
    // What compiles and launches a span pattern's prefilter (the transducer's block programs).
    facts::span_prefilter::install_span_prefilter(transducer::packed::regex_block::compile_span, transducer::packed::regex_block::launch_spans);
    // What compiles a program's `ast` matchers into the transducer wave facts runs (decision 0106).
    facts::vtab::syntax::install_ast_compiler(&facts::vtab::syntax::ast_wave::Compiler);
    // The literal grammars `parse` applies, in the order messages list them (`literal::LiteralGrammar`).
    literal::install(vec![
        literal::percent::GRAMMAR,
        literal::identifier_words::GRAMMAR,
        literal::python_fstring::GRAMMAR,
        literal::brace::GRAMMAR,
        literal::annotation::GRAMMAR,
        literal::eslint_directive::GRAMMAR,
        literal::printf::GRAMMAR,
        literal::scanf::GRAMMAR,
        literal::phpdoc::GRAMMAR,
        literal::rust_format::GRAMMAR,
        literal::regex::GRAMMAR,
    ]
    .into_iter()
    // The test grammar (`testChars`), in a test binary's boot (`install_for_tests`) alone.
    .chain(test_grammars(tests))
    .collect());
    // What a `package` file condition reads of a file without a manifest (ECMA's default Node range).
    language::set_no_manifest(&package_manifest::NoManifest);
    language::set_manifest_reader(package_manifest::read_wire);
    // The dialects their languages parse themselves (`tape::TreeParser`), each implementation registered
    // with the parser router, which takes the best this host runs (`tape::install_parsers`). A build
    // without a language's feature routes no file to it, so its parser is not linked.
    {
        let mut parsers: Vec<tape::ParserImpl> = Vec::new();
        if cfg!(feature = "lang-blade") {
            parsers.push(tape::ParserImpl { dialect: language::GrammarDialect::Blade, name: "Forte", rank: 0, runs: true, parser: &blade::tape::Forte });
        }
        if cfg!(feature = "lang-python") {
            // The lean parser (`tape::python_lean`); a file it refuses goes to tree-sitter-python.
            parsers.push(tape::ParserImpl { dialect: language::GrammarDialect::Python, name: "lean Python parser", rank: 0, runs: true, parser: &tape::python_lean::Lean });
        }
        // Python's format strings (decision 0176): guests only, parsed by Ruff's parsers' ports.
        if cfg!(feature = "lang-python") {
            parsers.push(tape::ParserImpl { dialect: language::GrammarDialect::PercentFormat, name: "percent-format strings (Ruff's parser ported)", rank: 0, runs: true, parser: { static P: literal::format_tape::FormatStrings = literal::format_tape::FormatStrings::new(language::GrammarDialect::PercentFormat); &P } });
            parsers.push(tape::ParserImpl { dialect: language::GrammarDialect::BraceFormat, name: "brace-format strings (Ruff's parser ported)", rank: 0, runs: true, parser: { static P: literal::format_tape::FormatStrings = literal::format_tape::FormatStrings::new(language::GrammarDialect::BraceFormat); &P } });
        }
        // Regular expressions in the engine's dialect (decision 0495 §2): a rule file's `match` regexes, and any
        // string a rule parses as one.
        parsers.push(tape::ParserImpl { dialect: language::GrammarDialect::Regex, name: "regexes (the Rust dialect, hand-written)", rank: 0, runs: true, parser: { static P: literal::format_tape::FormatStrings = literal::format_tape::FormatStrings::new(language::GrammarDialect::Regex); &P } });
        // ESLint's directive comments (decision 0194): guests only, parsed by main's directive parser.
        if cfg!(feature = "lang-typescript") {
            parsers.push(tape::ParserImpl { dialect: language::GrammarDialect::EslintDirective, name: "ESLint directive comments", rank: 0, runs: true, parser: { static P: literal::format_tape::FormatStrings = literal::format_tape::FormatStrings::new(language::GrammarDialect::EslintDirective); &P } });
        }
        // The lean ECMAScript parser (`tape::ecma_lean`, decision 0184); a file it refuses goes to
        // the dialect's tree-sitter grammar.
        if cfg!(feature = "lang-javascript") {
            parsers.push(tape::ParserImpl { dialect: language::GrammarDialect::JavaScript, name: "lean JavaScript parser", rank: 0, runs: true, parser: &tape::ecma_lean::JAVASCRIPT });
        }
        if cfg!(feature = "lang-typescript") {
            parsers.push(tape::ParserImpl { dialect: language::GrammarDialect::TypeScript, name: "lean TypeScript parser", rank: 0, runs: true, parser: &tape::ecma_lean::TYPESCRIPT });
            parsers.push(tape::ParserImpl { dialect: language::GrammarDialect::Tsx, name: "lean TSX parser", rank: 0, runs: true, parser: &tape::ecma_lean::TSX });
        }
        if cfg!(feature = "lang-rust") {
            // The lean Rust parser (`tape::rust_lean`, decision 0193); a file it refuses goes to
            // tree-sitter-rust.
            parsers.push(tape::ParserImpl { dialect: language::GrammarDialect::Rust, name: "lean Rust parser", rank: 0, runs: true, parser: &tape::rust_lean::Lean });
        }
        if cfg!(feature = "lang-java") {
            // The lean Java parser (`tape::java_lean`, decision 0195); a file it refuses goes to
            // tree-sitter-java.
            parsers.push(tape::ParserImpl { dialect: language::GrammarDialect::Java, name: "lean Java parser", rank: 0, runs: true, parser: &tape::java_lean::Lean });
        }
        if cfg!(feature = "lang-go") {
            // The lean Go parser (`tape::go_lean`, decision 0314); a file it refuses goes to
            // tree-sitter-go.
            parsers.push(tape::ParserImpl { dialect: language::GrammarDialect::Go, name: "lean Go parser", rank: 0, runs: true, parser: &tape::go_lean::Lean });
        }
        if cfg!(feature = "lang-c") {
            // The lean C parser (`tape::c_lean`, decision 0317); a file it refuses goes to
            // tree-sitter-c.
            parsers.push(tape::ParserImpl { dialect: language::GrammarDialect::C, name: "lean C parser", rank: 0, runs: true, parser: &tape::c_lean::Lean });
        }
        if cfg!(feature = "lang-cpp") {
            // The lean C++ parser (`tape::c_lean`, decision 0317); a file it refuses goes to
            // tree-sitter-cpp.
            parsers.push(tape::ParserImpl { dialect: language::GrammarDialect::Cpp, name: "lean C++ parser", rank: 0, runs: true, parser: &tape::c_lean::Lean });
        }
        if cfg!(feature = "lang-csharp") {
            // The lean C# parser (`tape::csharp_lean`, decision 0196); a file it refuses goes to
            // tree-sitter-c-sharp.
            parsers.push(tape::ParserImpl { dialect: language::GrammarDialect::CSharp, name: "lean C# parser", rank: 0, runs: true, parser: &tape::csharp_lean::Lean });
        }
        // The lean Kotlin, Swift and Scala parsers (`tape::kotlin_lean`, `swift_lean`, `scala_lean`,
        // decisions 0305 and 0326); a file one refuses goes to its tree-sitter grammar.
        if cfg!(feature = "lang-kotlin") {
            parsers.push(tape::ParserImpl { dialect: language::GrammarDialect::Kotlin, name: "lean Kotlin parser", rank: 0, runs: true, parser: &tape::kotlin_lean::Lean });
        }
        if cfg!(feature = "lang-swift") {
            parsers.push(tape::ParserImpl { dialect: language::GrammarDialect::Swift, name: "lean Swift parser", rank: 0, runs: true, parser: &tape::swift_lean::Lean });
        }
        if cfg!(feature = "lang-scala") {
            parsers.push(tape::ParserImpl { dialect: language::GrammarDialect::Scala, name: "lean Scala parser", rank: 0, runs: true, parser: &tape::scala_lean::Lean });
        }
        if cfg!(feature = "lang-php") {
            // The lean PHP parser (`tape::php_lean`, decision 0198); a file it refuses goes to
            // tree-sitter-php.
            parsers.push(tape::ParserImpl { dialect: language::GrammarDialect::Php, name: "lean PHP parser", rank: 0, runs: true, parser: &tape::php_lean::Lean });
        }
        if cfg!(feature = "lang-html") {
            // The lean HTML parser (`tape::html_lean`, decision 0199); a file it refuses goes to
            // tree-sitter-html.
            parsers.push(tape::ParserImpl { dialect: language::GrammarDialect::Html, name: "lean HTML parser", rank: 0, runs: true, parser: &tape::html_lean::Lean });
        }
        if cfg!(feature = "lang-css") {
            // The lean CSS parser (`tape::css_lean`, decision 0200); a file it refuses goes to
            // tree-sitter-css.
            parsers.push(tape::ParserImpl { dialect: language::GrammarDialect::Css, name: "lean CSS parser", rank: 0, runs: true, parser: &tape::css_lean::Lean });
        }
        if cfg!(feature = "lang-markdown") {
            // The lean Markdown parser (`tape::markdown_lean`, decision 0237); a file it refuses
            // goes to tree-sitter-md.
            parsers.push(tape::ParserImpl { dialect: language::GrammarDialect::Markdown, name: "lean Markdown parser", rank: 0, runs: true, parser: &tape::markdown_lean::Lean });
            // The lean parser of Markdown's inline grammar (`tape::markdown_inline_lean`, decision
            // 0234), for the inline texts Markdown's tapes embed; a text it refuses goes to
            // tree-sitter-md-inline.
            parsers.push(tape::ParserImpl { dialect: language::GrammarDialect::MarkdownInline, name: "lean Markdown inline parser", rank: 0, runs: true, parser: &tape::markdown_inline_lean::Lean });
        }
        if cfg!(feature = "lang-sql") {
            // The lean SQL parser (`tape::sql_lean`, decision 0214); a file it refuses goes to
            // tree-sitter-sql.
            parsers.push(tape::ParserImpl { dialect: language::GrammarDialect::Sql, name: "lean SQL parser", rank: 0, runs: true, parser: &tape::sql_lean::Lean });
        }
        if cfg!(feature = "lang-vue") {
            // The lean Vue parser (`tape::vue_lean`, decision 0242), whose tape the component's
            // embedded program is built from; a file it refuses goes to tree-sitter-vue.
            parsers.push(tape::ParserImpl { dialect: language::GrammarDialect::Vue, name: "lean Vue parser", rank: 0, runs: true, parser: &tape::vue_lean::Lean });
        }
        if cfg!(feature = "lang-json") {
            // simdjson where it builds (not wasm: tree-sitter-json there; on MSVC it is compiled by
            // clang-cl); the Bitlex parser (`json_parser::tape::Bitlex`) is not yet the product's.
            #[cfg(not(target_family = "wasm"))]
            parsers.extend(transducer::packed::json_parser::simdjson::registrations());
        }
        tape::install_parsers(parsers);
    }
    // Every family the build has, registered (`pack` refuses one that is not).
    Family::ALL.into_iter().for_each(|family| _ = pack(family));
}

/// The engine's devices, into the logic's and the transducer's registries (`logic::device`,
/// `transducer::packed::device`), and the transducer's compiled CPU executor (`transducer::packed::CPU`). Called as
/// the library loads, before any run.
pub fn install_devices() {
    transducer::packed::native_cpu::register();
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    device_metal::register();
}

/// This thread's bytes allocated less freed, from jemalloc's per-thread counters (`facts::rows::install_heap_net`).
#[cfg(not(any(windows, target_family = "wasm", feature = "system-allocator")))]
fn heap_net() -> i64 {
    thread_local! {
        static COUNTERS: Option<(tikv_jemalloc_ctl::thread::ThreadLocal<u64>, tikv_jemalloc_ctl::thread::ThreadLocal<u64>)> =
            tikv_jemalloc_ctl::thread::allocatedp::read().ok().zip(tikv_jemalloc_ctl::thread::deallocatedp::read().ok());
    }
    COUNTERS.with(|c| (*c).map_or(0, |(a, d)| a.get() as i64 - d.get() as i64))
}

/// jemalloc's `thread.peak.read` and `thread.peak.reset`, by their MIBs (looked up once; `None` where the allocator
/// has no such control).
#[cfg(not(any(windows, target_family = "wasm", feature = "system-allocator")))]
fn peak_mibs() -> Option<([usize; 3], [usize; 3])> {
    static MIBS: std::sync::OnceLock<Option<([usize; 3], [usize; 3])>> = std::sync::OnceLock::new();
    *MIBS.get_or_init(|| {
        let (mut read, mut reset) = ([0; 3], [0; 3]);
        tikv_jemalloc_ctl::raw::name_to_mib(b"thread.peak.read\0", &mut read).ok()?;
        tikv_jemalloc_ctl::raw::name_to_mib(b"thread.peak.reset\0", &mut reset).ok()?;
        Some((read, reset))
    })
}

/// This thread's most allocated less freed since its last `heap_peak_reset`, above what it was then
/// (`facts::rows::install_heap_peak`).
#[cfg(not(any(windows, target_family = "wasm", feature = "system-allocator")))]
fn heap_peak() -> i64 {
    // SAFETY: `thread.peak.read` is a `uint64_t`.
    peak_mibs().and_then(|(read, _)| unsafe { tikv_jemalloc_ctl::raw::read_mib::<u64>(&read) }.ok()).map_or(0, |v| v as i64)
}

/// Resets this thread's peak (`heap_peak`) to what it holds now.
#[cfg(not(any(windows, target_family = "wasm", feature = "system-allocator")))]
fn heap_peak_reset() {
    if let Some((_, reset)) = peak_mibs() {
        // SAFETY: `thread.peak.reset` reads and writes nothing: no old or new value is passed.
        unsafe { tikv_jemalloc_sys::mallctlbymib(reset.as_ptr(), reset.len(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), 0) };
    }
}

/// This crate's own test binary, which the engine does not load: booted as it loads, as the engine boots, so no
/// test meets an unregistered pack, device or SQL function.
#[cfg(test)]
mod test_load {
    extern "C" fn load() {
        super::install_for_tests();
    }
    #[used]
    #[cfg_attr(target_vendor = "apple", unsafe(link_section = "__DATA,__mod_init_func"))]
    #[cfg_attr(windows, unsafe(link_section = ".CRT$XCU"))]
    #[cfg_attr(not(any(target_vendor = "apple", windows)), unsafe(link_section = ".init_array"))]
    static LOAD: extern "C" fn() = load;
}
