use rerail_blocks::vector::Vector;
    use super::*;

    /// Every byte after which some substring ending there matches the pattern whole: the exact set Parabix marks.
    fn brute(re: &str, text: &str) -> Vector<usize> {
        let whole = regex::Regex::new(&format!("^(?:{re})$")).unwrap();
        let b = text.as_bytes();
        let mut out = rerail_blocks::vector::Vector::new();
        for e in 1..=b.len() {
            if !text.is_char_boundary(e) {
                continue;
            }
            if (0..e).filter(|&s| text.is_char_boundary(s)).any(|s| whole.is_match(&text[s..e])) {
                out.push(e - 1);
            }
        }
        out
    }

    /// The ends of the matches the host finds (`find_iter`), as marks.
    fn found(re: &str, text: &str) -> Vector<usize> {
        regex::Regex::new(re).unwrap().find_iter(text).filter(|m| m.end() > m.start()).map(|m| m.end() - 1).collect()
    }

    fn marks(p: &SpanPrefilter, text: &str) -> Vector<usize> {
        let ends = p.ends(text.as_bytes());
        (0..text.len()).filter(|&i| ends.get(i / 64).is_some_and(|w| w >> (i % 64) & 1 == 1)).collect()
    }

    const TEXTS: &[&str] = &[
        "x = 1  # SEC1 SEC22\nclass Foo:\n    pass\nclass Bar: pass\n",
        "aaa ab abab b\nba\n\nabba",
        "Grεek Α, Bαd Р, Русский; p）q",
        "",
        "a",
    ];

    /// The marks are a superset of the ends of the matches the host finds; for a pattern without look-arounds that
    /// the translation renders exactly, they are every end of every substring that matches.
    #[test]
    fn a_patterns_block_program_marks_every_end_and_exact_ones_only_those() {
        let long = "ab ".repeat(40) + "SEC123 tail";
        let cases: &[(&str, bool)] = &[
            ("SEC\\d+", true),
            ("ab|ba", true),
            ("a+b", true),
            ("(?<w>[a-z]+)", true),
            ("x = (?<v>\\d+).*", false),
            ("(?m)^class\\s+(?<c>\\w+)", false),
            ("\\bab\\b", false),
            ("(?-u:\\b)ab", false),
            ("[\\p{Alphabetic}\\p{N}]+", false),
            ("(?<ch>[\\x{a0}´¸×])", true),
            ("(ab)*c", false),
            ("a{2,3}", true),
        ];
        for &(re, exact) in cases {
            let p = SpanPrefilter::compile(re, false).unwrap_or_else(|| panic!("{re} compiles"));
            for text in TEXTS.iter().copied().chain([long.as_str()]) {
                let (got, want) = (marks(&p, text), found(re, text));
                assert!(want.iter().all(|e| got.contains(e)), "{re} over {text:?}: misses {:?}", want.iter().filter(|e| !got.contains(e)).collect::<Vec<_>>());
                if exact {
                    assert_eq!(got, brute(re, text), "{re} over {text:?}");
                }
            }
        }
        assert!(SpanPrefilter::compile("a*", false).is_none(), "a pattern matching the empty string is the host's");
    }

    /// The program is the same on every executor of block programs: the CPU interpreter, Cranelift and, on a Mac,
    /// Metal's forward carry.
    #[test]
    fn a_patterns_block_program_runs_alike_on_every_executor() {
        for re in ["SEC\\d+", "(?m)^class\\s+(?<c>\\w+)", "[\\p{Alphabetic}\\p{N}]+"] {
            let p = SpanPrefilter::compile(re, false).unwrap();
            let program = p.program();
            let mut rows = String::new();
            for (i, t) in program.tables.iter().enumerate() {
                let hex = |x: &[u8; 16]| x.iter().map(|b| format!("{b:02x}")).collect::<String>();
                rows.push_str(&format!("  block 0 table {i} lo={} hi={}\n", hex(&t.lo), hex(&t.hi)));
            }
            for (i, op) in program.ops.iter().enumerate() {
                rows.push_str(&format!("  block 0 %{i} = {}\n", op.text()));
            }
            rows.push_str(&format!("  block 0 output %{}\n  block 0 delay 1\n", program.outputs[0]));
            let assembled = rerail_rir::rir_asm::assemble(&format!("program -\n  mode set=0 query=0\n{rows}  0000 STOP 0\n")).unwrap();
            let executors = rerail_parser_tests::executors::Executors::new(&assembled);
            for text in TEXTS {
                let runs = executors.run(text.as_bytes(), true);
                let want = p.ends(text.as_bytes());
                for (name, got) in runs {
                    assert_eq!(got[0], want, "{re} on {name} over {text:?}");
                }
            }
        }
    }

    /// The batch, one launch of the executor (`ends_launch`), gives each (file, program) the marks the program gives
    /// that file alone: on the GPU's form and the cores', and wherever its price places it.
    #[test]
    fn a_batch_of_span_programs_marks_each_file_as_it_alone_would() {
        use std::sync::Arc;
        let filters: Vec<Arc<SpanPrefilter>> = ["SEC\\d+", "(?m)^class\\s+(?<c>\\w+)", "[\\p{Alphabetic}\\p{N}]+"].iter().map(|re| Arc::new(SpanPrefilter::compile(re, false).unwrap())).collect();
        let long = "x = 1  # SEC12\nclass Foo:\n    pass\n".repeat(500);
        let sources: Vec<Arc<crate::code_rule_dsl::tape::Text>> = TEXTS.iter().copied().chain([long.as_str()]).map(|t| Arc::new(t.into())).collect();
        let files: Vec<(usize, usize)> = (0..sources.len()).flat_map(|d| (0..filters.len()).map(move |f| (d, f))).collect();
        let ex = |device| crate::code_rule_dsl::logic::lanes::executor_for(device);
        let gpu = cfg!(any(target_os = "macos", target_os = "linux", windows)) && ex(true).lanes().contains(&rerail_accel::Lane::Gpu);
        let mut runs = vec![(false, Some(rerail_accel::Lane::Cpu)), (true, None)];
        if gpu {
            // The set's kernels compile on a thread of their own (the cores run it meanwhile): the GPU's run is
            // after.
            #[cfg(target_os = "macos")]
            {
                let natives: Vec<&crate::code_rule_dsl::transducer::packed::native_ir::BlockNative> = filters.iter().map(|f| f.native()).collect();
                compiled(&natives);
            }
            runs.push((true, Some(rerail_accel::Lane::Gpu)));
        }
        for (device, lane) in runs {
            let (got, ran) = super::ends_on(&ex(device), &filters, &sources, &files, device, lane).unwrap();
            if let Some(lane) = lane {
                assert_eq!(ran == RanOn::Metal, lane == rerail_accel::Lane::Gpu, "{lane:?}: {ran:?}");
            }
            for ((d, f), got) in files.iter().zip(&got) {
                assert_eq!(*got, filters[*f].ends(sources[*d].as_bytes()), "program {f} on {ran:?}");
            }
        }
    }

    /// The forward carry's kernels of `natives` (`serial::compiled`), compiled on their own threads: waited for, bounded.
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    fn compiled(natives: &[&crate::code_rule_dsl::transducer::packed::native_ir::BlockNative]) {
        use crate::code_rule_dsl::transducer::packed::metal_program_backend::block_program::serial::compiled as ready;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90);
        while !ready(natives) {
            assert!(std::time::Instant::now() < deadline, "the set's kernels did not compile within 90 s");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    /// A new set of patterns placed on the GPU runs on the cores while its kernels compile on other threads, then on
    /// those kernels: the same marks both times, and the first launch never waits on the compiler.
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    #[test]
    fn a_new_pattern_set_runs_on_the_cores_while_its_kernels_compile() {
        use std::sync::Arc;
        let ex = crate::code_rule_dsl::logic::lanes::executor_for(true);
        if !ex.lanes().contains(&rerail_accel::Lane::Gpu) {
            return;
        }
        // Patterns no other test uses: the set's kernels are not compiled yet in this process.
        let filters: Vec<Arc<SpanPrefilter>> = ["QZV\\d{2}k", "(?m)^def\\s+qzv_\\w+"].iter().map(|re| Arc::new(SpanPrefilter::compile(re, false).unwrap())).collect();
        let natives: Vec<&crate::code_rule_dsl::transducer::packed::native_ir::BlockNative> = filters.iter().map(|f| f.native()).collect();
        let long = "QZV12k = 1\ndef qzv_a():\n    QZV9k\n".repeat(300);
        let sources: Vec<Arc<crate::code_rule_dsl::tape::Text>> = TEXTS.iter().copied().chain([long.as_str()]).map(|t| Arc::new(t.into())).collect();
        let files: Vec<(usize, usize)> = (0..sources.len()).flat_map(|d| (0..filters.len()).map(move |f| (d, f))).collect();
        let check = |what: &str, on: RanOn| {
            let (got, ran) = super::ends_on(&ex, &filters, &sources, &files, true, Some(rerail_accel::Lane::Gpu)).unwrap();
            assert_eq!(ran, on, "{what}");
            for ((d, f), got) in files.iter().zip(&got) {
                assert_eq!(*got, filters[*f].ends(sources[*d].as_bytes()), "program {f}, {what}");
            }
        };
        let asked = std::time::Instant::now();
        check("on the cores, the kernels compiling", RanOn::Cpu);
        assert!(asked.elapsed() < std::time::Duration::from_secs(20), "the first launch waited {:?}", asked.elapsed());
        compiled(&natives);
        check("on the set's own kernels", RanOn::Metal);
    }
