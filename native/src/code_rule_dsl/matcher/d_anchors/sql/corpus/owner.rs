//! Logical memberships pin immutable source versions in one SQL fact owner.
use super::{Corpus, File, Key, State};
use std::sync::Arc;

const IDLE_BYTES: usize = 64 * 1024 * 1024;

impl State {
    fn pin(&mut self, key: &Key) {
        if let Some(tick) = self.idle_keys.remove(key) {
            self.idle.remove(&tick);
            let file = &self.files[key];
            self.idle_bytes -= file.published_bytes.saturating_add(file.bytes);
        }
        *self.pins.entry(key.clone()).or_default() += 1;
    }

    pub(super) fn unpin(&mut self, key: &Key) {
        let pins = self.pins.get_mut(key).expect("pinned membership");
        *pins -= 1;
        if *pins == 0 {
            self.tick += 1;
            self.idle.insert(self.tick, key.clone());
            self.idle_keys.insert(key.clone(), self.tick);
            let file = &self.files[key];
            self.idle_bytes += file.published_bytes.saturating_add(file.bytes);
        }
    }

    pub(super) fn collect(&mut self) -> rusqlite::Result<()> {
        let mut retired = Vec::new();
        let mut bytes = self.idle_bytes;
        for (tick, key) in &self.idle {
            if bytes <= IDLE_BYTES {
                break;
            }
            let file = &self.files[key];
            bytes = bytes.saturating_sub(file.published_bytes.saturating_add(file.bytes));
            retired.push((*tick, key.clone(), file.fid));
        }
        self.writer
            .remove_many(&retired.iter().map(|(_, _, fid)| *fid).collect::<Vec<_>>())?;
        for (tick, key, _) in retired {
            self.idle.remove(&tick);
            self.idle_keys.remove(&key);
            self.pins.remove(&key);
            self.files.remove(&key);
        }
        self.idle_bytes = bytes;
        Ok(())
    }
}

impl Corpus {
    pub(super) fn bind(&self, state: &mut State, key: Key, file: Arc<File>) {
        let mut members = self.members.lock();
        if members
            .get(key.0.as_ref())
            .is_some_and(|(previous, _)| previous == &key)
        {
            return;
        }
        state.pin(&key);
        if let Some((old, _)) = members.insert(key.0.to_string(), (key, file)) {
            state.unpin(&old);
        }
    }
}

impl Drop for Corpus {
    fn drop(&mut self) {
        if let Some(state) = self.state.lock().as_mut() {
            for (key, _) in self.members.get_mut().values() {
                state.unpin(key);
            }
        }
    }
}
