//! Where a tape's bytes live (decision 0353 step 4a). A tape just written holds its own vector. A loaded one is
//! placed: its bytes are in a room someone else holds, a residency slab that the device reads too, and every reader
//! reads them there. Nothing copies a placed tape.

use std::sync::Arc;

/// A room holding a tape's bytes, read in place (`Home::Placed`): the engine's slab room.
pub trait Room: Send + Sync + std::fmt::Debug {
    fn bytes(&self) -> &[u8];
}

/// A tape's bytes: its own, or placed in a room.
#[derive(Clone, Debug)]
pub enum Home {
    Own(Vec<u8>),
    Placed(Arc<dyn Room>),
}

impl Home {
    /// The bytes, mutable: a tape's own only (a writer's header fix-up). A placed tape is never written.
    pub fn own_mut(&mut self) -> &mut Vec<u8> {
        match self {
            Home::Own(v) => v,
            Home::Placed(_) => panic!("a placed tape is never written"),
        }
    }

    /// Whether the bytes are in a room (not the tape's own).
    pub fn placed(&self) -> bool {
        matches!(self, Home::Placed(_))
    }
}

impl std::ops::Deref for Home {
    type Target = [u8];
    #[inline]
    fn deref(&self) -> &[u8] {
        match self {
            Home::Own(v) => v,
            Home::Placed(r) => r.bytes(),
        }
    }
}

impl PartialEq for Home {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}
impl Eq for Home {}

impl From<Vec<u8>> for Home {
    fn from(v: Vec<u8>) -> Self {
        Home::Own(v)
    }
}
