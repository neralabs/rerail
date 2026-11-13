    use super::*;
    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::tape::{Grammar, TapeImage};
    use ast_grep_core::AstGrep;
    use ast_grep_core::matcher::Pattern;
    use ast_grep_core::tree_sitter::StrDoc;
    use ast_grep_language::LanguageExt;
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use std::fs;
    use std::hint::black_box;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    fn tape(source: &str) -> TapeDoc {
        let grammar = Grammar::new(GrammarDialect::Python).unwrap();
        let native = StrDoc::new(source, GrammarDialect::Python);
        let image =
            TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap();
        TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap()
    }

    fn identifier_program(text: &str) -> Program {
        let pattern = Pattern::try_new(text, GrammarDialect::Python).unwrap();
        Program::compile_ast_grep_literal(&pattern).unwrap()
    }

    #[test]
    fn fixed_words_execute_identifier_transducer() {
        let tape = tape("execute(x)\nother()\nexecute(y)\n");
        let program = identifier_program("execute");
        assert!(
            program
                .words
                .iter()
                .all(|word| word.to_le_bytes().len() == 4)
        );
        let output = program.execute(&tape).unwrap();
        assert_eq!(output.len(), 2);
        assert_eq!(output[0].captures[0].span, 0..7);
        assert_eq!(output[1].captures[0].span, 19..26);
    }

    #[test]
    fn navigation_captures_eval_call_and_callee() {
        let grammar = GrammarDialect::Python.get_ts_language();
        let call = grammar.id_for_node_kind("call", true);
        let identifier = grammar.id_for_node_kind("identifier", true);
        let function = grammar.field_id_for_name("function").unwrap().get();
        let program = Program::new(
            vec![
                word(KIND, 0, call),
                word(CAPTURE, 0, 0),
                word(CHILD, 0, 0),
                word(FIELD, 0, function),
                word(KIND, 0, identifier),
                word(TEXT, 0, 0),
                word(CAPTURE, 0, 1),
                word(EMIT, 0, 2),
            ],
            vec![
                b"eval".to_vec(),
                b"call".to_vec(),
                b"callee".to_vec(),
                b"match".to_vec(),
            ],
            vec![TextPredicate::Equal(0)],
            vec![1, 2, 3],
            vec![],
        )
        .unwrap();
        let output = program
            .execute(&tape("eval(x)\nobj.eval(x)\nother(x)\n"))
            .unwrap();
        assert_eq!(output.len(), 1);
        assert_eq!(output[0].captures[0].span, 0..7);
        assert_eq!(output[0].captures[1].span, 0..4);
    }

    #[test]
    fn alternatives_do_not_share_capture_mutation() {
        let program = Program::new(
            vec![
                word(
                    KIND,
                    0,
                    GrammarDialect::Python
                        .get_ts_language()
                        .id_for_node_kind("identifier", true),
                ),
                word(FORK, 0, 4),
                word(CAPTURE, 0, 0),
                word(EMIT, 0, 1),
                word(EMIT, 0, 1),
            ],
            vec![b"captured".to_vec(), b"match".to_vec()],
            vec![],
            vec![0, 1],
            vec![],
        )
        .unwrap();
        let output = program.execute(&tape("name\n")).unwrap();
        assert_eq!(output.len(), 2);
        assert_eq!(output[0].captures.len(), 1);
        assert!(output[1].captures.is_empty());
    }

    #[test]
    fn text_predicate_variants_use_declared_semantics() {
        let tape = tape("execute\n");
        for (predicate, source) in [
            (TextPredicate::Prefix(0), b"exec".to_vec()),
            (
                TextPredicate::Regex {
                    source: 0,
                    compiled: Regex::new("^exec(?:ute)?$").unwrap(),
                },
                b"^exec(?:ute)?$".to_vec(),
            ),
        ] {
            let program = Program::new(
                vec![
                    word(
                        KIND,
                        0,
                        GrammarDialect::Python
                            .get_ts_language()
                            .id_for_node_kind("identifier", true),
                    ),
                    word(TEXT, 0, 0),
                    word(EMIT, 0, 0),
                ],
                vec![source, b"match".to_vec()],
                vec![predicate],
                vec![1],
                vec![],
            )
            .unwrap();
            assert_eq!(program.execute(&tape).unwrap().len(), 1);
        }
    }

    #[test]
    fn traversal_flags_sizes_and_long_table_execute() {
        let grammar = GrammarDialect::Python.get_ts_language();
        let identifier = grammar.id_for_node_kind("identifier", true);
        let program = Program::new(
            vec![
                word(DESCEND, NAMED, 0),
                word(KIND, 0, identifier),
                word(FLAGS, 1, 0b1110),
                word(SIZE, 0x82, 0),
                word(CAPTURE, 0, 0),
                word(NEXT, NAMED, 0),
                word(PARENT, 0, 0),
                word(EMIT, 0, 1),
            ],
            vec![b"identifier".to_vec(), b"match".to_vec()],
            vec![],
            vec![0, 1],
            vec![100_000],
        )
        .unwrap();
        // Every identifier is shorter than the long constant. Only identifiers with a later
        // named sibling survive NEXT, and PARENT returns their shared syntax parent.
        let output = program.execute(&tape("pair = (alpha, beta)\n")).unwrap();
        assert!(!output.is_empty());
        assert!(
            output
                .iter()
                .all(|item| item.captures[0].span.len() <= 100_000)
        );
    }

    #[test]
    fn backward_goto_requires_and_uses_monotonic_next() {
        let identifier = GrammarDialect::Python
            .get_ts_language()
            .id_for_node_kind("identifier", true);
        let program = Program::new(
            vec![
                word(KIND, 0, identifier),
                word(CAPTURE, 0, 0),
                word(FORK, 0, 5),
                word(NEXT, NAMED, 0),
                word(GOTO, 0, 0),
                word(EMIT, 0, 1),
            ],
            vec![b"item".to_vec(), b"sequence".to_vec()],
            vec![],
            vec![0, 1],
            vec![],
        )
        .unwrap();
        let tape = tape("f(a, b, c)\n");
        let root = (0..tape.transducer_len() as u32)
            .find(|&node| tape.transducer_text_str(node).as_bytes() == b"a")
            .unwrap();
        let output = program.execute_roots(&tape, [root]).unwrap();
        assert_eq!(output.len(), 3);
        assert_eq!(output[0].captures.len(), 3);
        assert_eq!(output[1].captures.len(), 2);
        assert_eq!(output[2].captures.len(), 1);
    }

    #[test]
    fn batch_kind_dispatch_matches_individual_execution() {
        let tape = tape("execute(); commit(); execute()\n");
        let programs = [identifier_program("execute"), identifier_program("commit")];
        let individual = programs
            .iter()
            .map(|program| program.execute(&tape).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(execute_many(&programs, &tape).unwrap(), individual);
    }

    #[test]
    fn batch_runs_many_programs_files_and_exact_anchor_sets() {
        let files = [tape("execute(); commit()\n"), tape("commit(); execute()\n")];
        let programs = [identifier_program("execute"), identifier_program("commit")];
        let unanchored = execute_batch(&programs, &files, None).unwrap();
        assert_eq!(unanchored.len(), 2);
        assert!(
            unanchored
                .iter()
                .all(|file| file.iter().all(|matches| matches.len() == 1))
        );
        let anchors = files
            .iter()
            .map(|file| {
                programs
                    .iter()
                    .map(|program| {
                        let (_, _, kind) = split(program.words[0]);
                        (0..file.transducer_len() as u32)
                            .filter(|&node| file.transducer_kind(node) == kind)
                            .collect()
                    })
                    .collect()
            })
            .collect::<Vec<Vec<Vec<u32>>>>();
        assert_eq!(
            execute_batch(&programs, &files, Some(&anchors)).unwrap(),
            unanchored
        );
        let empty = vec![vec![Vec::new(), Vec::new()], vec![Vec::new(), Vec::new()]];
        assert!(
            execute_batch(&programs, &files, Some(&empty))
                .unwrap()
                .iter()
                .flatten()
                .all(Vec::is_empty)
        );
        let invalid = vec![
            vec![vec![u32::MAX], Vec::new()],
            vec![Vec::new(), Vec::new()],
        ];
        assert_eq!(
            execute_batch(&programs, &files, Some(&invalid)),
            Err(Error::InvalidAnchor)
        );
    }

    #[test]
    fn selector_index_persists_only_the_kind_family() {
        let database = std::env::temp_dir().join(format!(
            "rerail-kind-index-{}-{}.sqlite",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = crate::content_store::ContentStore::open(&database).unwrap();
        let files = [tape("execute();\n"), tape("commit();\n")];
        let source = store.put_source(b"source", &[]).unwrap();
        let tape_ids = [
            store.put_tape(source, 1, 1, 1, b"first").unwrap(),
            store.put_tape(source, 2, 1, 1, b"second").unwrap(),
        ];
        crate::content_store::reset_search_index_family_writes();
        persist_search_index_batch(&store, &tape_ids, &files).unwrap();
        assert_eq!(crate::content_store::search_index_family_writes(), 2);
        drop(store);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", database.display()));
        }
    }

    #[test]
    fn verifier_rejects_invalid_and_non_progressing_programs() {
        assert!(matches!(
            Program::new(vec![word(GOTO, 0, 0)], vec![], vec![], vec![], vec![]),
            Err(Error::NonProgressingCycle(0))
        ));
        assert!(matches!(
            Program::new(
                vec![word(TEXT, 0, 0), word(STOP, 0, 0)],
                vec![],
                vec![],
                vec![],
                vec![]
            ),
            Err(Error::InvalidOperand(0))
        ));
        assert!(matches!(
            Program::new(
                vec![word(CHILD, 2, 0), word(STOP, 0, 0)],
                vec![],
                vec![],
                vec![],
                vec![]
            ),
            Err(Error::InvalidMode(0))
        ));
        assert!(matches!(
            Program::new(
                vec![word(GOTO, 0, 2), word(NEXT, 0, 0), word(GOTO, 0, 0)],
                vec![],
                vec![],
                vec![],
                vec![]
            ),
            Err(Error::NonProgressingCycle(_))
        ));
        assert!(matches!(
            Program::new(
                vec![word(DESCEND, 0, 0), word(PARENT, 0, 0), word(GOTO, 0, 0)],
                vec![],
                vec![],
                vec![],
                vec![]
            ),
            Err(Error::NonProgressingCycle(_))
        ));
    }

    fn split_tree_sitter_patterns(text: &str) -> Vec<String> {
        let bytes = text.as_bytes();
        let mut patterns = Vec::new();
        let mut depth = 0_i32;
        let mut start = None;
        let mut index = 0;
        while index < bytes.len() {
            match bytes[index] {
                b';' => {
                    index += 1;
                    while index < bytes.len() && bytes[index] != b'\n' {
                        index += 1;
                    }
                    continue;
                }
                b'"' => {
                    if depth == 0
                        && let Some(start) = start.replace(index)
                    {
                        patterns.push(text[start..index].trim().to_owned());
                    }
                    index += 1;
                    while index < bytes.len() && bytes[index] != b'"' {
                        if bytes[index] == b'\\' {
                            index += 1;
                        }
                        index += 1;
                    }
                }
                b'(' | b'[' => {
                    if depth == 0
                        && let Some(start) = start.replace(index)
                    {
                        patterns.push(text[start..index].trim().to_owned());
                    }
                    depth += 1;
                }
                b')' | b']' => depth -= 1,
                _ => {}
            }
            index += 1;
        }
        if let Some(start) = start {
            patterns.push(text[start..].trim().to_owned());
        }
        patterns.retain(|pattern| !pattern.is_empty());
        patterns
    }

    fn collect_extension_files(root: &Path, extensions: &[&str], output: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(root) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                if path.file_name().is_none_or(|name| name != ".git") {
                    collect_extension_files(&path, extensions, output);
                }
            } else if file_type.is_file()
                && path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extensions.contains(&extension))
                && fs::metadata(&path).is_ok_and(|metadata| metadata.len() <= 4_000_000)
            {
                output.push(path);
            }
        }
    }

    /// The Python tree the transducer benchmarks read: `RERAIL_TRANSDUCER_CORPUS`. Without it they
    /// skip, saying so.
    fn transducer_corpus() -> Option<PathBuf> {
        let corpus = std::env::var_os("RERAIL_TRANSDUCER_CORPUS").map(PathBuf::from);
        if corpus.is_none() {
            eprintln!(
                "skipped: set RERAIL_TRANSDUCER_CORPUS to a directory of Python sources to benchmark"
            );
        }
        corpus
    }

    fn collect_python(root: &Path, output: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(root) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if !matches!(
                    path.file_name().and_then(|s| s.to_str()),
                    Some(".git" | ".venv" | "node_modules" | "__pycache__")
                ) {
                    collect_python(&path, output);
                }
            } else if path.extension().and_then(|s| s.to_str()) == Some("py") {
                output.push(path);
            }
        }
    }

    fn digest_ranges<'a>(ranges: impl IntoIterator<Item = &'a Range<usize>>) -> String {
        let mut digest = Sha256::new();
        for range in ranges {
            digest.update((range.start as u64).to_le_bytes());
            digest.update((range.end as u64).to_le_bytes());
        }
        crate::code_rule_dsl::prof::lower_hex(&digest.finalize())
    }

    fn median(mut values: Vec<Duration>) -> Duration {
        values.sort_unstable();
        values[values.len() / 2]
    }

    #[test]
    #[ignore = "controlled corpus benchmark"]
    fn benchmark_three_ast_grep_paths() {
        let Some(root) = transducer_corpus() else {
            return;
        };
        let limit = std::env::var("RERAIL_TRANSDUCER_FILES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(usize::MAX);
        let iterations = std::env::var("RERAIL_TRANSDUCER_ITERATIONS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(7usize);
        let names = [
            "execute", "commit", "rollback", "flush", "select", "where", "join", "delete",
        ];
        let patterns = names
            .iter()
            .map(|name| Pattern::try_new(name, GrammarDialect::Python).unwrap())
            .collect::<Vec<_>>();
        let programs = names
            .iter()
            .map(|name| identifier_program(name))
            .collect::<Vec<_>>();
        let mut paths = Vec::new();
        collect_python(&root, &mut paths);
        paths.sort();
        paths.truncate(limit);
        assert!(
            !paths.is_empty(),
            "benchmark corpus is empty: {}",
            root.display()
        );

        let grammar = Grammar::new(GrammarDialect::Python).unwrap();
        let preparation = Instant::now();
        let mut native = Vec::with_capacity(paths.len());
        let mut tape_roots = Vec::with_capacity(paths.len());
        let mut tapes = Vec::with_capacity(paths.len());
        let mut source_bytes = 0usize;
        let mut parse_time = Duration::ZERO;
        let mut encode_restore_time = Duration::ZERO;
        for path in &paths {
            let source = fs::read_to_string(path).unwrap();
            source_bytes += source.len();
            let started = Instant::now();
            let doc = StrDoc::new(&source, GrammarDialect::Python);
            parse_time += started.elapsed();
            let started = Instant::now();
            let image =
                TapeImage::encode(doc.tree.root_node(), &grammar, &source, &|| false).unwrap();
            let tape = TapeDoc::restore(Arc::new(source), Arc::clone(&grammar), &image, &|| false)
                .unwrap();
            encode_restore_time += started.elapsed();
            native.push(AstGrep::doc(doc));
            tape_roots.push(AstGrep::doc(tape.clone()));
            tapes.push(tape);
        }
        let preparation_time = preparation.elapsed();

        let native_run = || {
            let mut ranges = Vec::new();
            for root in &native {
                for pattern in &patterns {
                    ranges.extend(root.root().find_all(pattern).map(|m| m.get_node().range()));
                }
            }
            ranges
        };
        let tape_run = || {
            let mut ranges = Vec::new();
            for root in &tape_roots {
                for pattern in &patterns {
                    ranges.extend(root.root().find_all(pattern).map(|m| m.get_node().range()));
                }
            }
            ranges
        };
        let transducer_run = || {
            let mut ranges = Vec::new();
            for tape in &tapes {
                for output in execute_many(&programs, tape).unwrap() {
                    ranges.extend(output.into_iter().map(|m| m.captures[0].span.clone()));
                }
            }
            ranges
        };

        let expected = native_run();
        let tape_result = tape_run();
        let transducer_result = transducer_run();
        assert_eq!(tape_result, expected, "tape ast-grep parity failed");
        assert_eq!(transducer_result, expected, "transducer parity failed");
        let digest = digest_ranges(&expected);

        let mut native_times = Vec::new();
        let mut tape_times = Vec::new();
        let mut transducer_times = Vec::new();
        for iteration in 0..=iterations {
            let order = if iteration % 2 == 0 {
                [0, 1, 2]
            } else {
                [2, 1, 0]
            };
            for engine in order {
                let started = Instant::now();
                let count = match engine {
                    0 => native_run().len(),
                    1 => tape_run().len(),
                    _ => transducer_run().len(),
                };
                black_box(count);
                let elapsed = started.elapsed();
                if iteration > 0 {
                    match engine {
                        0 => native_times.push(elapsed),
                        1 => tape_times.push(elapsed),
                        _ => transducer_times.push(elapsed),
                    }
                }
            }
        }
        let native_median = median(native_times);
        let tape_median = median(tape_times);
        let transducer_median = median(transducer_times);
        let ms = |duration: Duration| duration.as_secs_f64() * 1000.0;
        let report = json!({
            "benchmark": "rerail-fixed-word-transducer-v1",
            "corpus": fs::canonicalize(&root).unwrap(),
            "files": paths.len(), "sourceBytes": source_bytes, "patterns": names,
            "matches": expected.len(), "orderedRangeSha256": digest,
            "iterations": iterations,
            "preparationMs": ms(preparation_time), "parseMs": ms(parse_time),
            "tapeEncodeRestoreMs": ms(encode_restore_time),
            "medianMs": {
                "astGrepTree": ms(native_median),
                "astGrepTape": ms(tape_median),
                "transducerTape": ms(transducer_median),
            },
            "speedupVsTree": ms(native_median) / ms(transducer_median),
            "speedupVsTapeAstGrep": ms(tape_median) / ms(transducer_median),
        });
        println!("TRANSDUCER_BENCHMARK={report}");
    }

    #[test]
    #[ignore = "full ast-index-spike tree-sitter query oracle benchmark"]
    fn benchmark_tree_sitter_query_oracle() {
        use tree_sitter::StreamingIterator;

        let query_root = PathBuf::from(
            std::env::var("RERAIL_QUERY_CORPUS")
                .expect("RERAIL_QUERY_CORPUS must name ast-index-spike bench/corpus/queries"),
        );
        let source_root = PathBuf::from(
            std::env::var("RERAIL_SOURCE_CORPUS")
                .expect("RERAIL_SOURCE_CORPUS must name ast-index-spike bench/corpus/repos"),
        );
        let languages: &[(&str, GrammarDialect, &[&str])] = &[
            ("python", GrammarDialect::Python, &["py"]),
            (
                "javascript",
                GrammarDialect::JavaScript,
                &["js", "mjs", "cjs", "jsx"],
            ),
            ("typescript", GrammarDialect::TypeScript, &["ts"]),
            ("tsx", GrammarDialect::Tsx, &["tsx"]),
            ("java", GrammarDialect::Java, &["java"]),
            ("csharp", GrammarDialect::CSharp, &["cs"]),
            (
                "cpp",
                GrammarDialect::Cpp,
                &["cc", "cpp", "cxx", "hpp", "hh", "hxx"],
            ),
            ("c", GrammarDialect::C, &["c", "h"]),
            ("php", GrammarDialect::Php, &["php"]),
            ("bash", GrammarDialect::Bash, &["sh", "bash", "zsh"]),
            ("ruby", GrammarDialect::Ruby, &["rb"]),
            ("go", GrammarDialect::Go, &["go"]),
            ("rust", GrammarDialect::TestRust, &["rs"]),
            ("kotlin", GrammarDialect::Kotlin, &["kt", "kts"]),
            ("scala", GrammarDialect::Scala, &["scala", "sc"]),
            ("swift", GrammarDialect::Swift, &["swift"]),
            ("html", GrammarDialect::Html, &["html", "htm"]),
            ("css", GrammarDialect::Css, &["css"]),
            ("lua", GrammarDialect::Lua, &["lua"]),
            ("json", GrammarDialect::Json, &["json"]),
        ];
        let selected_languages = std::env::var("RERAIL_QUERY_LANGS")
            .ok()
            .map(|value| value.split(',').map(str::to_owned).collect::<BTreeSet<_>>());
        let query_filter = std::env::var("RERAIL_QUERY_FILTER").ok();
        let source_limit = std::env::var("RERAIL_QUERY_SOURCE_BYTES")
            .ok()
            .map(|value| value.parse::<usize>().unwrap())
            .unwrap_or(1_000_000);
        let shard_count = std::env::var("RERAIL_QUERY_SHARD_COUNT")
            .ok()
            .map(|value| value.parse::<usize>().unwrap())
            .unwrap_or(1);
        let shard_index = std::env::var("RERAIL_QUERY_SHARD_INDEX")
            .ok()
            .map(|value| value.parse::<usize>().unwrap())
            .unwrap_or(0);
        assert!(shard_count > 0 && shard_index < shard_count);
        let mut all_micros = Vec::new();
        let mut per_language = serde_json::Map::new();
        let mut total_matches = 0usize;
        let mut deadline_violations = 0usize;
        let mut slow_queries = Vec::new();
        for &(name, language, extensions) in languages {
            if selected_languages
                .as_ref()
                .is_some_and(|selected| !selected.contains(name))
            {
                continue;
            }
            let mut patterns = BTreeSet::new();
            let language_query_root = if name == "tsx" {
                query_root.join("typescript")
            } else {
                query_root.join(name)
            };
            for entry in fs::read_dir(language_query_root).unwrap() {
                let path = entry.unwrap().path();
                if path.extension().is_some_and(|extension| extension == "scm") {
                    patterns.extend(split_tree_sitter_patterns(
                        &fs::read_to_string(path).unwrap(),
                    ));
                }
            }
            let grammar = language.get_ts_language();
            let queries = patterns
                .iter()
                .filter(|pattern| {
                    query_filter
                        .as_ref()
                        .is_none_or(|filter| pattern.contains(filter))
                })
                .filter_map(|pattern| {
                    tree_sitter::Query::new(&grammar, pattern)
                        .ok()
                        .map(|query| (pattern, query))
                })
                .collect::<Vec<_>>();
            let mut paths = Vec::new();
            let language_source_root = if name == "tsx" {
                source_root.join("typescript")
            } else {
                source_root.join(name)
            };
            collect_extension_files(&language_source_root, extensions, &mut paths);
            paths.sort();
            let mut parser = tree_sitter::Parser::new();
            parser.set_language(&grammar).unwrap();
            let mut documents = Vec::new();
            let mut source_bytes = 0usize;
            for (ordinal, path) in paths.into_iter().enumerate() {
                if ordinal % shard_count != shard_index {
                    continue;
                }
                let Ok(source) = fs::read(path) else { continue };
                if source.is_empty() || source_bytes + source.len() > source_limit {
                    continue;
                }
                if let Some(tree) = parser.parse(&source, None) {
                    source_bytes += source.len();
                    documents.push((source, tree));
                }
            }
            let language_started = Instant::now();
            let mut language_matches = 0usize;
            for (pattern, query) in &queries {
                let query_started = Instant::now();
                for (source, tree) in &documents {
                    let mut cursor = tree_sitter::QueryCursor::new();
                    let mut matches = cursor.matches(query, tree.root_node(), source.as_slice());
                    while let Some(found) = matches.next() {
                        language_matches += 1;
                        std::hint::black_box(found.captures().len());
                    }
                }
                let micros = query_started.elapsed().as_micros() as u64;
                if micros > 500_000 {
                    deadline_violations += 1;
                    slow_queries.push(json!({"language":name,"milliseconds":micros as f64/1_000.0,
                                            "pattern":pattern}));
                }
                all_micros.push(micros);
            }
            total_matches += language_matches;
            per_language.insert(
                name.to_owned(),
                json!({"queries":queries.len(),"files":documents.len(),"sourceBytes":source_bytes,
                       "matches":language_matches,"seconds":language_started.elapsed().as_secs_f64()}),
            );
        }
        all_micros.sort_unstable();
        let percentile = |numerator: usize, denominator: usize| {
            all_micros[(all_micros.len() - 1) * numerator / denominator] as f64 / 1_000.0
        };
        println!(
            "TREE_SITTER_QUERY_ORACLE_BENCHMARK={}",
            json!({"queries":all_micros.len(),"matches":total_matches,
                   "medianMs":percentile(1,2),"p95Ms":percentile(95,100),
                   "maxMs":all_micros.last().copied().unwrap_or_default() as f64/1_000.0,
                   "deadlineViolations":deadline_violations,"slowQueries":slow_queries,
                   "languages":per_language,
                   "unsupported":["SQL grammar","Rerail transducer compilation of .scm queries"]})
        );
    }

    #[test]
    #[ignore = "full ast-index-spike tree-sitter query corpus audit"]
    fn audit_tree_sitter_query_corpus() {
        let query_root = PathBuf::from(
            std::env::var("RERAIL_QUERY_CORPUS")
                .expect("RERAIL_QUERY_CORPUS must name ast-index-spike bench/corpus/queries"),
        );
        let languages = [
            ("python", GrammarDialect::Python),
            ("javascript", GrammarDialect::JavaScript),
            ("typescript", GrammarDialect::TypeScript),
            ("java", GrammarDialect::Java),
            ("csharp", GrammarDialect::CSharp),
            ("cpp", GrammarDialect::Cpp),
            ("c", GrammarDialect::C),
            ("php", GrammarDialect::Php),
            ("bash", GrammarDialect::Bash),
            ("ruby", GrammarDialect::Ruby),
            ("go", GrammarDialect::Go),
            ("rust", GrammarDialect::TestRust),
            ("kotlin", GrammarDialect::Kotlin),
            ("scala", GrammarDialect::Scala),
            ("swift", GrammarDialect::Swift),
            ("html", GrammarDialect::Html),
            ("css", GrammarDialect::Css),
            ("lua", GrammarDialect::Lua),
            ("json", GrammarDialect::Json),
        ];
        let mut report = serde_json::Map::new();
        let mut corpus = Vec::new();
        let mut total = 0usize;
        for (name, language) in languages {
            let mut patterns = BTreeSet::new();
            for entry in fs::read_dir(query_root.join(name)).unwrap() {
                let path = entry.unwrap().path();
                if path.extension().is_some_and(|extension| extension == "scm") {
                    patterns.extend(split_tree_sitter_patterns(
                        &fs::read_to_string(path).unwrap(),
                    ));
                }
            }
            let grammar = language.get_ts_language();
            let mut valid = 0usize;
            let mut invalid = 0usize;
            for pattern in &patterns {
                match tree_sitter::Query::new(&grammar, pattern) {
                    Ok(query) if query.pattern_count() == 1 => {
                        valid += 1;
                        corpus.push(json!({"lang":name,"text":pattern}));
                    }
                    _ => invalid += 1,
                }
            }
            total += valid;
            report.insert(
                name.to_owned(),
                json!({"attempted": patterns.len(), "valid": valid, "invalid": invalid,
                       "grammarSymbols": grammar.node_kind_count(),
                       "grammarFields": grammar.field_count()}),
            );
            if name == "typescript" {
                let tsx = GrammarDialect::Tsx.get_ts_language();
                let valid_tsx = patterns
                    .iter()
                    .filter(|pattern| tree_sitter::Query::new(&tsx, pattern).is_ok())
                    .count();
                report.insert(
                    "tsx".to_owned(),
                    json!({"attempted": patterns.len(), "valid": valid_tsx,
                           "invalid": patterns.len() - valid_tsx,
                           "grammarSymbols": tsx.node_kind_count(),
                           "grammarFields": tsx.field_count()}),
                );
            }
        }
        report.insert(
            "sql".to_owned(),
            json!({"supported": false, "reason": "ast-grep-language has no SQL grammar"}),
        );
        if let Ok(path) = std::env::var("RERAIL_QUERY_CORPUS_OUTPUT") {
            fs::write(path, serde_json::to_vec_pretty(&corpus).unwrap()).unwrap();
        }
        println!(
            "TREE_SITTER_QUERY_AUDIT={}",
            json!({"validWithoutSql": total, "languages": report})
        );
    }
