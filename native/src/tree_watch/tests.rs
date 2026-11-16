use super::notices::{NOTICE_CAP, Notices};
use super::{TreeNotice, TreeWatcher};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// How long a test waits for the OS to report a change.
const DEADLINE: Duration = Duration::from_secs(20);
/// How long a test waits before it holds that nothing was reported.
const QUIET: Duration = Duration::from_millis(500);

/// A folder of its own under the temporary folder, removed when dropped.
pub(super) struct Scratch {
    base: PathBuf,
}

impl Scratch {
    pub(super) fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let unique = format!(
            "rerail-tree-watch-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let base = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&base).expect("create scratch folder");
        // FSEvents reports real paths (`/private/var`, not `/var`); a host watches physical paths.
        #[cfg(unix)]
        let base = std::fs::canonicalize(&base).expect("canonical scratch folder");
        Self { base }
    }

    pub(super) fn path(&self, relative: &str) -> PathBuf {
        if relative.is_empty() {
            return self.base.clone();
        }
        self.base.join(relative)
    }

    pub(super) fn root(&self) -> String {
        self.base.to_string_lossy().into_owned()
    }

    pub(super) fn write(&self, relative: &str, text: &str) {
        let path = self.path(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent folder");
        }
        std::fs::write(path, text).expect("write file");
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

/// `into` with `notice` added: paths appended once, flags or'ed.
fn merge(into: &mut TreeNotice, notice: TreeNotice) {
    for path in notice.changed {
        if !into.changed.contains(&path) {
            into.changed.push(path);
        }
    }
    for path in notice.renamed {
        if !into.renamed.contains(&path) {
            into.renamed.push(path);
        }
    }
    into.lost |= notice.lost;
    into.ended |= notice.ended;
}

/// Takes `root`'s notices until what they add up to satisfies `done`; panics past the deadline.
fn wait_with(
    root: &str,
    mut take: impl FnMut() -> Vec<TreeNotice>,
    done: impl Fn(&TreeNotice) -> bool,
) -> TreeNotice {
    let mut seen = TreeNotice {
        root: root.to_owned(),
        changed: Vec::new(),
        renamed: Vec::new(),
        lost: false,
        ended: false,
    };
    let started = Instant::now();
    while !done(&seen) {
        assert!(
            started.elapsed() < DEADLINE,
            "no such notice for {root}: {seen:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
        for notice in take().into_iter().filter(|notice| notice.root == root) {
            merge(&mut seen, notice);
        }
    }
    seen
}

#[cfg(target_os = "linux")]
pub(super) fn wait_for(
    notices: &Notices,
    root: &str,
    done: impl Fn(&TreeNotice) -> bool,
) -> TreeNotice {
    wait_with(root, || notices.take(), done)
}

pub(super) fn wait_for_watcher(
    watcher: &mut TreeWatcher,
    root: &str,
    done: impl Fn(&TreeNotice) -> bool,
) -> TreeNotice {
    wait_with(root, || watcher.take().expect("take"), done)
}

fn names(paths: &[String], path: &str) -> bool {
    paths.iter().any(|seen| seen == path)
}

#[test]
fn a_root_takes_at_most_one_notice_with_each_path_once() {
    let notices = Notices::default();
    notices.start("/a");
    notices.start("/b");
    notices.changed("/a", "x".to_owned());
    notices.changed("/a", "x".to_owned());
    notices.renamed("/a", "y".to_owned());
    notices.renamed("/b", "z".to_owned());
    notices.changed("/elsewhere", "w".to_owned());
    let taken = notices.take();
    assert_eq!(
        taken,
        vec![
            TreeNotice {
                root: "/a".to_owned(),
                changed: vec!["x".to_owned()],
                renamed: vec!["y".to_owned()],
                lost: false,
                ended: false,
            },
            TreeNotice {
                root: "/b".to_owned(),
                changed: Vec::new(),
                renamed: vec!["z".to_owned()],
                lost: false,
                ended: false,
            },
        ]
    );
    assert!(notices.take().is_empty(), "a take empties what it took");
}

#[test]
fn one_entry_past_the_cap_drops_the_paths_and_loses_the_root() {
    let notices = Notices::default();
    notices.start("/a");
    for index in 0..NOTICE_CAP {
        notices.changed("/a", format!("f{index}"));
    }
    notices.changed("/a", "f0".to_owned());
    let taken = notices.take();
    assert_eq!(
        taken[0].changed.len(),
        NOTICE_CAP,
        "a path seen again is no new entry"
    );
    assert!(!taken[0].lost);

    for index in 0..=NOTICE_CAP {
        notices.renamed("/a", format!("f{index}"));
    }
    let taken = notices.take();
    assert!(taken[0].lost);
    assert!(taken[0].changed.is_empty() && taken[0].renamed.is_empty());
    notices.changed("/a", "next".to_owned());
    assert_eq!(
        notices.take()[0].changed,
        vec!["next"],
        "the cap counts between takes"
    );
}

#[test]
fn an_ended_root_is_no_longer_watched_and_watched_again_is_lost() {
    let notices = Notices::default();
    notices.start("/a");
    notices.ended("/a");
    assert!(!notices.watching("/a"));
    notices.changed("/a", "after".to_owned());
    notices.start("/a");
    let taken = notices.take();
    assert_eq!(taken.len(), 1);
    assert!(taken[0].lost && !taken[0].ended && taken[0].changed.is_empty());
}

#[test]
fn a_stopped_root_drops_its_notices() {
    let notices = Notices::default();
    notices.start("/a");
    notices.changed("/a", "x".to_owned());
    notices.lost_all();
    notices.stop("/a");
    notices.changed("/a", "y".to_owned());
    assert!(notices.take().is_empty());
}

#[cfg(any(target_os = "macos", target_os = "linux", windows))]
mod adapter {
    use super::{DEADLINE, NOTICE_CAP, QUIET, Scratch, names, wait_for_watcher};
    use crate::tree_watch::TreeWatcher;
    use std::time::{Duration, Instant};

    /// Watches `scratch`'s root and its `src` folder. Tests change files under `src`: on Windows a
    /// change among the root's own entries breaks the oplock and ends the watch.
    fn watched(scratch: &Scratch) -> (TreeWatcher, String) {
        std::fs::create_dir_all(scratch.path("src")).expect("create src");
        let mut watcher = TreeWatcher::new();
        let root = scratch.root();
        let folders = vec![String::new(), "src".to_owned()];
        assert!(
            watcher.watch(root.clone(), folders).expect("watch"),
            "{root} is watchable"
        );
        (watcher, root)
    }

    #[test]
    fn a_write_is_a_changed_path() {
        let scratch = Scratch::new("write");
        scratch.write("src/a.txt", "a");
        let (mut watcher, root) = watched(&scratch);
        scratch.write("src/a.txt", "written");
        wait_for_watcher(&mut watcher, &root, |notice| {
            names(&notice.changed, "src/a.txt")
        });
        watcher.close().expect("close");
    }

    #[test]
    fn a_create_and_a_delete_are_renamed_paths() {
        let scratch = Scratch::new("create-delete");
        scratch.write("src/old.txt", "old");
        let (mut watcher, root) = watched(&scratch);
        scratch.write("src/new.txt", "new");
        std::fs::remove_file(scratch.path("src/old.txt")).expect("delete");
        wait_for_watcher(&mut watcher, &root, |notice| {
            names(&notice.renamed, "src/new.txt") && names(&notice.renamed, "src/old.txt")
        });
        watcher.close().expect("close");
    }

    #[test]
    fn a_path_below_a_folder_is_slash_separated() {
        let scratch = Scratch::new("nested");
        scratch.write("src/deep/a.ts", "a");
        let mut watcher = TreeWatcher::new();
        let root = scratch.root();
        let folders = ["", "src", "src/deep"].map(str::to_owned).to_vec();
        assert!(watcher.watch(root.clone(), folders).expect("watch"));
        scratch.write("src/deep/a.ts", "b");
        wait_for_watcher(&mut watcher, &root, |notice| {
            names(&notice.changed, "src/deep/a.ts")
        });
        watcher.close().expect("close");
    }

    #[test]
    fn renaming_the_root_ends_its_watch_and_it_can_be_watched_again() {
        let scratch = Scratch::new("root-rename");
        scratch.write("checkout/src/a.txt", "a");
        let mut watcher = TreeWatcher::new();
        let root = scratch.path("checkout").to_string_lossy().into_owned();
        let folders = vec![String::new(), "src".to_owned()];
        assert!(watcher.watch(root.clone(), folders.clone()).expect("watch"));
        std::fs::rename(scratch.path("checkout"), scratch.path("moved")).expect("rename the root");
        let notice = wait_for_watcher(&mut watcher, &root, |notice| notice.ended);
        assert!(notice.ended);

        std::fs::rename(scratch.path("moved"), scratch.path("checkout")).expect("rename back");
        assert!(watcher.watch(root.clone(), folders).expect("watch again"));
        scratch.write("checkout/src/a.txt", "again");
        wait_for_watcher(&mut watcher, &root, |notice| {
            names(&notice.changed, "src/a.txt")
        });
        watcher.close().expect("close");
    }

    #[test]
    fn an_unwatched_root_reports_nothing() {
        let scratch = Scratch::new("unwatch");
        scratch.write("src/a.txt", "a");
        let (mut watcher, root) = watched(&scratch);
        watcher.unwatch(root.clone()).expect("unwatch");
        scratch.write("src/a.txt", "written");
        scratch.write("src/b.txt", "created");
        std::thread::sleep(QUIET);
        assert!(watcher.take().expect("take").is_empty());
        watcher.close().expect("close");
    }

    #[test]
    fn more_entries_than_the_cap_lose_the_root() {
        let scratch = Scratch::new("cap");
        let (mut watcher, root) = watched(&scratch);
        for index in 0..=NOTICE_CAP {
            scratch.write(&format!("src/f{index}"), "x");
        }
        // Nothing is taken until the root is lost: the cap counts between two takes.
        let started = Instant::now();
        while !watcher.notices.is_lost(&root) {
            assert!(started.elapsed() < DEADLINE, "{root} was not lost");
            std::thread::sleep(Duration::from_millis(20));
        }
        let taken = watcher.take().expect("take");
        assert!(taken[0].lost && taken[0].changed.is_empty() && taken[0].renamed.is_empty());
        watcher.close().expect("close");
    }

    #[test]
    fn a_root_that_is_not_there_is_not_watched() {
        let scratch = Scratch::new("missing");
        let mut watcher = TreeWatcher::new();
        let root = scratch.path("gone").to_string_lossy().into_owned();
        assert!(!watcher.watch(root, vec![String::new()]).expect("watch"));
        watcher.close().expect("close");
    }

    #[test]
    fn a_closed_watcher_watches_nothing() {
        let scratch = Scratch::new("closed");
        let (mut watcher, root) = watched(&scratch);
        watcher.close().expect("close");
        watcher.close().expect("close twice");
        scratch.write("src/a.txt", "a");
        std::thread::sleep(QUIET);
        assert!(watcher.take().expect("take").is_empty());
        assert!(
            !watcher
                .watch(root, vec![String::new()])
                .expect("watch after close")
        );
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
#[test]
fn an_os_without_an_adapter_watches_nothing() {
    let mut watcher = TreeWatcher::new();
    assert!(
        !watcher
            .watch("/".to_owned(), vec![String::new()])
            .expect("watch")
    );
}
