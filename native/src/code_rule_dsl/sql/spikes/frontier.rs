//! Persistent first-match and threshold-count cursors. Candidate order is supplied by a walk.
//! Dependencies from failed/unknown candidates survive early termination. A changed recursive
//! answer reopens affected roots. This is an execution-state prototype, not SQL lowering.

use super::measured;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verdict {
    No,
    Yes(u32),
    Unknown,
}

#[derive(Clone, Copy)]
enum Mode {
    First {
        strict: bool,
    },
    Count {
        min: usize,
        max: Option<usize>,
        exact: bool,
    },
}

struct Cursor {
    next: usize,
    yes: usize,
    unknown: usize,
    open: bool,
    dependencies: BTreeSet<usize>,
    answer: Option<Verdict>,
}

impl Cursor {
    fn new() -> Self {
        Self {
            next: 0,
            yes: 0,
            unknown: 0,
            open: false,
            dependencies: BTreeSet::new(),
            answer: None,
        }
    }

    fn advance(
        &mut self,
        mode: Mode,
        len: usize,
        quantum: usize,
        mut test: impl FnMut(usize) -> Result<(Verdict, Vec<usize>), &'static str>,
    ) -> Result<(), &'static str> {
        assert!(quantum > 0);
        if self.answer.is_some() {
            return Ok(());
        }
        for _ in 0..quantum {
            if self.next == len {
                break;
            }
            let (verdict, dependencies) = test(self.next)?;
            self.dependencies.extend(dependencies);
            self.next += 1;
            match verdict {
                Verdict::Yes(_) => self.yes += 1,
                Verdict::Unknown => self.unknown += 1,
                Verdict::No => {}
            }
            self.answer = match (mode, verdict) {
                (Mode::First { .. }, Verdict::Yes(capture)) => Some(Verdict::Yes(capture)),
                (Mode::First { strict: true }, Verdict::Unknown) => Some(Verdict::Unknown),
                (Mode::Count { max: Some(max), .. }, _) if self.yes > max => Some(Verdict::No),
                (
                    Mode::Count {
                        min,
                        max: None,
                        exact: false,
                    },
                    _,
                ) if self.yes >= min => Some(Verdict::Yes(0)),
                _ => None,
            };
            if self.answer.is_some() {
                return Ok(());
            }
        }
        if self.next == len {
            self.answer = Some(match mode {
                Mode::First { .. } => {
                    if self.unknown > 0 || self.open {
                        Verdict::Unknown
                    } else {
                        Verdict::No
                    }
                }
                Mode::Count { min, max, exact } => {
                    let high = self.yes + self.unknown;
                    if (!self.open && high < min) || max.is_some_and(|max| self.yes > max) {
                        Verdict::No
                    } else if self.yes < min
                        || max.is_some_and(|max| self.open || high > max)
                        || (exact && (self.open || self.unknown > 0))
                    {
                        Verdict::Unknown
                    } else {
                        Verdict::Yes(if exact { self.yes as u32 } else { 0 })
                    }
                }
            });
        }
        Ok(())
    }

    fn changed(&mut self, unit: usize) -> bool {
        if !self.dependencies.contains(&unit) {
            return false;
        }
        *self = Self::new();
        true
    }
}

fn reference(mode: Mode, events: &[Verdict]) -> Verdict {
    match mode {
        Mode::First { strict } => {
            let mut unknown = false;
            for event in events {
                match event {
                    Verdict::Yes(_) => return *event,
                    Verdict::Unknown if strict => return Verdict::Unknown,
                    Verdict::Unknown => unknown = true,
                    Verdict::No => {}
                }
            }
            if unknown {
                Verdict::Unknown
            } else {
                Verdict::No
            }
        }
        Mode::Count { min, max, exact } => {
            let low = events
                .iter()
                .filter(|v| matches!(v, Verdict::Yes(_)))
                .count();
            let high = events.iter().filter(|v| !matches!(v, Verdict::No)).count();
            if high < min || max.is_some_and(|max| low > max) {
                Verdict::No
            } else if low < min || max.is_some_and(|max| high > max) || (exact && high != low) {
                Verdict::Unknown
            } else {
                Verdict::Yes(if exact { low as u32 } else { 0 })
            }
        }
    }
}

