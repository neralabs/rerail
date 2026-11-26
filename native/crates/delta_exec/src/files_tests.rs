//! B4's checks: l0's tape through the file layer.

use super::*;
use std::time::{Duration, SystemTime};

/// A root of its own for test `name`, holding a copy of l0, its stat older than the racy window (a file written in
/// the window's tick is read again, as git's racy rule has it).
pub(crate) fn root(name: &str) -> (PathBuf, Files) {
    let dir = std::env::temp_dir().join(format!("rerail-b4-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    write(&dir.join("bootstrap-l0.rerail"), crate::L0, 60);
    let files = Files::new(dir.clone(), dir.join("observer.db"));
    (dir, files)
}

/// The racy window these tests run under (`RERAIL_RACY_MS`), as the engine's tests narrow it (`TEST_RACY_MS`): a file
/// written (its ctime now) is trusted by its stat once twice the window has passed.
pub(crate) fn racy_window() -> rerail_context::Bound {
    rerail_context::bind(rerail_context::Switches(Arc::new(vec![("RERAIL_RACY_MS".into(), "100".into())])))
}

pub(crate) fn past_racy_window() {
    std::thread::sleep(Duration::from_millis(200));
}

/// Writes `text` at `path`, its modification time `ago` seconds back.
pub(crate) fn write(path: &std::path::Path, text: &str, ago: u64) {
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
    let (s, t) = (blobs.get(&d.after.source).unwrap(), blobs.get(&d.after.tape).unwrap());
    let l = dir.read("t", &[t.block]).unwrap();
    assert_eq!(&l.block(0)[s.at..s.at + s.len], crate::L0.as_bytes());
    assert!(reads(l.block(0), s, t).len() > 100, "l0's tape");
}

/// The source lies where the read placed it, and the tape where the parser wrote it: both in the document's tape
/// block, frozen, each a blob named by the sha256 of its bytes where they lie.
#[test]
fn the_source_and_the_tape_are_blobs_in_the_documents_frozen_tape_block() {
    use sha2::{Digest as _, Sha256};
    let (_, mut files) = root("room");
    let (mut blobs, mut dir) = (Blobs::default(), Directory::default());
    let d = files.answer("bootstrap-l0.rerail", &mut blobs, &mut dir).unwrap();
    let (s, t) = (blobs.get(&d.after.source).unwrap(), blobs.get(&d.after.tape).unwrap());
    assert_eq!(s.block, t.block);
    assert!(dir.is_frozen(t.block), "a blob is never versioned");
    assert_eq!(dir.block(t.block).unwrap().header().kind, kind::TAPE);
    let l = dir.read("t", &[t.block]).unwrap();
    let b = l.block(0);
    assert_eq!(&b[s.at..s.at + s.len], crate::L0.as_bytes(), "the source, read where it lies in the block");
    assert_eq!(d.after.source.0, <[u8; 32]>::from(Sha256::digest(crate::L0.as_bytes())));
    assert_eq!(d.after.tape.0, <[u8; 32]>::from(Sha256::digest(&b[t.at..t.at + t.len])));
    // The same bytes housed again are the first blob, and their block is let go.
    let again = doc_of(&mut dir, &mut blobs, crate::L0).unwrap();
    assert_eq!(again, d.after);
    assert_eq!(blobs.len(), 2);
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
    assert_eq!(blobs.len(), 2, "its source and its tape");
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
    let s = blobs.get(&edited.after.source).unwrap();
    let l = dir.read("t", &[s.block]).unwrap();
    assert!(std::str::from_utf8(&l.block(0)[s.at..s.at + s.len]).unwrap().contains("b4_added"));
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
