//! Test-only ordered active-frontier model. It is not a production executor.
use super::*;
use ast_grep_language::LanguageExt;

#[path = "admission_bulk_closure.rs"]
mod bulk_closure;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct State {
    pc: usize,
    node: u32,
    cursors: Vec<Option<u32>>,
    repeats: Vec<u16>,
    captures: Vec<(u16, u32, usize, usize)>,
}

#[derive(Clone)]
enum Kind {
    Active,
    Terminal,
    // The marker encloses only the still-pending continuation subtree. An offer
    // from a later external branch cannot update an already separate exit.
    Greedy {
        body: Vec<usize>,
        last_offer: Vec<usize>,
    },
}

#[derive(Clone)]
struct Item {
    position: Vec<usize>,
    state: State,
    kind: Kind,
}

// Count both pending proposals and successors before reduction. These are
// bounded research payloads; process RSS and production Metal bytes are separate.
fn retain(items: &mut Vec<Item>, bytes: &mut usize, item: Item) -> Result<(), Error> {
    let payload = item_payload(&item);
    *bytes = bytes.checked_add(payload).ok_or(Error::BackendCapacity)?;
    if *bytes > LIMITS.payload_bytes || items.len() >= LIMITS.candidates {
        return Err(Error::BackendCapacity);
    }
    items.push(item);
    Ok(())
}

fn item_payload(item: &Item) -> usize {
    let pending_positions = match &item.kind {
        Kind::Greedy { body, last_offer } => body.capacity() + last_offer.capacity(),
        _ => 0,
    };
    std::mem::size_of::<Item>()
        + pending_positions * std::mem::size_of::<usize>()
        + item.position.capacity() * std::mem::size_of::<usize>()
        + item.state.cursors.capacity() * std::mem::size_of::<Option<u32>>()
        + item.state.repeats.capacity() * std::mem::size_of::<u16>()
        + item.state.captures.capacity() * std::mem::size_of::<(u16, u32, usize, usize)>()
}

fn same_exit(a: &State, b: &State) -> bool {
    a.pc == b.pc && a.cursors == b.cursors && a.repeats == b.repeats && a.captures == b.captures
}

fn execute(program: &Program, tape: &TapeDoc, root: u32) -> Result<Vec<Output>, Error> {
    let expected = execute_observed(program, tape, root, &mut ScheduleObservation::default());
    let actual = execute_bulk(program, tape, root, &mut ScheduleObservation::default());
    assert_eq!(
        actual, expected,
        "bulk closure changed the ordered reference result"
    );
    actual
}

fn execute_bulk(
    program: &Program,
    tape: &TapeDoc,
    root: u32,
    observation: &mut ScheduleObservation,
) -> Result<Vec<Output>, Error> {
    execute_mode(program, tape, root, observation, true)
}

#[derive(Default)]
struct ScheduleObservation {
    waves: usize,
    header_rounds: usize,
    steps: usize,
    closures: usize,
    closure_stages: usize,
    closure_reserved_bytes: usize,
}

