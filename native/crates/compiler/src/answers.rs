//! A grammar's prover answers kept across processes (decision 0414, Q234): every compile of the
//! grammar on the machine starts from the answers the compiles before it made, and adds its own.
//!
//! A grammar family's answers are one file, `<family>.answers` ([`bitlex_prove::encode`]'s bytes),
//! in the machine's answers directory. Its answers are by question, so they are never wrong for
//! another text of the grammar (an edit) or another compiler: a question nobody asked before is
//! proved, and kept; an answer nobody asks again is only bytes.
//!
//! The work is done once a machine. `<family>.done` names the grammar text whose questions the file
//! answers all of. A compile of that text reads the file and takes no lock. A compile of another
//! text takes the family's lock first (`<family>.lock`), and so does each one after it, until the
//! first has written its answers and the mark: the others then find the mark and compile from the
//! file, side by side. Tests that start together on an edited grammar prove its new questions once,
//! not once a process.
use crate::bitlex_prove::{self, Answer, Key, Known};
use std::path::{Path, PathBuf};

/// The machine's answers directory: beside the test stores' templates, in the system's temporary
/// directory, since what it holds is made again when it is gone.
pub fn directory() -> PathBuf {
    std::env::temp_dir().join("rerail-prover-answers")
}

/// What `compile` made of the grammar `text` of `family`, given `seed` (answers shipped with the
/// grammar, if any) and the answers kept in [`directory`]; the answers it made are kept there for the
/// next compile. `compile` returns its result and the answers it made.
pub fn answered<T>(family: &str, text: &[u8], seed: Known, compile: impl FnMut(Known) -> Result<(T, Vec<(Key, Answer)>), String>) -> Result<T, String> {
    answered_in(&directory(), family, text, seed, compile)
}

/// [`answered`], with the answers in `dir`.
pub fn answered_in<T>(dir: &Path, family: &str, text: &[u8], seed: Known, mut compile: impl FnMut(Known) -> Result<(T, Vec<(Key, Answer)>), String>) -> Result<T, String> {
    let Ok(()) = std::fs::create_dir_all(dir) else {
        // No directory: the compile alone, from the seed.
        return compile(seed).map(|(t, _)| t);
    };
    let files = Files::of(dir, family);
    let mark = mark_of(text);
    let done = || std::fs::read(&files.done).is_ok_and(|m| m == mark);
    let known = |seed: &Known| {
        let mut k = seed.clone();
        k.extend(bitlex_prove::decode(&std::fs::read(&files.answers).unwrap_or_default()));
        k
    };
    if !done() {
        let lock = files.lock();
        // A compile before took the lock and finished: its answers are this one's.
        if !done() {
            let (t, made) = compile(known(&seed))?;
            files.keep(&made);
            let _ = write_whole(&files.done, &mark);
            drop(lock);
            return Ok(t);
        }
        drop(lock);
    }
    let (t, made) = compile(known(&seed))?;
    if !made.is_empty() {
        // The file answered nearly everything (another compiler's questions, say): the rest kept too.
        let _lock = files.lock();
        files.keep(&made);
    }
    Ok(t)
}

/// What `compile` made of the grammar `text` of `family`, from the answers shipped with it
/// (`shipped`, `<grammar>.answers`' bytes, decision 0669) and the machine's ([`answered`]).
pub fn answered_shipped<T>(family: &str, text: &[u8], shipped: &[u8], compile: impl FnMut(Known) -> Result<(T, Vec<(Key, Answer)>), String>) -> Result<T, String> {
    answered(family, text, bitlex_prove::decode(shipped), compile)
}

/// The test of a grammar's shipped answers (decision 0669; `<grammar>.answers` at `path`, `shipped`
/// its bytes as built): `compile`, given them alone and not the machine's kept answers, proves
/// nothing. `compile` compiles each of the grammar's variants (C and C++, say) from the answers it is
/// given, as its callers do, and returns every answer they made.
///
/// With `RERAIL_BLESS_ANSWERS=1` it writes the file again instead, from `compile` given none. The
/// budget must be lifted too (`RERAIL_PROVING_STEPS`): answers a budget cut short would ship "not
/// proved" for questions a compile can prove, and every compile from them would keep that.
pub fn check_shipped(path: &Path, shipped: &[u8], compile: impl Fn(Known) -> Result<Vec<(Key, Answer)>, String>) {
    let name = path.display();
    if std::env::var_os("RERAIL_BLESS_ANSWERS").is_some() {
        assert!(std::env::var_os("RERAIL_PROVING_STEPS").is_some(), "{name}: blessing asks for the budget lifted too (RERAIL_PROVING_STEPS=1000000000000)");
        let made = compile(Known::default()).unwrap_or_else(|e| panic!("{name}: the grammar compiles: {e}"));
        std::fs::write(path, bitlex_prove::encode(&made)).unwrap_or_else(|e| panic!("{name}: written: {e}"));
        return;
    }
    let made = compile(bitlex_prove::decode(shipped)).unwrap_or_else(|e| panic!("{name}: the grammar compiles: {e}"));
    assert!(
        made.is_empty(),
        "{name}: {} questions the shipped answers lack (the grammar or the provers changed): RERAIL_BLESS_ANSWERS=1 RERAIL_PROVING_STEPS=1000000000000 writes it again",
        made.len()
    );
}

