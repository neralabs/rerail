//! The engine's test binary flushes nothing (decision 0481): its stores are thrown away, so neither SQLite nor the
//! blocks the store's rows name wait for the disk. Set before `main`, as the watchdog starts (`test_watchdog`), so
//! every store any test opens has it; a run that names a mode (`RERAIL_FLUSH=synced`) keeps its own.

#[used]
#[cfg_attr(target_os = "macos", unsafe(link_section = "__DATA,__mod_init_func"))]
#[cfg_attr(target_os = "linux", unsafe(link_section = ".init_array"))]
static START: extern "C" fn() = start;

extern "C" fn start() {
    if std::env::var_os("RERAIL_FLUSH").is_none() {
        // SAFETY: before `main`, on the one thread the process has: nothing reads the environment concurrently.
        unsafe { std::env::set_var("RERAIL_FLUSH", "none") };
    }
}

#[cfg(test)]
mod tests {
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
