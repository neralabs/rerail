use super::super::{capture_run_entry_tests::tape, capture_run_tests};
use super::*;

fn slow_initial_zero_path(mut program: Program) -> Program {
    let at = 9;
    let extra = 80;
    let mut words = Vec::new();
    for (pc, &w) in program.words.iter().enumerate() {
        if pc == at {
            words.extend((0..extra).map(|step| word(GOTO, 0, (at + step + 1) as u16)));
        }
        let (op, mode, arg) = split(w);
        words.push(if matches!(op, FORK | GOTO) && usize::from(arg) >= at {
            word(op, mode, arg + extra as u16)
        } else {
            w
        });
    }
    program.words = words;
    program.validate().unwrap();
    program
}

// Split by program and length so the four parts run in parallel; together they cover what the
// single test covered (both programs, every length 0..=6).
#[test]
fn streaming_runs_preserve_all_zero_capture_joins_raw_ordinals_and_failure_boundaries() {
    streaming_runs_preserve_joins(capture_run_tests::program(), 0..=5);
}

#[test]
fn streaming_runs_preserve_all_zero_capture_joins_at_length_six() {
    streaming_runs_preserve_joins(capture_run_tests::program(), 6..=6);
}

#[test]
fn slow_initial_streaming_runs_preserve_all_zero_capture_joins() {
    streaming_runs_preserve_joins(slow_initial_zero_path(capture_run_tests::program()), 0..=5);
}

#[test]
fn slow_initial_streaming_runs_preserve_all_zero_capture_joins_at_length_six() {
    streaming_runs_preserve_joins(slow_initial_zero_path(capture_run_tests::program()), 6..=6);
}

fn streaming_runs_preserve_joins(p: Program, lengths: std::ops::RangeInclusive<usize>) {
    {
        let prepared = prepare(&p).unwrap();
        for length in lengths {
            for bits in 0..(1usize << length) {
                let values = (0..length)
                    .map(|index| {
                        if bits & (1 << index) == 0 {
                            format!("v{index}")
                        } else {
                            index.to_string()
                        }
                    })
                    .chain(std::iter::once("\"end\"".to_owned()))
                    .collect::<Vec<_>>();
                let source = format!("f({})\n", values.join(","));
                let doc = tape(&source);
                let roots = (0..doc.transducer_len() as u32)
                    .filter(|&n| doc.transducer_text(n) == b"(")
                    .collect::<Vec<_>>();
                assert_eq!(roots.len(), 1);
                let root = roots[0];
                let reference = p.execute_roots(&doc, [root]).unwrap();
                let ranges = prepared.execute(&doc, root, Limits::default()).unwrap();
                let streaming = prepared
                    .execute_streaming(&doc, root, Limits::default())
                    .unwrap();
                assert_eq!(ranges.outputs, reference, "{source}");
                assert_eq!(streaming.outputs, reference, "{source}");
                assert_eq!(streaming.report, ranges.report, "{source}");
                for path in 0..=220 {
                    let expected =
                        p.execute_roots_from_with_work_limit(&doc, [root], 0, None, path);
                    let actual = prepared
                        .execute_streaming(
                            &doc,
                            root,
                            Limits {
                                path,
                                task: MAX_WORK_PER_TASK,
                            },
                        )
                        .map(|r| r.outputs);
                    assert_eq!(actual, expected, "source={source} path={path}");
                }
                let total = ranges.report.task_work;
                for task in [0, 1, total / 2, total - 1, total, total + 1] {
                    let _budget = capture_run_tests::TaskLimit::enter(task);
                    let expected = p.execute_roots(&doc, [root]);
                    let actual = prepared
                        .execute_streaming(
                            &doc,
                            root,
                            Limits {
                                path: MAX_STEPS_PER_PATH,
                                task,
                            },
                        )
                        .map(|r| r.outputs);
                    assert_eq!(
                        actual, expected,
                        "source={source} task={task} total={total}"
                    );
                }
            }
        }
    }
}

#[test]
fn streaming_runs_refuse_separated_body_domains_without_dropping_their_results() {
    let p = capture_run_tests::program();
    let prepared = prepare(&p).unwrap();
    let doc = tape("f(a,1,\"middle\",b,\"end\")\n");
    let root = (0..doc.transducer_len() as u32)
        .find(|&n| doc.transducer_text(n) == b"(")
        .unwrap();
    let expected = p.execute_roots(&doc, [root]).unwrap();
    assert_eq!(expected.len(), 2);
    assert_eq!(
        prepared
            .execute(&doc, root, Limits::default())
            .unwrap()
            .outputs,
        expected
    );
    assert_eq!(
        prepared
            .execute_streaming(&doc, root, Limits::default())
            .map(|r| r.outputs),
        Err(Error::UnsupportedBackend)
    );
}