#[test]
fn bulk_closure_preserves_terminal_results_without_one_round_per_sibling() {
    let program = Program::new(
        vec![
            word(CAPTURE, 0, 0),
            word(REPEAT_BEGIN, 0, 0),
            word(REPEAT_BUMP, 0, 0),
            word(FORK, GREEDY, 7),
            word(NEXT, 0, 0),
            word(REPEAT_BUMP, 0, 0),
            word(GOTO, 0, 3),
            word(REPEAT_END, 0, 0),
            word(LAST, NAMED, 0),
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
    let mut rounds = Vec::new();
    for siblings in [1, 128, 1024] {
        let tape = document(&"# item\n".repeat(siblings));
        let expected = program.execute_roots(&tape, [1]).unwrap();
        assert_eq!(expected.len(), 1);
        assert_eq!(expected[0].captures.len(), 1);
        assert_eq!(expected[0].captures[0].span, 0..6);
        let mut observation = ScheduleObservation::default();
        let start = std::time::Instant::now();
        let actual = execute_bulk(&program, &tape, 1, &mut observation).unwrap();
        let bulk_seconds = start.elapsed().as_secs_f64();
        assert_eq!(actual, expected, "siblings={siblings}");
        let mut baseline = ScheduleObservation::default();
        let before = std::time::Instant::now();
        assert_eq!(
            execute_observed(&program, &tape, 1, &mut baseline).unwrap(),
            expected
        );
        eprintln!(
            "BULK_CLOSURE_EXECUTION {}",
            serde_json::json!({
                "siblings":siblings, "beforeWaves":baseline.waves, "afterStages":observation.waves,
                "beforeSeconds":before.elapsed().as_secs_f64(), "afterSeconds":bulk_seconds,
                "closures":observation.closures, "closureStages":observation.closure_stages,
                "reservedBytes":observation.closure_reserved_bytes
            })
        );
        assert_eq!(observation.closures, 1);
        assert_eq!(observation.header_rounds, 0);
        rounds.push(observation.waves);
    }
    assert!(
        rounds.iter().all(|&rounds| rounds <= 64),
        "eligible closure must finish within 64 broad CPU stages, not one wave per sibling: {rounds:?}"
    );
}

fn execute_observed(
    program: &Program,
    tape: &TapeDoc,
    root: u32,
    observation: &mut ScheduleObservation,
) -> Result<Vec<Output>, Error> {
    execute_mode(program, tape, root, observation, false)
}

fn execute_mode(
    program: &Program,
    tape: &TapeDoc,
    root: u32,
    observation: &mut ScheduleObservation,
    bulk: bool,
) -> Result<Vec<Output>, Error> {
    program.validate()?;
    let query_admission =
        program.tree_sitter_query && program.words.iter().any(|&w| split(w).0 == REPEAT_BEGIN);
    let headers = if query_admission {
        super::super::greedy_contract::ordered_headers(&program.words)?
            .into_iter()
            .collect::<BTreeMap<_, _>>()
    } else {
        BTreeMap::new()
    };
    let mut frontier = vec![Item {
        position: vec![],
        state: State {
            pc: 0,
            node: root,
            cursors: vec![],
            repeats: vec![],
            captures: vec![],
        },
        kind: Kind::Active,
    }];
    let mut steps = 0;
    let mut waves = 0;
    while frontier.iter().any(|i| !matches!(i.kind, Kind::Terminal)) {
        // Finish all intervening structural work before admission of headers.
        // Region order prevents a later region from retiring before an earlier
        // region can produce its inputs. Within a region, every return advances
        // the node, so all predecessors arrive before the lowest node expands.
        let outside = frontier
            .iter()
            .any(|item| matches!(item.kind, Kind::Active) && !headers.contains_key(&item.state.pc));
        let admitted = (!outside)
            .then(|| {
                frontier
                    .iter()
                    .filter_map(|item| {
                        if !matches!(item.kind, Kind::Active) {
                            return None;
                        }
                        headers
                            .get(&item.state.pc)
                            .map(|&rank| (rank, item.state.node))
                    })
                    .min()
            })
            .flatten();
        if bulk && let Some((rank, _)) = admitted {
            let header = headers
                .iter()
                .find_map(|(&pc, &order)| (order == rank).then_some(pc))
                .unwrap();
            if let Some((closed, stages, reserved)) =
                bulk_closure::close(program, tape, &frontier, header)?
            {
                frontier = closed;
                observation.closures += 1;
                observation.closure_stages += stages;
                observation.closure_reserved_bytes =
                    observation.closure_reserved_bytes.max(reserved);
                continue;
            }
        }
        observation.header_rounds += usize::from(admitted.is_some());
        let mut next = Vec::new();
        let mut offers = Vec::new();
        let mut retained = 0;
        for mut item in frontier {
            if !matches!(item.kind, Kind::Active) {
                retain(&mut next, &mut retained, item)?;
                continue;
            }
            if let Some(&rank) = headers.get(&item.state.pc)
                && admitted != Some((rank, item.state.node))
            {
                retain(&mut next, &mut retained, item)?;
                continue;
            }
            steps += 1;
            // Research bounds do not change production limits or count as GPU evidence.
            if steps > LIMITS.steps {
                return Err(Error::WorkLimit);
            }
            let pc = item.state.pc;
            let (op, mode, arg) = split(*program.words.get(pc).ok_or(Error::InvalidTarget(pc))?);
            let node = item.state.node;
            item.state.pc += 1;
            let mut live = true;
            match op {
                STOP => live = false,
                KIND => live = tape.transducer_kind(node) == arg,
                FIELD => live = tape.transducer_field(node) == arg,
                FLAGS => {
                    live = tape.transducer_flags(node) & mode == mode
                        && tape.transducer_flags(node) & arg as u8 == 0
                }
                SIZE => {
                    let size = tape.transducer_span(node).len() as u64;
                    let expected = if mode & SIZE_LONG == 0 {
                        u64::from(arg)
                    } else {
                        program.longs[usize::from(arg)]
                    };
                    live = match mode & !SIZE_LONG {
                        0 => size == expected,
                        1 => size >= expected,
                        2 => size <= expected,
                        _ => unreachable!(),
                    };
                }
                FIRST | LAST => {
                    let siblings = tape
                        .transducer_parent(node)
                        .map(|p| {
                            tape.transducer_children(p)
                                .filter(|&n| mode != NAMED || tape.transducer_flags(n) & NAMED != 0)
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_else(|| vec![node]);
                    live = if op == FIRST {
                        siblings.first() == Some(&node)
                    } else {
                        siblings.last() == Some(&node)
                    };
                }
                ABSENT_FIELD => {
                    live = !tape
                        .transducer_children(node)
                        .any(|n| tape.transducer_field(n) == arg)
                }
                SUPERTYPE => live = tape.transducer_has_supertype(node, arg),
                NEXT | PARENT => match if op == NEXT {
                    tape.transducer_next_sibling(node, mode == NAMED)
                } else {
                    tape.transducer_parent(node)
                } {
                    Some(n) => item.state.node = n,
                    None => live = false,
                },
                CHILD | DESCEND | FOLLOWING => {
                    let nodes = match op {
                        CHILD => tape.transducer_children(node).collect::<Vec<_>>(),
                        DESCEND => tape.transducer_descendants(node).collect(),
                        FOLLOWING => std::iter::successors(
                            tape.transducer_next_sibling(node, mode == NAMED),
                            |&n| tape.transducer_next_sibling(n, mode == NAMED),
                        )
                        .collect(),
                        _ => unreachable!(),
                    };
                    for (choice, n) in nodes
                        .into_iter()
                        .filter(|&n| mode != NAMED || tape.transducer_flags(n) & NAMED != 0)
                        .enumerate()
                    {
                        let mut branch = item.clone();
                        branch.position.push(choice);
                        branch.state.node = n;
                        retain(&mut next, &mut retained, branch)?;
                    }
                    live = false;
                }
                SAVE_CURSOR => {
                    item.state.cursors.resize(usize::from(arg) + 1, None);
                    item.state.cursors[usize::from(arg)] = Some(node);
                }
                RESTORE_CURSOR => {
                    item.state.node = item
                        .state
                        .cursors
                        .get(usize::from(arg))
                        .copied()
                        .flatten()
                        .ok_or(Error::InvalidOperand(pc))?
                }
                REPEAT_BEGIN => item.state.repeats.push(arg),
                REPEAT_BUMP => {
                    if item.state.repeats.last() != Some(&arg) {
                        return Err(Error::InvalidOperand(pc));
                    }
                }
                REPEAT_END => {
                    if item.state.repeats.pop() != Some(arg) {
                        return Err(Error::InvalidOperand(pc));
                    }
                }
                CAPTURE => {
                    let span = tape.transducer_span(node);
                    let value = (arg, node, span.start, span.end);
                    let at = item.state.captures.partition_point(|c| {
                        (c.2, std::cmp::Reverse(c.3)) <= (value.2, std::cmp::Reverse(value.3))
                    });
                    item.state.captures.insert(at, value);
                }
                FORK => {
                    let mut alternative = item.clone();
                    alternative.state.pc = usize::from(arg);
                    alternative.position.push(1);
                    if mode == GREEDY {
                        let mut body = item.position.clone();
                        body.push(0);
                        alternative.kind = Kind::Greedy {
                            body,
                            last_offer: item.position.clone(),
                        };
                        retain(&mut offers, &mut retained, alternative)?;
                    } else {
                        retain(&mut next, &mut retained, alternative)?;
                    }
                    item.position.push(0);
                }
                GOTO => item.state.pc = usize::from(arg),
                EMIT => {
                    item.state.pc = pc;
                    item.kind = Kind::Terminal;
                }
                ASSERT if program.tree_sitter_query => {}
                _ => {
                    return Err(Error::UnsupportedFeature(
                        "ordered model requires structural instructions",
                    ));
                }
            }
            if live {
                retain(&mut next, &mut retained, item)?;
            }
        }
        offers.sort_by(|a, b| a.position.cmp(&b.position));
        for offer in offers {
            let ancestor = next
                .iter()
                .enumerate()
                .filter_map(|(index, item)| {
                    let Kind::Greedy { body, .. } = &item.kind else {
                        return None;
                    };
                    (offer.position.starts_with(body) && same_exit(&item.state, &offer.state))
                        .then_some((body.len(), index))
                })
                .min();
            if let Some((_, index)) = ancestor {
                let Kind::Greedy { last_offer, .. } = &mut next[index].kind else {
                    unreachable!()
                };
                let Kind::Greedy {
                    last_offer: position,
                    ..
                } = offer.kind
                else {
                    unreachable!()
                };
                if position > *last_offer {
                    *last_offer = position;
                    next[index].state = offer.state;
                }
            } else {
                next.push(offer);
            }
        }
        next.sort_by(|a, b| a.position.cmp(&b.position));
        if query_admission {
            let mut active = HashSet::new();
            let mut terminal = HashSet::new();
            next.retain(|item| match item.kind {
                Kind::Active => active.insert(item.state.clone()),
                Kind::Terminal => terminal.insert(item.state.clone()),
                Kind::Greedy { .. } => true,
            });
        }
        let ready = next
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                let Kind::Greedy { body, .. } = &item.kind else {
                    return None;
                };
                (!next.iter().any(|other| {
                    !matches!(other.kind, Kind::Terminal) && other.position.starts_with(body)
                }))
                .then_some(index)
            })
            .collect::<Vec<_>>();
        for index in ready {
            next[index].kind = Kind::Active;
        }
        if next.len() > LIMITS.candidates {
            return Err(Error::BackendCapacity);
        }
        frontier = next;
        waves += 1;
    }
    let output = frontier
        .into_iter()
        .enumerate()
        .map(|(ordinal, item)| {
            let (_, _, label) = split(program.words[item.state.pc]);
            let mut captures = item
                .state
                .captures
                .into_iter()
                .map(|(label, node, start, end)| Capture {
                    label,
                    node,
                    span: start..end,
                    text: None,
                    properties: vec![],
                })
                .collect::<Vec<_>>();
            captures.sort_by_key(|c| {
                (
                    c.span.start,
                    std::cmp::Reverse(c.span.end),
                    if program.tree_sitter_query {
                        0
                    } else {
                        c.label
                    },
                )
            });
            Output {
                task_ordinal: 0,
                local_ordinal: ordinal as u32,
                start: root,
                label,
                captures,
                properties: program.properties(),
                dynamic_properties: vec![],
            }
        })
        .collect();
    // Terminal filtering is independently covered. The model replaces scheduling,
    // admission and candidate numbering, not predicates or output transforms.
    let result = program.finalize_outputs(tape, output, query_admission);
    assert!(waves <= steps || steps == 0);
    observation.waves = waves + observation.closure_stages;
    observation.steps = steps;
    result
}

#[test]
#[ignore = "Research schedule measurements; CPU model rounds are not Metal dispatches"]
fn measure_ordered_greedy_schedule_depth() {
    // A validated capture-free terminal loop, with an observable prefix capture.
    // This is a hand-built contract fixture, not a pinned-catalog benchmark.
    let program = Program::new(
        vec![
            word(CAPTURE, 0, 0),
            word(REPEAT_BEGIN, 0, 0),
            word(REPEAT_BUMP, 0, 0),
            word(FORK, GREEDY, 7),
            word(NEXT, 0, 0),
            word(REPEAT_BUMP, 0, 0),
            word(GOTO, 0, 3),
            word(REPEAT_END, 0, 0),
            word(LAST, NAMED, 0),
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
    for siblings in [1, 128, 1024] {
        let tape = document(&"# item\n".repeat(siblings));
        let expected = program.execute_roots(&tape, [1]).unwrap();
        assert_eq!(expected.len(), 1);
        assert_eq!(expected[0].captures.len(), 1);
        assert_eq!(expected[0].captures[0].span, 0..6);
        let mut observation = ScheduleObservation::default();
        let start = std::time::Instant::now();
        let actual = execute_observed(&program, &tape, 1, &mut observation).unwrap();
        let elapsed = start.elapsed();
        assert_eq!(actual, expected, "siblings={siblings}");
        eprintln!(
            "ORDERED_SCHEDULE {}",
            serde_json::json!({
                "siblings": siblings,
                "waves": observation.waves,
                "headerRounds": observation.header_rounds,
                "steps": observation.steps,
                "modelSeconds": elapsed.as_secs_f64(),
                "outputs": actual.len(),
            })
        );
    }
}

#[test]
fn ordered_active_model_preserves_full_outputs_for_accepted_greedy_queries() {
    let queries = [
        "(module (expression_statement) @prefix (comment)+)",
        "(module (expression_statement)? @prefix (comment)*)",
        "(module (comment)+)",
        "(module [(comment) (expression_statement)]+)",
        "(module (function_definition body: (block (expression_statement)+))*)",
        "(module (comment)+ @items)",
        "(module (expression_statement) @prefix . (comment)+ .)",
        "(module (function_definition !return_type body: (block (expression_statement)+))+)",
        "(module (expression_statement) @prefix . (comment) @tail)",
        "(module . (comment) @first)",
        "(module (comment)+ @tail .)",
        "(module (comment)+ (comment) @middle (comment)+)",
    ];
    for query in queries {
        let program = Program::compile_tree_sitter_query_pattern(
            query,
            &SupportLang::Python.get_ts_language(),
        )
        .unwrap();
        let mut sources = [0, 1, 2, 16]
            .map(|count| format!("x\n{}", "# item\n".repeat(count)))
            .to_vec();
        sources.extend([
            "def f():\n    x\n    y\n\ndef g():\n    z\n".to_owned(),
            "# first\nx\n# last\ny\n".to_owned(),
        ]);
        for source in sources {
            let tape = document(&source);
            for root in 0..tape.transducer_len() as u32 {
                assert_eq!(
                    execute(&program, &tape, root),
                    program.execute_roots(&tape, [root]),
                    "query={query} root={root} source={source:?}"
                );
            }
        }
    }
}

#[test]
fn ordered_active_model_retains_dfs_ordinals_and_ordinary_occurrences() {
    let tape = document("# first\n# second\n");
    for query in [false, true] {
        let mut program = Program::new(
            vec![
                word(REPEAT_BEGIN, 0, 0),
                word(FORK, 0, 6),
                word(SAVE_CURSOR, 0, 0),
                word(NEXT, NAMED, 0),
                word(RESTORE_CURSOR, 0, 0),
                word(GOTO, 0, 7),
                word(GOTO, 0, 7),
                word(CAPTURE, 0, 0),
                word(EMIT, 0, 0),
            ],
            vec![b"item".to_vec()],
            vec![],
            vec![0],
            vec![],
        )
        .unwrap();
        program.tree_sitter_query = query;
        program.validate().unwrap();
        assert_eq!(
            execute(&program, &tape, 1),
            program.execute_roots(&tape, [1])
        );
    }
    let program = Program::new(
        vec![word(FORK, GREEDY, 2), word(EMIT, 0, 0), word(EMIT, 0, 0)],
        vec![b"item".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap();
    let expected = program.execute_roots(&tape, [1]).unwrap();
    assert_eq!(
        expected.iter().map(|o| o.local_ordinal).collect::<Vec<_>>(),
        [0, 1]
    );
    assert_eq!(execute(&program, &tape, 1), Ok(expected));
}

#[test]
fn ordered_active_model_bounds_ambiguous_capture_growth_without_partial_success() {
    let query = "(module [(comment) @x @y (comment) @x]+ @z)";
    let program =
        Program::compile_tree_sitter_query_pattern(query, &SupportLang::Python.get_ts_language())
            .unwrap();
    for count in [0, 1, 2, 3, 6] {
        let tape = document(&format!("x\n{}", "# item\n".repeat(count)));
        assert_eq!(
            execute(&program, &tape, 0),
            program.execute_roots(&tape, [0]),
            "count={count}"
        );
    }
    let tape = document(&format!("x\n{}", "# item\n".repeat(16)));
    let actual = execute(&program, &tape, 0);
    assert!(
        matches!(actual, Err(Error::WorkLimit | Error::BackendCapacity)),
        "bounded model must refuse, without partial output: {actual:?}"
    );
}

#[test]
fn ordered_active_model_preserves_the_production_convergence_ordinal_gap() {
    let mut program = Program::new(
        vec![
            word(SAVE_CURSOR, 0, 0),
            word(REPEAT_BEGIN, 0, 0),
            word(FORK, 0, 12),
            word(FORK, 0, 8),
            word(NEXT, NAMED, 0),
            word(RESTORE_CURSOR, 0, 0),
            word(GOTO, 0, 11),
            word(STOP, 0, 0),
            word(FOLLOWING, NAMED, 0),
            word(RESTORE_CURSOR, 0, 0),
            word(GOTO, 0, 11),
            word(EMIT, 0, 0),
            word(CAPTURE, 0, 1),
            word(EMIT, 0, 1),
        ],
        vec![b"empty".to_vec(), b"captured".to_vec()],
        vec![],
        vec![0, 1],
        vec![],
    )
    .unwrap();
    program.tree_sitter_query = true;
    program.validate().unwrap();
    for siblings in [2, 130] {
        let tape = document(&"# node\n".repeat(siblings));
        let expected = vec![Output {
            task_ordinal: 0,
            local_ordinal: 1,
            start: 1,
            label: 1,
            captures: vec![Capture {
                label: 1,
                node: 1,
                span: 0..6,
                text: None,
                properties: vec![],
            }],
            properties: vec![],
            dynamic_properties: vec![],
        }];
        assert_eq!(program.execute_roots(&tape, [1]), Ok(expected.clone()));
        assert_eq!(
            execute(&program, &tape, 1),
            Ok(expected),
            "siblings={siblings}"
        );
    }
}

fn greedy_diamond() -> Program {
    // Header edges: 1 -> 2 or 3, 2 -> 4, 3 -> 4. Historical admission
    // visits headers 1,2,4,3. Unfolded paths visit 1,2,4,3,4 instead.
    // All cycles advance. The body and exit are capture-free. The exit's
    // LAST test makes the selected node observable through the prefix capture.
    let mut program = Program::new(
        vec![
            word(CAPTURE, 0, 0),
            word(REPEAT_BEGIN, 0, 0),
            word(REPEAT_BUMP, 0, 0),
            word(FORK, GREEDY, 20),
            word(NEXT, 0, 0),
            word(FORK, 0, 16),
            word(FORK, 0, 10),
            word(SIZE, 0, 3),
            word(GOTO, 0, 18),
            word(STOP, 0, 0),
            word(FORK, 0, 14),
            word(SIZE, 0, 4),
            word(NEXT, 0, 0),
            word(GOTO, 0, 18),
            word(SIZE, 0, 5),
            word(GOTO, 0, 18),
            word(SIZE, 0, 3),
            word(NEXT, 0, 0),
            word(REPEAT_BUMP, 0, 0),
            word(GOTO, 0, 3),
            word(REPEAT_END, 0, 0),
            word(LAST, NAMED, 0),
            word(EMIT, 0, 0),
        ],
        vec![b"prefix".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap();
    program.tree_sitter_query = true;
    program
}

#[test]
fn ordered_active_model_preserves_accepted_greedy_reconvergence() {
    let program = greedy_diamond();
    assert_eq!(
        program.validate(),
        Ok(()),
        "this program satisfies the approved construction boundary"
    );
    let tape = document("#a\n#aa\n#aaa\n#aaaa\n");
    assert_eq!(
        (1..=4)
            .map(|n| tape.transducer_span(n).len())
            .collect::<Vec<_>>(),
        [2, 3, 4, 5]
    );
    assert_eq!(program.execute_roots(&tape, [1]), Ok(vec![]));
    assert_eq!(
        execute(&program, &tape, 1),
        Ok(vec![]),
        "pending GREEDY selection must preserve the admitted DFS order"
    );
}

#[test]
fn greedy_contract_rejects_reentry_through_the_preceding_repeat_bump() {
    let mut program = greedy_diamond();
    program.words.truncate(21);
    program.words.extend([
        word(FORK, 0, 24),
        word(LAST, NAMED, 0),
        word(EMIT, 0, 0),
        word(NEXT, 0, 0),
        word(REPEAT_BEGIN, 0, 0),
        word(GOTO, 0, 2),
    ]);
    // The alternative advances and re-enters through the required preceding
    // BUMP. Testing only the header's direct predecessors misses this path.
    program.tree_sitter_query = false;
    assert_eq!(program.validate(), Ok(()));
    let tape = document("#a\n#aa\n#aaa\n#aaaa\n");
    assert_eq!(
        program.execute_roots(&tape, [1]),
        Ok(vec![Output {
            task_ordinal: 0,
            local_ordinal: 0,
            start: 1,
            label: 0,
            captures: vec![Capture {
                label: 0,
                node: 1,
                span: 0..2,
                text: None,
                properties: vec![]
            }],
            properties: vec![],
            dynamic_properties: vec![],
        }])
    );
    program.tree_sitter_query = true;
    assert_eq!(
        program.validate(),
        Err(Error::InvalidGreedyRegion(3)),
        "a terminal GREEDY exit must not return to its retired header indirectly"
    );
}

#[test]
fn ordered_active_model_waits_for_upstream_regions_before_admitting_shared_headers() {
    let mut program = greedy_diamond();
    let diamond = program.words.clone();
    program.words = vec![
        word(CAPTURE, 0, 0),
        word(FORK, 0, 11),
        word(REPEAT_BEGIN, 0, 0),
        word(REPEAT_BUMP, 0, 0),
        word(FORK, GREEDY, 9),
        word(NEXT, 0, 0),
        word(SIZE, 2, 4),
        word(REPEAT_BUMP, 0, 0),
        word(GOTO, 0, 4),
        word(REPEAT_END, 0, 0),
        word(GOTO, 0, 11),
    ];
    for &instruction in &diamond[1..] {
        let (op, mode, arg) = split(instruction);
        let arg = match op {
            FORK | GOTO => arg + 10,
            REPEAT_BEGIN | REPEAT_BUMP | REPEAT_END => 1,
            _ => arg,
        };
        program.words.push(word(op, mode, arg));
    }
    // The earlier logical branch runs another region before it enters the
    // diamond at node 3. The later branch enters the diamond directly at 1.
    // CPU admission visits 3,4 first, then 1,2, so only the later exit passes.
    program.words[31] = word(SIZE, 0, 3);
    assert_eq!(program.validate(), Ok(()));
    let tape = document("#a\n#aa\n#aaa\n#aaaa\n");
    let expected = vec![Output {
        task_ordinal: 0,
        local_ordinal: 0,
        start: 1,
        label: 0,
        captures: vec![Capture {
            label: 0,
            node: 1,
            span: 0..2,
            text: None,
            properties: vec![],
        }],
        properties: vec![],
        dynamic_properties: vec![],
    }];
    assert_eq!(program.execute_roots(&tape, [1]), Ok(expected.clone()));
    assert_eq!(execute(&program, &tape, 1), Ok(expected));
}

#[test]
fn greedy_contract_preserves_a_maximum_length_terminal_repetition() {
    let mut program = greedy_diamond();
    let size = usize::from(u16::MAX) + 1;
    program.words = vec![word(FLAGS, 0, 0); size];
    program.words[..5].copy_from_slice(&[
        word(CAPTURE, 0, 0),
        word(REPEAT_BEGIN, 0, 0),
        word(REPEAT_BUMP, 0, 0),
        word(FORK, GREEDY, (size - 2) as u16),
        word(NEXT, 0, 0),
    ]);
    program.words[size - 4..].copy_from_slice(&[
        word(REPEAT_BUMP, 0, 0),
        word(GOTO, 0, 3),
        word(REPEAT_END, 0, 0),
        word(EMIT, 0, 0),
    ]);
    assert_eq!(program.validate(), Ok(()));
}

#[test]
fn ordered_active_model_matches_every_ordered_advancing_graph_on_four_nodes() {
    let tape = document("#a\n#aa\n#aaa\n#aaaa\n");
    let mut graphs = BTreeSet::new();
    for mask in 0..64u32 {
        for first in [
            [1, 2, 3],
            [1, 3, 2],
            [2, 1, 3],
            [2, 3, 1],
            [3, 1, 2],
            [3, 2, 1],
        ] {
            for second in [[2, 3], [3, 2]] {
                let mut graph = vec![vec![]; 4];
                for (from, choices) in [(0, first.to_vec()), (1, second.to_vec()), (2, vec![3])] {
                    for to in choices {
                        let bit = match (from, to) {
                            (0, to) => to - 1,
                            (1, to) => to + 1,
                            (2, 3) => 5,
                            _ => unreachable!(),
                        };
                        if mask & (1 << bit) != 0 {
                            graph[from].push(to);
                        }
                    }
                }
                graphs.insert(graph);
            }
        }
    }
    assert_eq!(
        graphs.len(),
        160,
        "all ordered subsets of 3, 2 and 1 possible forward edges"
    );
    for graph in graphs {
        // Independent graph oracle: the final first-visited vertex in ordered DFS.
        let mut seen = [false; 4];
        let mut pending = vec![0];
        let mut last = 0;
        while let Some(node) = pending.pop() {
            if std::mem::replace(&mut seen[node], true) {
                continue;
            }
            last = node;
            pending.extend(graph[node].iter().rev().copied());
        }
        let mut words = vec![
            word(CAPTURE, 0, 0),
            word(REPEAT_BEGIN, 0, 0),
            word(REPEAT_BUMP, 0, 0),
            word(FORK, GREEDY, 0),
            word(NEXT, 0, 0),
        ];
        let mut returns = Vec::new();
        for (from, successors) in graph.iter().enumerate() {
            for &to in successors {
                let fork = words.len();
                words.push(word(FORK, 0, 0));
                words.push(word(SIZE, 0, (from + 3) as u16));
                words.extend(std::iter::repeat_n(word(NEXT, 0, 0), to - from - 1));
                returns.push(words.len());
                words.push(word(GOTO, 0, 0));
                words[fork] = word(FORK, 0, words.len() as u16);
            }
        }
        words.push(word(STOP, 0, 0));
        let join = words.len();
        words.extend([word(REPEAT_BUMP, 0, 0), word(GOTO, 0, 3)]);
        let exit = words.len();
        words.extend([word(REPEAT_END, 0, 0), word(SIZE, 0, 0), word(EMIT, 0, 0)]);
        words[3] = word(FORK, GREEDY, exit as u16);
        for at in returns {
            words[at] = word(GOTO, 0, join as u16);
        }
        let mut program =
            Program::new(words, vec![b"prefix".to_vec()], vec![], vec![0], vec![]).unwrap();
        program.tree_sitter_query = true;
        for selected in 0..4 {
            program.words[exit + 1] = word(SIZE, 0, selected as u16 + 2);
            program.validate().unwrap();
            let expected = if selected == last {
                vec![Output {
                    task_ordinal: 0,
                    local_ordinal: 0,
                    start: 1,
                    label: 0,
                    captures: vec![Capture {
                        label: 0,
                        node: 1,
                        span: 0..2,
                        text: None,
                        properties: vec![],
                    }],
                    properties: vec![],
                    dynamic_properties: vec![],
                }]
            } else {
                vec![]
            };
            assert_eq!(
                program.execute_roots(&tape, [1]),
                Ok(expected.clone()),
                "CPU graph={graph:?} selected={selected}"
            );
            assert_eq!(
                execute(&program, &tape, 1),
                Ok(expected),
                "active graph={graph:?} selected={selected}"
            );
        }
    }
}
