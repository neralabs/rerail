//! B4's checks: l0's tape through the file layer.

use super::*;
use std::time::{Duration, SystemTime};

/// A root of its own for test `name`, holding a copy of l0, its stat older than the racy window (a file written in
/// the window's tick is read again, as git's racy rule has it), and the store beside it: the root and the store.
pub(crate) fn root(name: &str) -> (PathBuf, Store) {
    let base = std::env::temp_dir().join(format!("rerail-b4-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let dir = base.join("root");
    std::fs::create_dir_all(&dir).unwrap();
    write(&dir.join("bootstrap-l0.rerail"), crate::L0, 60);
    (dir, Store::open(&base.join("store")).unwrap())
}

/// The file layer over a root: the world, and a registry, blobs and directory of the run's memory; each path's last
/// answer (the document set's, for these checks of the layer below it).
pub(crate) struct W {
    pub root: PathBuf,
    pub world: World,
    pub reg: crate::op::Registry,
    pub answers: crate::answer::AnswerOps,
    pub traces: crate::trace::TraceOps,
    pub blobs: BlobRows,
    pub dir: Directory,
    last: std::collections::BTreeMap<String, DocIds>,
}

impl W {
    pub(crate) fn new(name: &str) -> W {
        let (root, store) = root(name);
        W::over(root, store, Directory::default())
    }

    pub(crate) fn over(root: PathBuf, store: Store, dir: Directory) -> W {
        let mut reg = crate::op::Registry::default();
        let traces = crate::trace::TraceOps::declare(&mut reg).unwrap();
        let answers = crate::answer::AnswerOps::declare(&mut reg).unwrap();
        let world = World::new(root.clone(), store, answers, &dir, [None; 3]).unwrap();
        W { root, world, reg, answers, blobs: BlobRows::new(&traces), traces, dir, last: Default::default() }
    }

    /// The load answer for `path`, from its last one.
    pub(crate) fn answer(&mut self, path: &str) -> Result<Doc, String> {
        let d = key_of(path, |_| false);
        let module = path.strip_suffix(".rerail").unwrap_or(path);
        let before = self.last.get(path).copied();
        let mut leaves = self.world.leaves_w(&mut self.dir);
        let doc = self.world.load(&self.reg, &self.traces, &mut self.dir, &mut self.blobs, &mut leaves, d, path, module, before);
        self.world.commit_leaves(&self.reg, &self.traces, &mut self.dir, leaves)?;
        let doc = doc?;
        self.last.insert(path.to_string(), doc.after);
        Ok(doc)
    }
}

/// The racy window these tests run under (`RERAIL_RACY_MS`), as the engine's tests narrow it (`TEST_RACY_MS`): a file
/// written (its ctime now) is trusted by its stat once twice the window has passed.
pub(crate) fn racy_window() -> rerail_context::Bound {
    rerail_context::bind(rerail_context::Switches(Arc::new(vec![("RERAIL_RACY_MS".into(), "100".into())])))
}

/// A registry with the trace's operations, no blob, and a directory (a document in hand's: no world).
fn fresh() -> (Reg, BlobRows, Directory) {
    let mut reg = crate::op::Registry::default();
    let traces = crate::trace::TraceOps::declare(&mut reg).unwrap();
    let answers = crate::answer::AnswerOps::declare(&mut reg).unwrap();
    (Reg(reg, answers), BlobRows::new(&traces), Directory::default())
}

/// A registry and Δanswer's operations in it.
pub(crate) struct Reg(pub crate::op::Registry, pub crate::answer::AnswerOps);

impl std::ops::Deref for Reg {
    type Target = crate::op::Registry;
    fn deref(&self) -> &crate::op::Registry {
        &self.0
    }
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
    let mut w = W::new("cold");
    let d = w.answer("bootstrap-l0.rerail").unwrap();
    assert_eq!((d.path.as_str(), d.module.as_str(), d.before), ("bootstrap-l0.rerail", "bootstrap-l0", None));
    let (s, t) = (w.blobs.get(&w.dir, &d.after.source).unwrap().unwrap(), w.blobs.get(&w.dir, &d.after.tape).unwrap().unwrap());
    let l = w.dir.read("t", &[t.block]).unwrap();
    assert_eq!(&l.block(0)[s.at..s.at + s.len], crate::L0.as_bytes());
    assert!(reads(l.block(0), s, l.block(0), t).len() > 100, "l0's tape");
}

/// The source lies where the read placed it, and the tape where the parser wrote it: both rooms of a slab, sealed,
/// each a blob named by the sha256 of its bytes where they lie.
#[test]
fn the_source_and_the_tape_are_blobs_in_sealed_rooms_of_a_slab() {
    use sha2::{Digest as _, Sha256};
    let mut w = W::new("room");
    let d = w.answer("bootstrap-l0.rerail").unwrap();
    let W { reg, answers, mut blobs, mut dir, .. } = w;
    let reg = Reg(reg, answers);
    let (s, t) = (blobs.get(&dir, &d.after.source).unwrap().unwrap(), blobs.get(&dir, &d.after.tape).unwrap().unwrap());
    assert_eq!(s.block, t.block);
    assert!(dir.is_fixed(s.block, s.at + s.len) && dir.is_fixed(t.block, t.at + t.len), "a blob is never versioned");
    assert_eq!(dir.block(t.block).unwrap().header().kind, kind::TAPE);
    {
        let l = dir.read("t", &[t.block]).unwrap();
        let b = l.block(0);
        assert_eq!(&b[s.at..s.at + s.len], crate::L0.as_bytes(), "the source, read where it lies in the block");
        assert_eq!(d.after.source.0, <[u8; 32]>::from(Sha256::digest(crate::L0.as_bytes())));
        assert_eq!(d.after.tape.0, <[u8; 32]>::from(Sha256::digest(&b[t.at..t.at + t.len])));
    }
    // The same bytes housed again are the first blob.
    let again = doc_of(&reg, &reg.1, &mut dir, &mut blobs, crate::L0).unwrap();
    assert_eq!(again, d.after);
    assert_eq!(blobs.len(&dir).unwrap(), 2);
}

/// Two sources of the same shape have the same tape: the second's tape blob is the first's, where it lies, and its
/// source a new blob in a room of its own. Both are read through `reads`, from their two places.
#[test]
fn a_new_source_may_share_an_earlier_tape() {
    let (reg, mut blobs, mut dir) = fresh();
    let a = doc_of(&reg, &reg.1, &mut dir, &mut blobs, "e(1, 2).\n").unwrap();
    let b = doc_of(&reg, &reg.1, &mut dir, &mut blobs, "e(3, 4).\n").unwrap();
    assert_ne!(a.source, b.source);
    assert_eq!(a.tape, b.tape, "the tape holds the shape, not the text");
    let (s, t) = (blobs.get(&dir, &b.source).unwrap().unwrap(), blobs.get(&dir, &b.tape).unwrap().unwrap());
    assert!(s.at > t.at, "the second source's room, past the first document's tape");
    assert!(dir.is_fixed(s.block, s.at + s.len));
    let l = dir.read("t", &[s.block, t.block]).unwrap();
    let r = reads(l.block(0), s, l.block(1), t);
    assert_eq!(r.text(r.len() - 1), "4");
}

#[test]
fn an_unchanged_file_gives_the_identity_with_no_read_past_its_stat() {
    let _racy = racy_window();
    let mut w = W::new("same");
    past_racy_window();
    let first = w.answer("bootstrap-l0.rerail").unwrap();
    let (c0, asked) = (w.world.counts(), w.world.asked);
    let again = w.answer("bootstrap-l0.rerail").unwrap();
    assert!(again.is_identity(), "{again:?}");
    assert_eq!(again.after, first.after);
    let c = w.world.counts().since(&c0);
    // The observer hashes one trusted file in 64 again (`VALIDATE_EVERY`, a slice by path and the second): the only
    // bytes read past the stat are that check's, and it finds them unchanged.
    assert_eq!(c.hashed, c.validated, "no byte read but the observer's sampled check: {c:?}");
    assert_eq!((c.reused + c.validated, c.mismatched), (1, 0), "the answer trusted on its stat: {c:?}");
    assert_eq!(w.world.asked, asked, "nothing read, loaded or parsed");
    assert_eq!(w.blobs.len(&w.dir).unwrap(), 2, "its source and its tape");
}

#[test]
fn an_edited_file_is_read_once_and_placed_once() {
    let mut w = W::new("edit");
    let first = w.answer("bootstrap-l0.rerail").unwrap();
    write(&w.root.join("bootstrap-l0.rerail"), &format!("{}\nb4_added(x) :- rule(x).\n", crate::L0), 30);
    let (c0, asked) = (w.world.counts(), w.world.asked);
    let edited = w.answer("bootstrap-l0.rerail").unwrap();
    assert_eq!(edited.before, Some(first.after));
    assert_ne!(edited.after, first.after);
    assert_eq!((w.world.asked.reads, w.world.asked.parses), (asked.reads + 1, asked.parses + 1), "read once, placed once (read_placed places what it reads), parsed once");
    assert_eq!(w.world.counts().since(&c0).hashed, 1);
    let s = w.blobs.get(&w.dir, &edited.after.source).unwrap().unwrap();
    let l = w.dir.read("t", &[s.block]).unwrap();
    assert!(std::str::from_utf8(&l.block(0)[s.at..s.at + s.len]).unwrap().contains("b4_added"));
}

#[test]
fn a_file_that_does_not_parse_is_refused_naming_its_path() {
    let mut w = W::new("bad");
    write(&w.root.join("broken.rerail"), "this is not a rule (\n", 30);
    let e = w.answer("broken.rerail").unwrap_err();
    assert!(e.starts_with("broken.rerail: "), "{e}");
    let e = w.answer("missing.rerail").unwrap_err();
    assert!(e.starts_with("missing.rerail: "), "{e}");
}

/// Documents share a slab, as the residency's do (`rerail_slabs`): each one's source and tape are rooms of it, sealed
/// as they are written, read in place by a launch and by a reference while the slab takes more; a document past the
/// slab's room starts the next.
#[test]
fn documents_share_a_slab_their_rooms_sealed_as_written() {
    let (reg, mut blobs, mut dir) = fresh();
    let a = doc_of(&reg, &reg.1, &mut dir, &mut blobs, "e(1, 2).\n").unwrap();
    let b = doc_of(&reg, &reg.1, &mut dir, &mut blobs, "p(x) :- e(x, _).\n").unwrap();
    let place = |dir: &Directory, blobs: &BlobRows, id| blobs.get(dir, id).unwrap().unwrap();
    let (at, bt) = (place(&dir, &blobs, &a.tape), place(&dir, &blobs, &b.tape));
    assert_eq!(at.block, bt.block, "one slab");
    let slab = dir.block(at.block).unwrap();
    assert!(slab.is_writable() && slab.reserved() >= crate::dir::SLAB_BYTES, "an open slab, taking more");
    assert!(dir.is_fixed(at.block, bt.at + bt.len) && !dir.is_fixed(at.block, slab.used() + 1), "sealed to what is written");
    // A block may refer to a sealed room while the slab takes more, and a launch reads it there.
    let r = dir.make(kind::CONTROL, 8);
    dir.refer(r, at.block, at.at).unwrap();
    let c = doc_of(&reg, &reg.1, &mut dir, &mut blobs, "q(y) :- p(y).\n").unwrap();
    let ct = place(&dir, &blobs, &c.tape);
    assert_eq!(ct.block, at.block);
    {
        let l = dir.read("t", &[r]).unwrap();
        assert_eq!(l.table().block(at.block.0).map(|x| x.len() > bt.at), Some(true), "the slab, reached by reference");
    }
    // A source past the slab's room: a slab of its own size.
    let big = "e(1, 2).\n".repeat(crate::dir::SLAB_BYTES / 81 / 9 + 1);
    let d = doc_of(&reg, &reg.1, &mut dir, &mut blobs, &big).unwrap();
    assert_ne!(place(&dir, &blobs, &d.tape).block, at.block, "the next slab");
}

/// The leaves (P§11.4: path, digest, hit or miss): each leaf read is recorded, a miss too (D3), and a leaf's row
/// changes only when what its path holds does.
#[test]
fn every_leaf_read_is_recorded_misses_too() {
    let mut w = W::new("leaves");
    w.answer("bootstrap-l0.rerail").unwrap();
    w.answer("missing.rerail").unwrap_err();
    let (l0, gone) = (key_of("bootstrap-l0.rerail", |_| false), key_of("missing.rerail", |_| false));
    let hit = probe(&w.dir, &w.world.leaves, &[l0]).unwrap().expect("l0's leaf");
    assert_eq!(BlobId::of_words(&hit[1..5]), BlobId::of(crate::L0.as_bytes()));
    assert_eq!(hit[5], 1, "found");
    let miss = probe(&w.dir, &w.world.leaves, &[gone]).unwrap().expect("the missing file's leaf");
    assert_eq!((&miss[1..5], miss[5]), (&[0i64; 4][..], 0), "a miss, recorded");
    let head = w.world.leaves.head();
    w.answer("missing.rerail").unwrap_err();
    w.answer("bootstrap-l0.rerail").unwrap();
    assert_eq!(w.world.leaves.head(), head, "an unchanged leaf writes nothing");
    write(&w.root.join("missing.rerail"), "e(1, 2).\n", 30);
    w.answer("missing.rerail").unwrap();
    assert_eq!(probe(&w.dir, &w.world.leaves, &[gone]).unwrap().unwrap()[5], 1, "found now");
}

/// The three answerers (P§12 item 7), one key each, through `Answerer::answer`. Load (a world reader) gives a path's
/// source, or a miss, and writes both leaves into the run's one layer of the leaves (D3: misses too). Parse (pure)
/// gives the tape of a source a home holds, the same at a second ask and with no second parse, and a miss for a
/// content no home holds. Intern (pure) gives the same symbols blob at every ask of its key.
#[test]
fn each_answerer_answers_one_key_its_leaves_one_layer_a_run() {
    let mut w = W::new("answerers");
    let iops = crate::intern::InternOps::declare(&mut w.reg).unwrap();
    let atoms = crate::sym::Atoms::new(&mut w.dir);
    let (l0, gone) = (key_of("bootstrap-l0.rerail", |_| false), key_of("missing.rerail", |_| false));
    let named = [
        Load1 { d: l0, path: "bootstrap-l0.rerail".into(), module: "bootstrap-l0".into(), before: None },
        Load1 { d: gone, path: "missing.rerail".into(), module: "missing".into(), before: None },
    ];
    assert!(matches!((Load::KIND, Parse::KIND, Intern::KIND), (AnswerKind::World, AnswerKind::Pure, AnswerKind::Pure)));
    let mut leaves = w.world.leaves_w(&mut w.dir);
    let head = w.world.leaves.head();
    let W { world, reg, traces, dir, blobs, .. } = &mut w;
    macro_rules! at {
        () => {
            Reach { world: &mut *world, reg, ops: traces, dir: &mut *dir, blobs: &mut *blobs }
        };
    }
    let source = match (Load { at: at!(), named: &named }).answer(&[l0], &mut leaves).unwrap() {
        Answer::Blob(s) => s,
        a => panic!("{a:?}"),
    };
    assert_eq!(source, BlobId::of(crate::L0.as_bytes()));
    assert_eq!((Load { at: at!(), named: &named }).answer(&[gone], &mut leaves).unwrap(), Answer::Miss);
    assert_eq!(world.leaves.head(), head, "the leaves wait for the run's end");
    let parses = world.asked.parses;
    let tape = (Parse { at: at!() }).answer(&source.words(), &mut leaves).unwrap();
    assert!(matches!(tape, Answer::Blob(_)));
    assert_eq!((Parse { at: at!() }).answer(&source.words(), &mut leaves).unwrap(), tape, "pure: the same tape");
    assert_eq!(world.asked.parses, parses, "a content Load answered is not parsed again");
    assert_eq!((Parse { at: at!() }).answer(&BlobId::of(b"no home holds this").words(), &mut leaves).unwrap(), Answer::Miss);
    let Answer::Blob(t) = tape else { unreachable!() };
    let key: Vec<i64> = std::iter::once(l0).chain(source.words()).chain(t.words()).collect();
    let syms = (Intern { at: at!(), iops: &iops, atoms, named: &named }).answer(&key, &mut leaves).unwrap();
    assert!(matches!(syms, Answer::Blob(_)));
    assert_eq!((Intern { at: at!(), iops: &iops, atoms, named: &named }).answer(&key, &mut leaves).unwrap(), syms, "pure: by its key");
    world.commit_leaves(reg, traces, dir, leaves).unwrap();
    let layers = |w: &W| {
        let l = w.dir.read("t", &[w.world.leaves.head().unwrap()]).unwrap();
        rerail_blocks::layer::Layer::new(l.block(0)).unwrap().below().is_some()
    };
    assert!(!layers(&w), "the run's leaves are one layer: {:?}", w.world.leaves.head());
    assert_eq!(probe(&w.dir, &w.world.leaves, &[gone]).unwrap().expect("the miss's leaf")[5], 0);
    assert_eq!(probe(&w.dir, &w.world.leaves, &[l0]).unwrap().expect("l0's leaf")[5], 1);
}

/// The parse answer is pure, keyed by (content, grammar) (*Operations* §5, §6.2: "a hit skips the read and the
/// parse"): a content answered before is not parsed again, at another path, nor once the residency has let its slab
/// go (it is loaded from the store's objects, where it lies), nor in a later process over the same store.
#[test]
fn a_content_answered_before_is_never_parsed_again() {
    let mut w = W::new("hit");
    let a = w.answer("bootstrap-l0.rerail").unwrap();
    assert_eq!((w.world.asked.parses, w.world.asked.loads), (1, 0));
    // The same content at another path: resident, so not parsed (its file is read: a new path's leaf).
    write(&w.root.join("copy.rerail"), crate::L0, 30);
    let b = w.answer("copy.rerail").unwrap();
    assert_eq!(b.after, a.after);
    assert_eq!((w.world.asked.parses, w.world.asked.loads), (1, 0), "a resident hit");
    // Its slab dropped (the residency's drop after an execution): loaded from the store's objects, not parsed.
    w.dir = std::mem::take(&mut w.dir).with_residency(1 << 40, Duration::from_millis(20));
    w.dir.executed();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(w.dir.residency().1, 1, "dropped");
    assert!(w.blobs.get(&w.dir, &a.after.tape).unwrap().is_none(), "no longer resident");
    write(&w.root.join("third.rerail"), crate::L0, 30);
    let c = w.answer("third.rerail").unwrap();
    assert_eq!(c.after, a.after);
    assert_eq!((w.world.asked.parses, w.world.asked.loads), (1, 1), "loaded where the store's segments hold it");
    let (s, t) = (w.blobs.get(&w.dir, &c.after.source).unwrap().unwrap(), w.blobs.get(&w.dir, &c.after.tape).unwrap().unwrap());
    {
        let l = w.dir.read("t", &[s.block, t.block]).unwrap();
        assert_eq!(&l.block(0)[s.at..s.at + s.len], crate::L0.as_bytes());
        assert!(reads(l.block(0), s, l.block(1), t).len() > 100, "the tape, read where it was loaded");
    }
    // A later process over the same store: nothing resident, nothing parsed.
    let W { root, world, .. } = w;
    let store = Store::open(&world.store.folder).unwrap();
    drop(world);
    let mut later = W::over(root, store, Directory::default());
    let d = later.answer("bootstrap-l0.rerail").unwrap();
    assert_eq!(d.after, a.after);
    assert_eq!((later.world.asked.parses, later.world.asked.loads), (0, 1));
}

/// The residency's policy (`rerail_slabs`, as it is): a slab is retired only while what is held and a new one pass the
/// budget, and then the least recently used; a slab a document was asked for since is kept over an older one, and one
/// a reader holds is not retired.
#[test]
fn the_residency_retires_the_least_recently_used_slab_only_past_its_budget() {
    let (reg, mut blobs, dir) = fresh();
    // Slabs of no more than a document's need, each source 1000 bytes longer than the last: its room is past what the
    // last one's slab has left, so each lies in a slab of its own.
    let mut dir = dir.with_slab_bytes(1);
    let src = |k: usize| format!("e({k}, 2).\n// {}\n", "x".repeat(1000 * (k + 1)));
    let docs: Vec<DocIds> = (0..3).map(|k| doc_of(&reg, &reg.1, &mut dir, &mut blobs, &src(k)).unwrap()).collect();
    assert_eq!((dir.slabs().len(), dir.residency().0), (3, 0), "within the budget: nothing retired");
    // The budget: the three held and the fourth's slab, less a byte.
    let held: usize = dir.slabs().iter().map(|&b| dir.block(b).unwrap().reserved()).sum();
    let fourth = dir.slab_room_bytes(room(src(3).len()));
    let mut dir = dir.with_residency(held + fourth - 1, Duration::from_secs(3600));
    let slab = |dir: &Directory, blobs: &BlobRows, d: &DocIds| blobs.get(dir, &d.tape).unwrap().map(|p| p.block);
    // The first document asked for again: the second is the least recently used.
    dir.touch(slab(&dir, &blobs, &docs[0]).unwrap());
    let d = doc_of(&reg, &reg.1, &mut dir, &mut blobs, &src(3)).unwrap();
    assert_eq!(dir.residency().0, 1, "one retired, to make room");
    assert!(slab(&dir, &blobs, &docs[1]).is_none(), "the least recently used");
    assert!(slab(&dir, &blobs, &docs[0]).is_some() && slab(&dir, &blobs, &docs[2]).is_some() && slab(&dir, &blobs, &d).is_some());
    // A slab a reader holds is not retired: the third's, the least recently used now, held; the first goes.
    let held = dir.block(slab(&dir, &blobs, &docs[2]).unwrap()).unwrap();
    dir.touch(slab(&dir, &blobs, &docs[0]).unwrap());
    dir.touch(slab(&dir, &blobs, &d).unwrap());
    dir.touch(slab(&dir, &blobs, &docs[0]).unwrap());
    doc_of(&reg, &reg.1, &mut dir, &mut blobs, &src(4)).unwrap();
    assert!(slab(&dir, &blobs, &docs[2]).is_some(), "held by a reader");
    assert!(dir.residency().0 >= 2 && slab(&dir, &blobs, &d).is_none(), "the least recently used of those no reader holds");
    drop(held);
}
