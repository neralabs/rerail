//! A run's source texts, read once (decision 0300).
//!
//! A cold run over a tree read each source file three times: the observer's digest before phase 1
//! (`world::bytes_digests`), phase 1's builder, and the outer run's builder (its memo lookup before the first wave,
//! then its wave). The digest's read is now the run's one read: the run keeps what it read (`Sources`), hands it to
//! phase 1's run (`Texts::sources`), whose builder borrows each text, and its own builder takes it. A text is used
//! only while its file's stat is the one taken before it was read: a file changed since is read again.

use crate::code_rule_dsl::tape::Text;
use crate::content_store::FileStat;
use crate::code_rule_dsl::tape::Home;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The texts one run read, by the path it read each at: the run's, gone with it.
#[derive(Default)]
/// Each is held where it was read into (`placed::placer`: a slab's room where a device reads sources), and every
/// reader shares it there (decision 0353 step 7).
pub struct Sources(parking_lot::Mutex<HashMap<PathBuf, (Option<FileStat>, Arc<Text>)>>);

impl Sources {
    /// Keeps the text read at `path` while the file's stat is `stat`. Bytes that are not UTF-8 are no text: a
    /// builder reads and refuses them itself.
    pub fn keep(&self, path: &Path, stat: FileStat, bytes: Home) {
        let Ok(text) = Text::new(bytes) else { return };
        self.0.lock().insert(path.to_path_buf(), (Some(stat), Arc::new(text)));
    }

    /// A Git source's path includes its immutable object identity, so it needs no filesystem stat. Several paths of
    /// one blob share its text.
    pub fn keep_git(&self, path: &Path, text: &Arc<Text>) {
        self.0.lock().insert(path.to_path_buf(), (None, Arc::clone(text)));
    }

    /// The text kept for `path`, if its file is as it was read (else it is dropped): taken, by its last reader,
    /// or shared.
    fn text(&self, path: &Path, take: bool) -> Option<Arc<Text>> {
        let mut all = self.0.lock();
        let (stat, _) = all.get(path)?;
        if stat.is_some() && crate::git_workspace::snapshot::file_stat(path) != *stat {
            all.remove(path);
            return None;
        }
        if take { all.remove(path).map(|(_, t)| t) } else { all.get(path).map(|(_, t)| t.clone()) }
    }

    /// Drops what no builder took (the run's waves are over): how many texts, and their bytes.
    pub fn release(&self) -> (usize, usize) {
        let all = std::mem::take(&mut *self.0.lock());
        (all.len(), all.values().map(|(_, t)| t.len()).sum())
    }
}

/// A builder's texts: its run's own, which it takes (their last reader), and those the run inside which it runs
/// handed it (phase 1's: the outer run's), which it borrows.
#[derive(Clone, Copy)]
pub struct Given<'a> {
    pub own: &'a Sources,
    pub handed: Option<&'a Sources>,
}

impl Given<'_> {
    /// `path`'s text where the run holds it as the file is now; else `None`, and the caller reads the file.
    /// `take`: the caller is its last reader (its wave), not a look before it.
    pub fn text(&self, path: &Path, take: bool) -> Option<Arc<Text>> {
        self.own.text(path, take).or_else(|| self.handed?.text(path, false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_text_is_borrowed_then_taken_and_read_again_once_its_file_changed() {
        let dir = std::env::temp_dir().join(format!("rerail-texts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("a.py"), dir.join("b.py"));
        std::fs::write(&a, "x = 1\n").unwrap();
        std::fs::write(&b, "y = 2\n").unwrap();
        let stat = |p: &Path| FileStat::of(&std::fs::metadata(p).unwrap());
        let outer = Sources::default();
        outer.keep(&a, stat(&a), Home::from(rerail_tape_format::Vector::from(&b"x = 1\n"[..])));
        outer.keep(&b, stat(&b), Home::from(rerail_tape_format::Vector::from(&b"y = 2\n"[..])));
        outer.keep(&dir.join("c.bin"), stat(&a), Home::from(rerail_tape_format::Vector::from(&[0xffu8, 0xfe][..])));
        let inner_own = Sources::default();
        let inner = Given { own: &inner_own, handed: Some(&outer) };
        let mine = Given { own: &outer, handed: None };
        assert_eq!(inner.text(&a, true).as_deref().map(Text::as_str), Some("x = 1\n"), "a run handed the texts borrows");
        assert_eq!(mine.text(&a, false).as_deref().map(Text::as_str), Some("x = 1\n"), "a look before the wave leaves it");
        assert_eq!(mine.text(&a, true).as_deref().map(Text::as_str), Some("x = 1\n"), "the run's wave takes it");
        assert_eq!(mine.text(&a, true), None, "taken");
        assert_eq!(inner.text(&dir.join("c.bin"), false), None, "not UTF-8: not kept");
        // Changed since it was read: not used, and dropped.
        std::thread::sleep(std::time::Duration::from_millis(5));
        std::fs::write(&b, "y = 3\n").unwrap();
        assert_eq!(mine.text(&b, true), None);
        std::fs::write(&b, "y = 2\n").unwrap();
        outer.keep(&b, stat(&b), Home::from(rerail_tape_format::Vector::from(&b"y = 2\n"[..])));
        assert_eq!(outer.release(), (1, 6), "what no wave took");
        assert_eq!(inner.text(&b, false), None, "released");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
