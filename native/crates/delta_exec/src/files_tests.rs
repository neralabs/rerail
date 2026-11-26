//! B4's checks: l0's tape through the file layer.

use super::*;
use rerail_blocks::maps::Placed as _;
use std::time::{Duration, SystemTime};

/// A root of its own for test `name`, holding a copy of l0, its stat older than the racy window (a file written in
/// the window's tick is read again, as git's racy rule has it).
fn root(name: &str) -> (PathBuf, Files) {
    let dir = std::env::temp_dir().join(format!("rerail-b4-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    write(&dir.join("bootstrap-l0.rerail"), crate::L0, 60);
    let files = Files::new(dir.clone(), dir.join("observer.db"));
    (dir, files)
}

/// The racy window these tests run under (`RERAIL_RACY_MS`), as the engine's tests narrow it (`TEST_RACY_MS`): a file
/// written (its ctime now) is trusted by its stat once twice the window has passed.
fn racy_window() -> rerail_context::Bound {
    rerail_context::bind(rerail_context::Switches(Arc::new(vec![("RERAIL_RACY_MS".into(), "100".into())])))
}

fn past_racy_window() {
    std::thread::sleep(Duration::from_millis(200));
}

/// Writes `text` at `path`, its modification time `ago` seconds back.
fn write(path: &std::path::Path, text: &str, ago: u64) {
    std::fs::write(path, text).unwrap();
    let f = std::fs::File::options().write(true).open(path).unwrap();
    f.set_modified(SystemTime::now() - Duration::from_secs(ago)).unwrap();
}

#[test]
fn a_document_is_its_path_its_module_and_a_tape_from_nothing() {
    let (_, mut files) = root("cold");
    let (mut blobs, mut dir) = (Blobs::default(), Directory::default());
    let d = files.answer("bootstrap-l0.rerail", &mut blobs, &mut dir).unwrap();
    assert_eq!((d.path.as_str(), d.module.as_str(), d.before), ("bootstrap-l0.rerail", "bootstrap-l0", None));
    let t = blobs.get(&d.after).unwrap();
    assert_eq!(t.src.as_str(), crate::L0);
    assert!(!t.tape.is_empty());
}

/// The tape lies where the parser wrote it: a tape block's room.
#[test]
fn the_parser_writes_the_tape_into_a_tape_block() {
    let (_, mut files) = root("room");
    let (mut blobs, mut dir) = (Blobs::default(), Directory::default());
    let d = files.answer("bootstrap-l0.rerail", &mut blobs, &mut dir).unwrap();
    let t = blobs.get(&d.after).unwrap();
    let home::Home::Own(v) = t.tape.home() else { panic!("a tape the parser wrote is its writer's") };
    let (block, _) = v.place().expect("the tape's bytes in a block");
    assert_eq!(block.header().kind, kind::TAPE);
}

#[test]
fn an_unchanged_file_gives_the_identity_with_no_read_past_its_stat() {
    let _racy = racy_window();
    let (_, mut files) = root("same");
    past_racy_window();
    let (mut blobs, mut dir) = (Blobs::default(), Directory::default());
    let first = files.answer("bootstrap-l0.rerail", &mut blobs, &mut dir).unwrap();
    let (c0, reads) = (files.counts(), files.reads);
    let again = files.answer("bootstrap-l0.rerail", &mut blobs, &mut dir).unwrap();
    assert!(again.is_identity(), "{again:?}");
    assert_eq!(again.after, first.after);
    let c = files.counts().since(&c0);
    assert_eq!((c.hashed, c.bytes), (0, 0), "no byte read: {c:?}");
    assert_eq!(c.reused, 1, "the answer reused on its stat: {c:?}");
    assert_eq!(files.reads, reads);
    assert_eq!(blobs.len(), 1);
}

#[test]
fn an_edited_file_is_read_once_and_placed_once() {
    let (root, mut files) = root("edit");
    let (mut blobs, mut dir) = (Blobs::default(), Directory::default());
    let first = files.answer("bootstrap-l0.rerail", &mut blobs, &mut dir).unwrap();
    write(&root.join("bootstrap-l0.rerail"), &format!("{}\nb4_added(x) :- rule(x).\n", crate::L0), 30);
    let (c0, reads) = (files.counts(), files.reads);
    let edited = files.answer("bootstrap-l0.rerail", &mut blobs, &mut dir).unwrap();
    assert_eq!(edited.before, Some(first.after));
    assert_ne!(edited.after, first.after);
    assert_eq!(files.reads, reads + 1, "read once, placed once (read_placed places what it reads)");
    assert_eq!(files.counts().since(&c0).hashed, 1);
    assert!(blobs.get(&edited.after).unwrap().src.as_str().contains("b4_added"));
}

#[test]
fn a_file_that_does_not_parse_is_refused_naming_its_path() {
    let (root, mut files) = root("bad");
    write(&root.join("broken.rerail"), "this is not a rule (\n", 30);
    let (mut blobs, mut dir) = (Blobs::default(), Directory::default());
    let e = files.answer("broken.rerail", &mut blobs, &mut dir).unwrap_err();
    assert!(e.starts_with("broken.rerail: "), "{e}");
    let e = files.answer("missing.rerail", &mut blobs, &mut dir).unwrap_err();
    assert!(e.starts_with("missing.rerail: "), "{e}");
}
