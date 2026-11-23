//! The engine's test binary flushes nothing (decision 0481): its stores are thrown away, so neither SQLite nor the
//! blocks the store's rows name wait for the disk. Set before `main`, as the watchdog starts (`test_watchdog`), so
//! every store any test opens has it; a run that names a mode (`RERAIL_FLUSH=synced`) keeps its own.
//!
//! Under nextest, whose tests are each a process of their own, as many at once as its test threads, a test's pool
//! is its share of the machine's cores (`RERAIL_THREADS`, at least 2), unless the run names one. Each process's pool
//! was all the cores: six tests at once on a 20-core host each ran 20 threads, and their spinning and switching was
//! 13% of the corpus suite's CPU, its wall 35 s against 27 s with a share each (spark2, the farm's 6 test threads).

#[used]
#[cfg_attr(target_os = "macos", unsafe(link_section = "__DATA,__mod_init_func"))]
#[cfg_attr(target_os = "linux", unsafe(link_section = ".init_array"))]
static START: extern "C" fn() = start;

extern "C" fn start() {
    if std::env::var_os("RERAIL_FLUSH").is_none() {
        // SAFETY: before `main`, on the one thread the process has: nothing reads the environment concurrently.
        unsafe { std::env::set_var("RERAIL_FLUSH", "none") };
    }
    if let Some(share) = nextest_share() {
        // SAFETY: as above.
        unsafe { std::env::set_var("RERAIL_THREADS", share.to_string()) };
    }
}

/// A nextest test process's share of the cores (module docs): the cores over its test threads (`NEXTEST_TEST_THREADS`,
/// else one a core, nextest's default), at least 2 (a run's main thread waits on its pool's jobs); `None` outside
/// nextest, or where the run names its pool.
fn nextest_share() -> Option<usize> {
    if std::env::var_os("NEXTEST").is_none() || std::env::var_os("RERAIL_THREADS").is_some() {
        return None;
    }
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let tests = std::env::var("NEXTEST_TEST_THREADS").ok().and_then(|v| v.trim().parse::<usize>().ok()).filter(|&n| n > 0).unwrap_or(cores);
    Some((cores / tests).max(2))
}

#[cfg(test)]
mod tests {
    /// A nextest test process runs a pool of its share of the cores, at least 2 (`nextest_share`), and the pool a run
    /// names is its own.
    #[test]
    fn a_nextest_process_runs_its_share_of_the_cores() {
        if std::env::var_os("NEXTEST").is_none() {
            return;
        }
        let pool: usize = std::env::var("RERAIL_THREADS").expect("a nextest test's pool").parse().expect("a count");
        assert!(pool >= 2, "a pool of {pool}");
        let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
        assert!(pool <= cores.max(2), "a pool of {pool} on {cores} cores");
        assert_eq!(super::nextest_share(), None, "a pool named is the run's own");
    }

    /// A store a test opens flushes nothing (`RERAIL_FLUSH=none`, unless the run names a mode): its connections'
    /// `synchronous` is `OFF`, which its blocks' write-backs follow.
    #[test]
    fn a_tests_store_flushes_nothing() {
        use crate::content_store::Flush;
        if std::env::var("RERAIL_FLUSH").as_deref() != Ok("none") {
            return;
        }
        assert_eq!(crate::content_store::store_flush().unwrap(), Flush::None);
        let path = crate::code_rule_dsl::fixtures::temp();
        let store = crate::content_store::ContentStore::open(&path).unwrap();
        assert_eq!(crate::content_store::flush_of(&store.open_connection().unwrap()), Flush::None);
    }
}
