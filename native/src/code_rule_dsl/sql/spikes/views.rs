//! Immutable typed capture pages and composed selections. Metadata remains separate.
//! Prototype subset: nullable integer columns (including the four physical list columns).
//! General SQLite values, xUpdate, affinity and indexes are not implemented here.

use super::measured;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Default)]
struct Budget {
    live: AtomicUsize,
    peak: AtomicUsize,
    limit: usize,
}

impl Budget {
    fn reserve(self: &Arc<Self>, bytes: usize) -> Result<Lease, &'static str> {
        self.live
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |live| {
                live.checked_add(bytes).filter(|&next| next <= self.limit)
            })
            .map_err(|_| "retained capture budget")?;
        self.peak
            .fetch_max(self.live.load(Ordering::Relaxed), Ordering::Relaxed);
        Ok(Lease {
            budget: self.clone(),
            bytes,
        })
    }
}

struct Lease {
    budget: Arc<Budget>,
    bytes: usize,
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.budget.live.fetch_sub(self.bytes, Ordering::Relaxed);
    }
}
enum Data {
    Narrow(Box<[i32]>),
    Wide(Box<[i64]>),
}
struct Page {
    data: Data,
    nulls: Box<[u64]>,
    _lease: Lease,
}
struct Selection {
    ids: Box<[u32]>,
    _lease: Lease,
}
struct View {
    columns: Vec<Arc<Page>>,
    rows: Arc<Selection>,
    budget: Arc<Budget>,
}

impl View {
    fn new(columns: Vec<Vec<Option<i64>>>, budget: Arc<Budget>) -> Result<Self, &'static str> {
        let len = columns.first().map_or(0, Vec::len);
        assert!(columns.iter().all(|column| column.len() == len));
        let columns = columns
            .into_iter()
            .map(|column| {
                let narrow = column
                    .iter()
                    .flatten()
                    .all(|&value| i32::try_from(value).is_ok());
                let width = if narrow {
                    size_of::<i32>()
                } else {
                    size_of::<i64>()
                };
                let lease = budget
                    .reserve(column.len() * width + column.len().div_ceil(64) * size_of::<u64>())?;
                let mut nulls = vec![0u64; column.len().div_ceil(64)];
                for (row, value) in column.iter().enumerate() {
                    if value.is_none() {
                        nulls[row / 64] |= 1 << (row % 64);
                    }
                }
                let data = if narrow {
                    Data::Narrow(
                        column
                            .into_iter()
                            .map(|value| value.unwrap_or_default() as i32)
                            .collect(),
                    )
                } else {
                    Data::Wide(column.into_iter().map(Option::unwrap_or_default).collect())
                };
                Ok(Arc::new(Page {
                    data,
                    nulls: nulls.into_boxed_slice(),
                    _lease: lease,
                }))
            })
            .collect::<Result<Vec<_>, &'static str>>()?;
        let lease = budget.reserve(len * size_of::<u32>())?;
        Ok(Self {
            columns,
            rows: Arc::new(Selection {
                ids: (0..len as u32).collect(),
                _lease: lease,
            }),
            budget,
        })
    }

    fn select(&self, selected: &[usize], columns: &[usize]) -> Result<Self, &'static str> {
        // Charge before publishing the selection. Pages are charged once, until their last view dies.
        let lease = self.budget.reserve(selected.len() * size_of::<u32>())?;
        let ids = selected.iter().map(|&row| self.rows.ids[row]).collect();
        Ok(Self {
            columns: columns
                .iter()
                .map(|&column| self.columns[column].clone())
                .collect(),
            rows: Arc::new(Selection { ids, _lease: lease }),
            budget: self.budget.clone(),
        })
    }

    fn value(&self, row: usize, column: usize) -> Option<i64> {
        let row = self.rows.ids[row] as usize;
        let page = &self.columns[column];
        (page.nulls[row / 64] & (1 << (row % 64)) == 0).then(|| match &page.data {
            Data::Narrow(values) => i64::from(values[row]),
            Data::Wide(values) => values[row],
        })
    }
}

