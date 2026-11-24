//! The engine's test binary flushes nothing (decision 0481): its stores are thrown away, so neither SQLite nor the
//! blocks the store's rows name wait for the disk. Set before `main`, as the watchdog starts (`test_watchdog`), so
//! every store any test opens has it; a run that names a mode (`RERAIL_FLUSH=synced`) keeps its own.
//!
//! Under nextest, whose tests are each a process of their own, as many at once as its test threads, a test's pool
//! is its share of the machine's cores (`RERAIL_THREADS`, at least 2), unless the run names one. Each process's pool
//! was all the cores: six tests at once on a 20-core host each ran 20 threads, and their spinning and switching was
//! 13% of the corpus suite's CPU, its wall 35 s against 27 s with a share each (spark2, the farm's 6 test threads).
//!
//! In one process (`cargo test`), whose tests run as many at once as the harness's test threads, each test's machine
//! has a pool of its own: a test's pool is the same share where the run names its test threads (`--test-threads`,
//! `RUST_TEST_THREADS`) and does not run one test by name (`--exact`). With all the cores each, the corpus suite at 6
//! test threads took 89.9 s of user CPU against 83.1 s with a share each (spark2), and nextest's 77.2 s.

#[used]
#[cfg_attr(target_os = "macos", unsafe(link_section = "__DATA,__mod_init_func"))]
#[cfg_attr(target_os = "linux", unsafe(link_section = ".init_array"))]
static START: extern "C" fn() = start;

extern "C" fn start() {
    if std::env::var_os("RERAIL_FLUSH").is_none() {
        // SAFETY: before `main`, on the one thread the process has: nothing reads the environment concurrently.
        unsafe { std::env::set_var("RERAIL_FLUSH", "none") };
    }
    if let Some(share) = nextest_share().or_else(harness_share) {
        // SAFETY: as above.
        unsafe { std::env::set_var("RERAIL_THREADS", share.to_string()) };
    }
}

/// The cores over `tests` run at once, at least 2 (a run's main thread waits on its pool's jobs).
fn share_of(tests: usize) -> usize {
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    (cores / tests.max(1)).max(2)
}

/// A test's share of the cores in one process of many tests (module docs): `None` under nextest, where the run names
/// its pool, where it names no test threads, or runs one test by name.
fn harness_share() -> Option<usize> {
    if std::env::var_os("NEXTEST").is_some() || std::env::var_os("RERAIL_THREADS").is_some() {
        return None;
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    named_test_threads(&args, std::env::var("RUST_TEST_THREADS").ok().as_deref()).map(share_of)
}

/// The test threads the harness's arguments (`--test-threads N`, `--test-threads=N`) or `RUST_TEST_THREADS` name; the
/// arguments first, as the harness reads them. `None` where neither names a count, or `--exact` names one test.
fn named_test_threads(args: &[String], env: Option<&str>) -> Option<usize> {
    if args.iter().any(|a| a == "--exact") {
        return None;
    }
    let arg = args.iter().enumerate().find_map(|(i, a)| match a.strip_prefix("--test-threads") {
        Some("") => args.get(i + 1).cloned(),
        Some(v) => v.strip_prefix('=').map(str::to_owned),
        None => None,
    });
    arg.as_deref().or(env).and_then(|v| v.trim().parse::<usize>().ok()).filter(|&n| n > 0)
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
    Some(share_of(tests))
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

    /// One process of many tests reads its test threads from the harness's arguments first, then `RUST_TEST_THREADS`,
    /// and takes no share for a test run by name (`named_test_threads`).
    #[test]
    fn a_process_of_many_tests_reads_its_test_threads() {
        let args = |a: &[&str]| a.iter().map(|s| (*s).to_owned()).collect::<Vec<String>>();
        assert_eq!(super::named_test_threads(&args(&["--test-threads", "6", "-q"]), None), Some(6));
        assert_eq!(super::named_test_threads(&args(&["--test-threads=4"]), Some("6")), Some(4));
        assert_eq!(super::named_test_threads(&args(&["-q"]), Some("6")), Some(6));
        assert_eq!(super::named_test_threads(&args(&["-q"]), None), None);
        assert_eq!(super::named_test_threads(&args(&["--exact", "a::b", "--test-threads", "6"]), None), None);
        assert_eq!(super::named_test_threads(&args(&["--test-threads", "0"]), None), None);
        assert!(super::share_of(6) >= 2);
    }

    /// A test binary allocates as the engine does, through jemalloc (`rerail_prof::heap::Sampled`): its thread's
    /// counter of bytes allocated sees a Rust allocation. A test crate that declared no allocator ran on the system's.
    #[cfg(not(any(windows, target_family = "wasm", feature = "system-allocator")))]
    #[test]
    fn a_test_binary_allocates_through_jemalloc() {
        let allocated = || unsafe { tikv_jemalloc_ctl::raw::read::<u64>(b"thread.allocated\0") }.expect("jemalloc's thread counter");
        let before = allocated();
        let bytes = std::hint::black_box(vec![0u8; 1 << 20]);
        let after = allocated();
        assert!(after - before >= bytes.len() as u64, "{} bytes counted by jemalloc for {} allocated", after - before, bytes.len());
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