#[test]
fn progressive_cursors_preserve_first_captures_and_count_intervals() {
    for code in 0..729usize {
        let mut code = code;
        let events: Vec<_> = (0..6)
            .map(|index| {
                let value = match code % 3 {
                    0 => Verdict::No,
                    1 => Verdict::Unknown,
                    _ => Verdict::Yes(index),
                };
                code /= 3;
                value
            })
            .collect();
        let mut modes = vec![Mode::First { strict: false }, Mode::First { strict: true }];
        for min in 0..=3 {
            for max in [None, Some(0), Some(2), Some(6)] {
                for exact in [false, true] {
                    modes.push(Mode::Count { min, max, exact });
                }
            }
        }
        for mode in modes {
            for quantum in [1, 2, 4, 16] {
                let mut cursor = Cursor::new();
                while cursor.answer.is_none() {
                    cursor
                        .advance(mode, events.len(), quantum, |index| {
                            Ok((events[index], vec![index]))
                        })
                        .unwrap();
                }
                assert_eq!(cursor.answer, Some(reference(mode, &events)));
            }
        }
    }
}

#[test]
fn recursive_changes_reopen_a_previously_failed_prefix() {
    let mode = Mode::First { strict: false };
    let mut cursor = Cursor::new();
    cursor
        .advance(mode, 3, 3, |index| {
            Ok((
                if index == 2 {
                    Verdict::Yes(200)
                } else {
                    Verdict::No
                },
                vec![index],
            ))
        })
        .unwrap();
    assert_eq!(cursor.answer, Some(Verdict::Yes(200)));
    assert!(!cursor.changed(99));
    assert!(cursor.changed(0));
    cursor
        .advance(mode, 3, 3, |index| {
            Ok((Verdict::Yes(index as u32), vec![index]))
        })
        .unwrap();
    assert_eq!(cursor.answer, Some(Verdict::Yes(0)));
    assert_eq!(cursor.next, 1);
    let mut cursor = Cursor::new();
    assert!(
        cursor
            .advance(mode, 3, 3, |index| if index == 1 {
                Err("terminal failure")
            } else {
                Ok((Verdict::No, vec![9]))
            })
            .is_err()
    );
    assert!(
        cursor.answer.is_none(),
        "a failed phase must not publish a result"
    );
}

// A phase collects work from every unresolved root, evaluates one request set, then commits
// each root's ordered prefix. Quantum controls dispatch frequency and overfetch separately.
fn batched(
    roots: usize,
    candidates: usize,
    quantum: usize,
    mode: Mode,
    test: impl Fn(usize) -> Verdict,
) -> (Vec<Cursor>, usize, usize) {
    let mut cursors: Vec<_> = (0..roots).map(|_| Cursor::new()).collect();
    let (mut phases, mut produced) = (0, 0);
    loop {
        let mut requests = Vec::new();
        let mut ranges = Vec::new();
        for (root, cursor) in cursors
            .iter()
            .enumerate()
            .filter(|(_, c)| c.answer.is_none())
        {
            let start = requests.len();
            requests.extend(cursor.next..(cursor.next + quantum).min(candidates));
            ranges.push((root, start..requests.len()));
        }
        if ranges.is_empty() {
            break;
        }
        phases += 1;
        produced += requests.len();
        let answers: Vec<_> = requests.into_iter().map(&test).collect();
        for (root, range) in ranges {
            let start = cursors[root].next;
            cursors[root]
                .advance(mode, candidates, quantum, |index| {
                    Ok((answers[range.start + index - start], vec![]))
                })
                .unwrap();
        }
    }
    (cursors, phases, produced)
}

