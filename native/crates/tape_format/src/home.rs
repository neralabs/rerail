//! Where a tape's bytes live (decision 0353 step 4a). A tape just written holds its own vector, in the run's blocks
//! (`rerail_vector`, decision 0628 §3). A loaded one is
//! placed: its bytes are in a room someone else holds, a residency slab that the device reads too, and every reader
//! reads them there. Nothing copies a placed tape.

use std::sync::Arc;

/// A room holding a tape's bytes, read in place (`Home::Placed`): the engine's slab room.
pub trait Room: Send + Sync + std::fmt::Debug {
    fn bytes(&self) -> &[u8];
    /// The room itself, for its holder to find where the device reads it (the engine's slab and offset).
    fn as_any(&self) -> &dyn std::any::Any;
}

/// A tape's bytes: its own, or placed in a room.
#[derive(Clone, Debug)]
pub enum Home {
    Own(crate::Vector<u8>),
    Placed(Arc<dyn Room>),
}

impl Home {
    /// The bytes, mutable: a tape's own only (a writer's header fix-up). A placed tape is never written.
    pub fn own_mut(&mut self) -> &mut crate::Vector<u8> {
        match self {
            Home::Own(v) => v,
            Home::Placed(_) => panic!("a placed tape is never written"),
        }
    }

    /// Whether the bytes are in a room (not the tape's own).
    pub fn placed(&self) -> bool {
        matches!(self, Home::Placed(_))
    }

    /// The room the bytes are in, if they are placed.
    pub fn room(&self) -> Option<&Arc<dyn Room>> {
        match self {
            Home::Own(_) => None,
            Home::Placed(r) => Some(r),
        }
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

impl AsRef<[u8]> for Home {
    fn as_ref(&self) -> &[u8] {
        self
    }
}

impl PartialEq for Home {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}
impl Eq for Home {}

impl From<crate::Vector<u8>> for Home {
    fn from(v: crate::Vector<u8>) -> Self {
        Home::Own(v)
    }
}
