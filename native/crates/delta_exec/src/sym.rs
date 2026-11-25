//! The symbol table: every string a program names (a relation, a variable, a kind, a string constant) as an id, in
//! one atoms block (decision 0401). It is the one program global every level shares (decision 0686): K-form rows and
//! facts hold ids, and an id names the same string at every level.

use rerail_blocks::atoms::{AtomRead, AtomsW};
use std::sync::Mutex;

/// Strings to ids and back, in an atoms block. Ids start at 1, so 0 is never a symbol.
pub struct Symbols {
    atoms: Mutex<AtomsW>,
}

impl Default for Symbols {
    fn default() -> Self {
        Self::new()
    }
}

impl Symbols {
    pub fn new() -> Symbols {
        Symbols {
            atoms: Mutex::new(AtomsW::anon(1, 1 << 20, 64 << 20)),
        }
    }

    /// The id of `s`, made when it has none.
    pub fn intern(&self, s: &str) -> i64 {
        let h = rerail_blocks::maps::hash_bytes(s.as_bytes());
        let mut a = self.atoms.lock().expect("the symbols");
        if let Some(id) = a.find(s, h) {
            return i64::from(id);
        }
        i64::from(a.append(s, h).expect("the symbols' block is full"))
    }

    /// The id of `s`, when it has one.
    pub fn find(&self, s: &str) -> Option<i64> {
        let h = rerail_blocks::maps::hash_bytes(s.as_bytes());
        self.atoms
            .lock()
            .expect("the symbols")
            .find(s, h)
            .map(i64::from)
    }

    /// The string of `id`, for messages and tests.
    pub fn name(&self, id: i64) -> String {
        let a = self.atoms.lock().expect("the symbols");
        // copy: not block data (a message's text)
        u32::try_from(id)
            .ok()
            .and_then(|i| a.str(i))
            .map_or_else(|| format!("#{id}"), str::to_owned)
    }
}
