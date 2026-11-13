    use super::*;

    // Deliberately serial first-visit oracle. No prefix scan or interval formula
    // is shared with the operator. It records each visited state exactly once.
    fn oracle(domains: &[Domain], entries: &[Entry], allowed: &[bool]) -> Vec<Exit> {
        let mut visited = vec![false; allowed.len()];
        let mut exits = Vec::new();
        for (priority, entry) in entries.iter().enumerate() {
            let domain = domains[entry.domain as usize];
            let mut at = (domain.first + entry.offset) as usize;
            if visited[at] {
                continue;
            }
            loop {
                visited[at] = true;
                let next = at + 1;
                if next == (domain.first + domain.count) as usize || !allowed[next] || visited[next]
                {
                    break;
                }
                at = next;
            }
            exits.push(Exit {
                entry: priority as u32,
                last: at as u32,
            });
        }
        exits
    }

    #[test]
    fn bulk_closure_matches_every_short_ordered_entry_list_and_guard_partition() {
        let mut checked = 0;
        for len in 1..=5u32 {
            let domains = [Domain {
                first: 0,
                count: len,
            }];
            for mask in 0..(1 << len) {
                let allowed = (0..len).map(|i| mask & (1 << i) != 0).collect::<Vec<_>>();
                // Repetition permits duplicate starts; order is semantically significant.
                for count in 0..=3u32 {
                    for mut encoded in 0..len.pow(count) {
                        let entries = (0..count)
                            .map(|_| {
                                let offset = encoded % len;
                                encoded /= len;
                                Entry { domain: 0, offset }
                            })
                            .collect::<Vec<_>>();
                        let expected = oracle(&domains, &entries, &allowed);
                        let (actual, _) = resolve(&domains, &entries, &allowed).unwrap();
                        assert_eq!(
                            actual, expected,
                            "len={len} mask={mask} entries={entries:?}"
                        );
                        checked += 1;
                    }
                }
            }
        }
        assert_eq!(checked, 6740);
    }

    #[test]
    fn bulk_closure_scales_independent_domains_without_serial_entry_rounds() {
        let mut expected_stages = None;
        for segments in [1, 128, 4096] {
            let domains = (0..segments)
                .map(|i| Domain {
                    first: i * 8,
                    count: 8,
                })
                .collect::<Vec<_>>();
            let mut entries = Vec::new();
            for group in (0..segments).rev() {
                // Earlier entry owns 5..7; later entry must stop at 4. Duplicate
                // entry 5 is suppressed. The third surviving entry owns 0..1.
                entries.extend([5, 2, 5, 0].map(|offset| Entry {
                    domain: group,
                    offset,
                }));
            }
            let allowed = vec![true; segments as usize * 8];
            let expected = oracle(&domains, &entries, &allowed);
            let start = std::time::Instant::now();
            let (actual, report) = resolve(&domains, &entries, &allowed).unwrap();
            let elapsed = start.elapsed();
            assert_eq!(actual, expected);
            assert_eq!(actual.len(), segments as usize * 3);
            assert_eq!(
                report.dispatches.len(),
                *expected_stages.get_or_insert(report.dispatches.len())
            );
            assert_eq!(
                report
                    .dispatches
                    .iter()
                    .filter(|d| d.stage == Stage::ScanBlocks)
                    .count(),
                3
            );
            assert!(
                report
                    .dispatches
                    .iter()
                    .all(|d| d.lanes <= MAX_CELLS && matches!(d.local_rounds, 0 | 8))
            );
            assert!(report.reserved_bytes <= LIMITS.payload_bytes);
            eprintln!(
                "BULK_SEGMENTS {}",
                serde_json::json!({"segments":segments,"exits":actual.len(),"stages":report.dispatches.len(),"reservedBytes":report.reserved_bytes,"cpuSeconds":elapsed.as_secs_f64()})
            );
        }
    }

    #[test]
    fn bulk_closure_crosses_workgroup_boundaries_and_preserves_all_run_breaks() {
        for count in [255, 256, 257, 511, 1024, 32768] {
            let domains = [Domain { first: 0, count }];
            let entries = (0..count)
                .rev()
                .step_by(29)
                .map(|offset| Entry { domain: 0, offset })
                .collect::<Vec<_>>();
            let allowed = (0..count)
                .map(|i| i % 127 != 0 && i % 257 != 0)
                .collect::<Vec<_>>();
            assert_eq!(
                resolve(&domains, &entries, &allowed).unwrap().0,
                oracle(&domains, &entries, &allowed)
            );
        }
    }

    #[test]
    fn bulk_closure_rejects_inconsistent_domains_and_capacity_before_output() {
        for domains in [
            vec![Domain { first: 1, count: 2 }],
            vec![Domain { first: 0, count: 0 }],
            vec![Domain { first: 0, count: 4 }],
            vec![Domain { first: 0, count: 2 }, Domain { first: 1, count: 2 }],
            vec![Domain { first: 0, count: 2 }, Domain { first: 3, count: 1 }],
            vec![Domain {
                first: 0,
                count: u32::MAX,
            }],
        ] {
            assert!(matches!(
                resolve(&domains, &[], &[true; 3]),
                Err(Error::InvalidAnchor)
            ));
        }
        let domains = [Domain { first: 0, count: 3 }];
        for entry in [
            Entry {
                domain: 1,
                offset: 0,
            },
            Entry {
                domain: 0,
                offset: 3,
            },
        ] {
            assert!(matches!(
                resolve(&domains, &[entry], &[true; 3]),
                Err(Error::InvalidAnchor)
            ));
        }
        assert_eq!(
            reservation(MAX_CELLS + 1, 0, 0),
            Err(Error::BackendCapacity)
        );
        assert_eq!(
            reservation(0, MAX_CELLS + 1, 0),
            Err(Error::BackendCapacity)
        );
        assert_eq!(resolve(&[], &[], &[]).unwrap().0, vec![]);
    }
    fn linear_program(body: &[u32]) -> Program {
        let exit = (7 + body.len()) as u16;
        let mut words = vec![
            word(CAPTURE, 0, 0),
            word(REPEAT_BEGIN, 0, 0),
            word(REPEAT_BUMP, 0, 0),
            word(FORK, GREEDY, exit),
            word(NEXT, 0, 0),
        ];
        words.extend_from_slice(body);
        words.extend([
            word(REPEAT_BUMP, 0, 0),
            word(GOTO, 0, 3),
            word(REPEAT_END, 0, 0),
            word(EMIT, 0, 0),
        ]);
        Program::new(
            words,
            vec![b"a".to_vec(), b"b".to_vec()],
            vec![],
            vec![0, 1],
            vec![],
        )
        .unwrap()
        .with_tree_sitter_query_semantics()
        .unwrap()
    }

    #[test]
    fn bulk_closure_keeps_complete_frame_capture_values_and_exit_positions() {
        let program = linear_program(&[]);
        let tape = document("x\n# a\n# bb\n# ccc\n");
        let siblings = tape.transducer_children(0).collect::<Vec<_>>();
        assert_eq!(siblings.len(), 4);
        let expression = siblings[0];
        let identifier = tape.transducer_children(expression).next().unwrap();
        assert_ne!(expression, identifier);
        assert_eq!(
            tape.transducer_span(expression),
            tape.transducer_span(identifier)
        );
        let base = State {
            pc: 3,
            node: siblings[0],
            cursors: vec![Some(0)],
            repeats: vec![0],
            captures: vec![(0, expression, 0, 1), (1, expression, 0, 1)],
        };
        let mut frontier = (0..8)
            .map(|index| Item {
                position: vec![index],
                state: base.clone(),
                kind: Kind::Active,
            })
            .collect::<Vec<_>>();
        frontier[0].state.node = siblings[2];
        frontier[2].state.node = siblings[2]; // exact duplicate of the first entry
        frontier[3].state.cursors[0] = Some(expression);
        frontier[4].state.repeats.insert(0, 1);
        frontier[5].state.captures.reverse();
        frontier[6].state.captures.push((1, expression, 0, 1));
        frontier[7].state.captures[0].1 = identifier; // equal geometry, different identity
        let (actual, _, reserved) = close(&program, &tape, &frontier, 3).unwrap().unwrap();
        assert_eq!(actual.len(), 7);
        assert!(reserved <= LIMITS.payload_bytes);
        for (actual, index) in actual.iter().zip([0, 1, 3, 4, 5, 6, 7]) {
            let mut expected = frontier[index].state.clone();
            expected.node = if index == 1 { siblings[1] } else { siblings[3] };
            expected.pc = 7;
            assert_eq!(actual.state, expected, "entry={index}");
            assert_eq!(actual.position, vec![index, 1]);
            assert!(matches!(actual.kind, Kind::Active));
        }
        let mut invalid = frontier.clone();
        invalid[0].state.node = u32::MAX;
        assert!(matches!(
            close(&program, &tape, &invalid, 3),
            Err(Error::InvalidAnchor)
        ));
        invalid = frontier.clone();
        invalid[0].position = invalid[1].position.clone();
        assert!(matches!(
            close(&program, &tape, &invalid, 3),
            Err(Error::InvalidAnchor)
        ));
        invalid = frontier.clone();
        invalid[0].state.repeats.clear();
        assert!(matches!(
            close(&program, &tape, &invalid, 3),
            Err(Error::InvalidOperand(3))
        ));
        let mut pending = frontier;
        pending.push(Item {
            position: vec![9],
            state: State { pc: 7, ..base },
            kind: Kind::Greedy {
                body: vec![9, 0],
                last_offer: vec![9],
            },
        });
        assert!(matches!(
            close(&program, &tape, &pending, 3),
            Err(Error::BackendFailure)
        ));
    }

    #[test]
    fn bulk_closure_preserves_cpu_results_when_earlier_entries_start_later() {
        for body_guard in [word(FLAGS, 0, 0), word(SIZE, 2, 4)] {
            for exit_size in 2..=5 {
                let program = Program::new(
                    vec![
                        word(CAPTURE, 0, 0),
                        word(REPEAT_BEGIN, 0, 0),
                        word(FORK, 0, 6),
                        word(NEXT, 0, 0),
                        word(NEXT, 0, 0),
                        word(GOTO, 0, 6),
                        word(REPEAT_BUMP, 0, 0),
                        word(FORK, GREEDY, 12),
                        word(NEXT, 0, 0),
                        body_guard,
                        word(REPEAT_BUMP, 0, 0),
                        word(GOTO, 0, 7),
                        word(REPEAT_END, 0, 0),
                        word(SIZE, 0, exit_size),
                        word(EMIT, 0, 0),
                    ],
                    vec![b"prefix".to_vec()],
                    vec![],
                    vec![0],
                    vec![],
                )
                .unwrap()
                .with_tree_sitter_query_semantics()
                .unwrap();
                let tape = document("#a\n#aa\n#aaa\n#aaaa\n");
                let expected = program.execute_roots(&tape, [1]).unwrap();
                let selected_sizes = if split(body_guard).0 == SIZE {
                    [3, 4]
                } else {
                    [3, 5]
                };
                assert_eq!(
                    expected.len(),
                    usize::from(selected_sizes.contains(&exit_size))
                );
                let mut measured = ScheduleObservation::default();
                let actual = execute_bulk(&program, &tape, 1, &mut measured).unwrap();
                assert_eq!(actual, expected, "body={body_guard} exit={exit_size}");
                assert_eq!(measured.closures, 1);
                assert!(measured.waves < 64);
            }
        }
    }

    #[test]
    fn bulk_closure_proof_selects_only_bounded_scalar_bodies_without_changing_acceptance() {
        let tape = document("#a\n#aa\n#aaa\n#aaaa\n");
        for body in [
            vec![],
            vec![word(SIZE, 2, 4)],
            vec![word(FLAGS, 0, 0); 64],
            vec![word(SAVE_CURSOR, 0, 0)],
            vec![word(FLAGS, 0, 0); 65],
        ] {
            let program = linear_program(&body);
            let supported = body.len() <= 64 && !body.iter().any(|&w| split(w).0 == SAVE_CURSOR);
            assert_eq!(prove(&program, 3).unwrap().is_some(), supported);
            let expected = program.execute_roots(&tape, [1]).unwrap();
            let mut measured = ScheduleObservation::default();
            assert_eq!(
                execute_bulk(&program, &tape, 1, &mut measured).unwrap(),
                expected
            );
            assert_eq!(measured.closures, usize::from(supported));
        }
        assert!(prove(&greedy_diamond(), 3).unwrap().is_none());
        let mut ordinary = linear_program(&[]);
        ordinary.tree_sitter_query = false;
        assert!(close(&ordinary, &tape, &[], 3).unwrap().is_none());
        assert_eq!(
            execute(&ordinary, &tape, 1),
            ordinary.execute_roots(&tape, [1])
        );
    }

    #[test]
    fn bulk_closure_refuses_oversized_input_before_partial_publication() {
        let program = linear_program(&[]);
        let tape = document(&"# a\n".repeat(MAX_CELLS + 1));
        let frontier = [Item {
            position: vec![],
            state: State {
                pc: 3,
                node: 1,
                cursors: vec![],
                repeats: vec![0],
                captures: vec![],
            },
            kind: Kind::Active,
        }];
        assert!(matches!(
            close(&program, &tape, &frontier, 3),
            Err(Error::BackendCapacity)
        ));
    }
    #[test]
    fn production_closure_proof_preserves_observable_guarded_exits_and_query_acceptance() {
        let tape = document("#a\n#aa\n#aaa\n#aaaa\n");
        for body in [
            vec![],
            vec![word(SIZE, 2, 4)],
            vec![word(FLAGS, 0, 0); 64],
            vec![word(FLAGS, 0, 0); 65],
            vec![word(SAVE_CURSOR, 0, 0)],
        ] {
            let program = linear_program(&body);
            let regions = closure_proof::regions(&program).unwrap();
            let eligible = body.len() <= 64 && body.iter().all(|&w| split(w).0 != SAVE_CURSOR);
            assert_eq!(regions.len(), usize::from(eligible));
            for size in 2..=5 {
                let mut guarded = program.clone();
                let last = guarded.words.len() - 1;
                guarded.words.insert(last, word(SIZE, 0, size));
                guarded.validate().unwrap();
                let expected = guarded.execute_roots(&tape, [1]).unwrap();
                // SAVE_CURSOR changes complete state identity on each advance.
                // Its four exit positions remain observable through general execution.
                let exit_count = usize::from(match body.first().copied().map(split) {
                    Some((SAVE_CURSOR, _, _)) => true,
                    Some((SIZE, _, _)) => size == 4,
                    _ => size == 5,
                });
                assert_eq!(expected.len(), exit_count, "body={body:?} size={size}");
                let mut measured = ScheduleObservation::default();
                assert_eq!(
                    execute_bulk(&guarded, &tape, 1, &mut measured).unwrap(),
                    expected
                );
                assert_eq!(measured.closures, usize::from(eligible));
            }
        }
        assert!(
            closure_proof::regions(&greedy_diamond())
                .unwrap()
                .is_empty()
        );
        let mut observable_body = linear_program(&[word(FLAGS, 0, 0)]);
        observable_body.words[5] = word(CAPTURE, 0, 0);
        assert_eq!(
            observable_body.validate(),
            Err(Error::InvalidGreedyRegion(3))
        );
        assert_eq!(
            closure_proof::regions(&observable_body),
            Err(Error::InvalidGreedyRegion(3))
        );
        let mut ordinary = linear_program(&[]);
        ordinary.tree_sitter_query = false;
        assert!(closure_proof::regions(&ordinary).unwrap().is_empty());
        assert_eq!(
            execute(&ordinary, &tape, 1),
            ordinary.execute_roots(&tape, [1])
        );
        let mut invalid = ordinary;
        invalid.words = vec![word(FORK, GREEDY, u16::MAX)];
        assert_eq!(
            closure_proof::regions(&invalid),
            invalid.validate().map(|()| vec![])
        );
        assert_eq!(
            close(&invalid, &tape, &[], 0).err(),
            invalid.validate().err()
        );
    }

    #[test]
    fn bulk_closure_scalar_guards_retain_observable_cpu_endpoints() {
        let tape = document("#a\nx\n#aa\n");
        let comment = tape.transducer_kind(1);
        let guards = [
            (word(KIND, 0, comment), vec![]),
            (word(FIELD, 0, 0), vec![]),
            (word(FIELD, 0, 1), vec![]),
            (word(FLAGS, NAMED, 0), vec![]),
            (word(FLAGS, 0, u16::from(NAMED)), vec![]),
            (word(SIZE, 0, 3), vec![]),
            (word(SIZE, 1, 3), vec![]),
            (word(SIZE, 2, 3), vec![]),
            (word(SIZE, SIZE_LONG, 0), vec![3]),
            (word(SIZE, SIZE_LONG | 1, 0), vec![u64::MAX]),
            (word(SIZE, SIZE_LONG | 2, 0), vec![u64::MAX]),
        ];
        let mut comparisons = 0;
        for (guard, longs) in guards {
            let mut positives = 0;
            for endpoint_size in 1..=4 {
                let mut program = linear_program(&[word(FLAGS, 0, 0)]);
                program.words[5] = guard;
                program.longs = longs.clone();
                program.words.insert(9, word(SIZE, 0, endpoint_size));
                program.validate().unwrap();
                let expected = program.execute_roots(&tape, [1]).unwrap();
                positives += expected.len();
                let mut measured = ScheduleObservation::default();
                assert_eq!(
                    execute_bulk(&program, &tape, 1, &mut measured).unwrap(),
                    expected,
                    "guard={guard} endpoint={endpoint_size}"
                );
                assert_eq!(measured.closures, 1);
                comparisons += 1;
            }
            assert_eq!(
                positives, 1,
                "exactly one endpoint must be observable for guard={guard}"
            );
        }
        assert_eq!(comparisons, 44);
    }