#[test]
fn views_keep_nulls_lists_and_occurrences_after_register_reuse() {
    let budget = Arc::new(Budget {
        limit: 4096,
        ..Default::default()
    });
    let original = View::new(
        vec![
            vec![None, Some(7), Some(12)],
            vec![Some(-1), Some(9), Some(14)],
            vec![Some(-1), Some(10), Some(16)],
            vec![Some(0), Some(123), Some(i64::MAX)],
        ],
        budget.clone(),
    )
    .unwrap();
    let intermediate = original.select(&[2, 0, 2], &[1, 2, 3, 0]).unwrap();
    let saved = intermediate.select(&[2, 1], &[3, 0, 1, 2]).unwrap();
    drop(intermediate);
    drop(original);
    let replacement = View::new(vec![vec![Some(999)]], budget.clone()).unwrap();
    assert_eq!((saved.value(0, 0), saved.value(1, 0)), (Some(12), None));
    assert_eq!((saved.value(0, 1), saved.value(0, 2)), (Some(14), Some(16)));
    assert_eq!(saved.value(0, 3), Some(i64::MAX));
    assert_eq!(replacement.value(0, 0), Some(999));
    let live = budget.live.load(Ordering::Relaxed);
    assert!(View::new(vec![vec![None; 4096]], budget.clone()).is_err());
    assert_eq!(budget.live.load(Ordering::Relaxed), live);
    drop(saved);
    drop(replacement);
    assert_eq!(budget.live.load(Ordering::Relaxed), 0);
}

#[test]
#[ignore = "representation measurement; run with --test-threads=1 --nocapture"]
fn measure_capture_views_against_frame_copies() {
    use crate::code_rule_dsl::sql::frames::FrameStore;
    use rusqlite::{Connection, params_from_iter};
    let (rows, width, passes) = (20_000, 16, 8);
    let columns: Vec<Vec<Option<i64>>> = (0..width)
        .map(|column| {
            (0..rows)
                .map(|row| (row % 11 != 0).then_some((row * width + column) as i64))
                .collect()
        })
        .collect();
    let budget = Arc::new(Budget {
        limit: 64 << 20,
        ..Default::default()
    });
    let input = View::new(columns.clone(), budget.clone()).unwrap();
    let connection = Connection::open_in_memory().unwrap();
    let store = Arc::new(FrameStore::default());
    let names = (0..width)
        .map(|i| format!("s{i}"))
        .collect::<Vec<_>>()
        .join(",");
    for frame in 0..=passes {
        store
            .register(
                &connection,
                &format!("v{frame}(k INTEGER PRIMARY KEY,{names})"),
            )
            .unwrap();
    }
    let placeholders = (1..=width)
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let mut insert = connection
        .prepare(&format!("INSERT INTO v0({names}) VALUES({placeholders})"))
        .unwrap();
    for row in 0..rows {
        insert
            .execute(params_from_iter(columns.iter().map(|column| column[row])))
            .unwrap();
    }
    drop(insert);
    let (_, frame_ms) = measured(|| {
        for frame in 1..=passes {
            connection
                .execute(
                    &format!("INSERT INTO v{frame} SELECT * FROM v{}", frame - 1),
                    [],
                )
                .unwrap();
        }
    });
    let selected: Vec<usize> = (0..rows).rev().collect();
    let projection: Vec<usize> = (0..width).collect();
    let (views, view_ms) = measured(|| {
        let mut chain = vec![input];
        for _ in 0..passes {
            chain.push(
                chain
                    .last()
                    .unwrap()
                    .select(&selected, &projection)
                    .unwrap(),
            );
        }
        chain
    });
    // An even number of reversals restores the source order. Compare every nullable cell.
    let mut statement = connection
        .prepare(&format!("SELECT {names} FROM v{passes} ORDER BY k"))
        .unwrap();
    let mut cursor = statement.query([]).unwrap();
    for row in 0..rows {
        let actual = cursor.next().unwrap().unwrap();
        for column in 0..width {
            assert_eq!(
                actual.get::<_, Option<i64>>(column).unwrap(),
                views.last().unwrap().value(row, column)
            );
        }
    }
    assert!(cursor.next().unwrap().is_none());
    eprintln!(
        "SPIKE {}",
        serde_json::json!({"name":"capture-views", "rows":rows,"width":width,"passes":passes,
        "frameMs":frame_ms,"viewMs":view_ms,"copiedCaptureCells":rows*width*passes,
        "viewCopiedCaptureCells":0,"retainedPayloadBytes":budget.peak.load(Ordering::Relaxed),"fullCellParity":true})
    );
}
