//! The cost statistics' read (`rerail_logic`'s `logic::costs`), mounted by the engine: rerail-logic's own tests are
//! not run since the split.

use super::costs;

/// A form the cores run in every stratum fills the newest samples; a form run less often (the GPU's matcher wave)
/// keeps its own newest samples in the read, so its rate is fitted and its prior replaced.
#[test]
fn each_form_keeps_its_newest_samples_in_the_read() {
    let path = std::env::temp_dir().join(format!("rerail-costs-read-{}.sqlite", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let db = costs::open(&path).unwrap();
    let insert = |at: f64, lane: &str, forms: &str| {
        db.execute(
            "INSERT INTO samples VALUES (?1, ?2, 1, ?3, ?4, 1, 1000.0, 0.001, 0.001, 0.002, 0.002, 0.0, '', 0, 1, 1.0)",
            rusqlite::params![at, costs::machine(), lane, forms],
        )
        .unwrap();
    };
    for i in 0..10 {
        insert(f64::from(i), "gpu", "ast matcher wave");
    }
    for i in 0..50 {
        insert(100.0 + f64::from(i), "cpu", "strata");
    }
    let samples = costs::read_samples(&db, 20, 10).unwrap();
    let gpu = samples.iter().filter(|s| s.lane == rerail_accel::Lane::Gpu).count();
    let strata = samples.iter().filter(|s| s.forms == "strata").count();
    assert_eq!((gpu, strata), (10, 20), "the newest 20, and each form's newest 10");
    // Oldest first, as the fit and the training read them.
    assert!(samples.first().is_some_and(|s| s.lane == rerail_accel::Lane::Gpu));
    drop(db);
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
}