/// A family's files in the answers directory.
struct Files {
    answers: PathBuf,
    done: PathBuf,
    lock: PathBuf,
}

impl Files {
    fn of(dir: &Path, family: &str) -> Files {
        Files { answers: dir.join(format!("{family}.answers")), done: dir.join(format!("{family}.done")), lock: dir.join(format!("{family}.lock")) }
    }

    /// The family's lock, held until dropped; `None` where the file system has none (the compile then
    /// goes on unlocked: its answers are the same either way).
    fn lock(&self) -> Option<std::fs::File> {
        let f = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(&self.lock).ok()?;
        f.lock().ok()?;
        Some(f)
    }

    /// `made` added to the kept answers, the file written whole (a reader sees the old or the new).
    /// Under the family's lock.
    fn keep(&self, made: &[(Key, Answer)]) {
        if made.is_empty() {
            return;
        }
        let mut all: Vec<(Key, Answer)> = bitlex_prove::decode(&std::fs::read(&self.answers).unwrap_or_default()).into_iter().collect();
        all.extend_from_slice(made);
        let _ = write_whole(&self.answers, &bitlex_prove::encode(&all));
    }
}

/// The mark of a grammar text whose questions the answers file holds all of: the text's digest under
/// the provers' version (a new version asks new questions).
fn mark_of(text: &[u8]) -> Vec<u8> {
    let mut t = format!("v{} ", bitlex_prove::PROOFS_VERSION).into_bytes();
    t.extend_from_slice(text);
    bitlex_prove::key_of(&t).to_vec()
}

/// `bytes` written to `path` through a file of their own beside it, renamed into place.
fn write_whole(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn key(n: u8) -> Key {
        [n; 16]
    }

    /// A compile that asks `asks` questions and proves the ones it is not given (counting its proofs).
    fn compile_asking<'a>(asks: &'a [u8], proved: &'a AtomicUsize) -> impl FnMut(Known) -> Result<((), Vec<(Key, Answer)>), String> + 'a {
        move |known: Known| {
            let made: Vec<(Key, Answer)> = asks.iter().filter(|&&q| !known.contains_key(&key(q))).map(|&q| (key(q), Answer::Never(q % 2 == 0))).collect();
            proved.fetch_add(made.len(), Ordering::Relaxed);
            Ok(((), made))
        }
    }

    /// A text's answers are proved once a machine: a second compile of it proves nothing; an edited
    /// text proves only its new questions; the seed's answers are never proved.
    #[test]
    fn a_grammars_answers_are_proved_once_a_machine() {
        let dir = std::env::temp_dir().join(format!("rerail-answers-test-{}-a", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let proved = AtomicUsize::new(0);
        answered_in(&dir, "g", b"one", Known::default(), compile_asking(&[1, 2, 3], &proved)).unwrap();
        assert_eq!(proved.load(Ordering::Relaxed), 3);
        answered_in(&dir, "g", b"one", Known::default(), compile_asking(&[1, 2, 3], &proved)).unwrap();
        assert_eq!(proved.load(Ordering::Relaxed), 3, "the same text proved again");
        answered_in(&dir, "g", b"two", Known::default(), compile_asking(&[2, 3, 4], &proved)).unwrap();
        assert_eq!(proved.load(Ordering::Relaxed), 4, "an edit proves its new question only");
        let seed: Known = [(key(9), Answer::Never(true))].into_iter().collect();
        answered_in(&dir, "g", b"three", seed, compile_asking(&[4, 9], &proved)).unwrap();
        assert_eq!(proved.load(Ordering::Relaxed), 4, "the seed's answer proved");
        let kept = bitlex_prove::decode(&std::fs::read(dir.join("g.answers")).unwrap());
        assert_eq!(kept.get(&key(4)), Some(&Answer::Never(true)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Compiles that start together on a text no compile has answered prove its questions once: the
    /// first holds the family's lock while it proves, and the others then compile from its answers.
    #[test]
    fn compiles_that_start_together_prove_once() {
        let dir = std::env::temp_dir().join(format!("rerail-answers-test-{}-b", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let proved = AtomicUsize::new(0);
        std::thread::scope(|s| {
            for _ in 0..8 {
                s.spawn(|| {
                    let mut inner = compile_asking(&[1, 2, 3, 4, 5], &proved);
                    answered_in(&dir, "g", b"text", Known::default(), |k| {
                        let r = inner(k);
                        // Long enough that the others reach the lock while this one holds it.
                        std::thread::sleep(std::time::Duration::from_millis(50));
                        r
                    })
                    .unwrap();
                });
            }
        });
        assert_eq!(proved.load(Ordering::Relaxed), 5, "the questions proved more than once");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