#[test]
fn batched_frontiers_commit_the_ordered_prefix_and_stop_counts() {
    let events = [
        Verdict::No,
        Verdict::Unknown,
        Verdict::Yes(7),
        Verdict::Yes(9),
    ];
    for quantum in [1, 2, 8] {
        for mode in [
            Mode::First { strict: true },
            Mode::First { strict: false },
            Mode::Count {
                min: 1,
                max: None,
                exact: false,
            },
            Mode::Count {
                min: 0,
                max: Some(1),
                exact: true,
            },
        ] {
            let (cursors, _, _) = batched(17, events.len(), quantum, mode, |index| events[index]);
            assert!(
                cursors
                    .iter()
                    .all(|c| c.answer == Some(reference(mode, &events)))
            );
        }
    }
}

#[test]
#[ignore = "representation measurement; run with --test-threads=1 --nocapture"]
fn measure_progressive_work_and_quantum() {
    let (roots, candidates) = (512, 4096);
    for target in [0, 127, candidates - 1, candidates] {
        for quantum in [1, 8, 32, 128] {
            let ((cursors, phases, produced), ms) = measured(|| {
                batched(
                    roots,
                    candidates,
                    quantum,
                    Mode::First { strict: false },
                    |index| {
                        if std::hint::black_box(index) == target {
                            Verdict::Yes(index as u32)
                        } else {
                            Verdict::No
                        }
                    },
                )
            });
            assert!(cursors.iter().all(|c| c.answer
                == Some(if target < candidates {
                    Verdict::Yes(target as u32)
                } else {
                    Verdict::No
                })));
            let committed: usize = cursors.iter().map(|c| c.next).sum();
            eprintln!(
                "SPIKE {}",
                serde_json::json!({"name":"progressive-frontier","roots":roots,"candidatesPerRoot":candidates,
            "target":target,"quantum":quantum,"phases":phases,"produced":produced,"committed":committed,
            "eagerPairs":roots*candidates,"ms":ms})
            );
        }
    }
    for (name, mode) in [
        (
            "min3",
            Mode::Count {
                min: 3,
                max: None,
                exact: false,
            },
        ),
        (
            "max3",
            Mode::Count {
                min: 0,
                max: Some(3),
                exact: false,
            },
        ),
        (
            "exact",
            Mode::Count {
                min: 0,
                max: None,
                exact: true,
            },
        ),
    ] {
        let ((cursors, phases, produced), ms) = measured(|| {
            batched(roots, candidates, 32, mode, |index| {
                if index.is_multiple_of(2) {
                    Verdict::Yes(index as u32)
                } else {
                    Verdict::No
                }
            })
        });
        let expected = reference(
            mode,
            &(0..candidates)
                .map(|index| {
                    if index.is_multiple_of(2) {
                        Verdict::Yes(index as u32)
                    } else {
                        Verdict::No
                    }
                })
                .collect::<Vec<_>>(),
        );
        assert!(cursors.iter().all(|c| c.answer == Some(expected)));
        eprintln!(
            "SPIKE {}",
            serde_json::json!({"name":"progressive-count","case":name,"quantum":32,
            "roots":roots,"candidatesPerRoot":candidates,"phases":phases,"produced":produced,"eagerPairs":roots*candidates,"ms":ms})
        );
    }
}

#[test]
fn unknown_stops_leave_the_count_upper_bound_open() {
    let cases = [
        (
            Mode::Count {
                min: 3,
                max: None,
                exact: false,
            },
            Verdict::Unknown,
        ),
        (
            Mode::Count {
                min: 0,
                max: Some(0),
                exact: false,
            },
            Verdict::Unknown,
        ),
        (
            Mode::Count {
                min: 0,
                max: None,
                exact: true,
            },
            Verdict::Unknown,
        ),
        (
            Mode::Count {
                min: 0,
                max: None,
                exact: false,
            },
            Verdict::Yes(0),
        ),
        (Mode::First { strict: true }, Verdict::Unknown),
        (Mode::First { strict: false }, Verdict::Unknown),
    ];
    for (mode, expected) in cases {
        let mut cursor = Cursor::new();
        cursor.open = true;
        cursor.dependencies.insert(99);
        cursor.advance(mode, 0, 8, |_| unreachable!()).unwrap();
        assert_eq!(cursor.answer, Some(expected));
        assert!(cursor.changed(99));
    }
}
