//! Where a tape's bytes live (decision 0353 step 4a). A tape just written holds its own vector, in the run's blocks
//! (`rerail_vector`, decision 0628 §3), or the room its writer wrote it in (row 17). A loaded one is
//! placed: its bytes are in a room someone else holds, a residency slab that the device reads too, and every reader
//! reads them there. Nothing copies a placed tape.

use std::sync::Arc;

/// A room holding a tape's bytes, read in place (`Home::Placed`): the engine's slab room.
pub trait Room: Send + Sync + std::fmt::Debug {
    fn bytes(&self) -> &[u8];
    /// The room itself, for its holder to find where the device reads it (the engine's slab and offset).
    fn as_any(&self) -> &dyn std::any::Any;
}

/// Where a writer writes its tapes (decision 0353 step 4a, row 17): rooms where every reader reads them, the device
/// too. A writer writes a tape into a region's free bytes as it goes, and gives the tape out of the region where it
/// lies when it finishes: nothing copies it into a room after.
pub trait Rooms: Send + Sync {
    /// A region of at least `len` free bytes, its taker's alone; `None` where none is given (a device's budget spent),
    /// and the writer keeps its tape in a vector of its own.
    fn region(&self, len: usize) -> Option<Box<dyn Region>>;
}

/// A writer's region of a home's memory (`Rooms::region`): its free bytes are the writer's to write, and a tape
/// written at their start is given out as a room (`give`). What a region holds when dropped and was never given is
/// given back where the home can take it.
pub trait Region: Send {
    /// Its free bytes, from the first not given out.
    fn free(&mut self) -> &mut [u8];
    /// The first `len` of its free bytes given out as a room, never written again; its free bytes start after them.
    fn give(&mut self, len: usize) -> Arc<dyn Room>;
}

/// A tape's bytes: its own, or placed in a room. Either is shared by every holder (a clone is the same bytes, never
/// a copy of them): a tape is written once, by its writer or its load, and read where it lies after.
#[derive(Clone, Debug)]
pub enum Home {
    Own(Arc<crate::Vector<u8>>),
    Placed(Arc<dyn Room>),
}

impl Home {
    /// The bytes, mutable: a tape's own (a test's header fix-up, `Tape16::set_tag`), made its alone where a clone
    /// shares them. A placed tape is never written.
    pub fn own_mut(&mut self) -> &mut crate::Vector<u8> {
        match self {
            Home::Own(v) => Arc::make_mut(v),
            Home::Placed(_) => panic!("a placed tape is never written"),
        }
    }

    /// The bytes as a vector, where they are a tape's own and held by no one else (a writer takes its tape's room back).
    pub fn into_own(self) -> Option<crate::Vector<u8>> {
        match self {
            Home::Own(v) => Arc::try_unwrap(v).ok(),
            Home::Placed(_) => None,
        }
    }

    /// The capacity of the bytes held apart from any room: a tape's own (0 where they are shared or placed, counted by
    /// their room's holder).
    pub fn own_capacity(&self) -> usize {
        match self {
            Home::Own(v) if Arc::strong_count(v) == 1 => v.capacity(),
            _ => 0,
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
        Home::Own(Arc::new(v))
    }
}

/// Bytes the store holds (a decompressed object): a room someone else holds, read where they lie.
#[derive(Debug)]
struct Held(Arc<Vec<u8>>);

impl Room for Held {
    fn bytes(&self) -> &[u8] {
        &self.0
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl From<Arc<Vec<u8>>> for Home {
    /// Bytes already shared (a store's decompressed object): held with them, not copied.
    fn from(v: Arc<Vec<u8>>) -> Self {
        Home::Placed(Arc::new(Held(v)))
    }
}

/// A document's source (decision 0353 step 7, row 13): its UTF-8 bytes, its own or placed in a room a reader loaded
/// them into, which the device reads too. Every reader reads it as a `str` where it lies; nothing copies it into a
/// slab for the device.
#[derive(Clone, PartialEq, Eq)]
pub struct Text(Home);

impl Text {
    /// The text of `home`'s bytes, checked as UTF-8 once.
    pub fn new(home: Home) -> Result<Text, std::str::Utf8Error> {
        std::str::from_utf8(&home)?;
        Ok(Text(home))
    }

    /// The text.
    #[inline]
    pub fn as_str(&self) -> &str {
        // SAFETY: checked as UTF-8 when made (`new`, or a `String`'s own), and never written after.
        unsafe { std::str::from_utf8_unchecked(&self.0) }
    }

    /// Where its bytes lie: its own, or a room.
    pub fn home(&self) -> &Home {
        &self.0
    }

    /// Its bytes' home, given up.
    pub fn into_home(self) -> Home {
        self.0
    }
}

impl Default for Text {
    fn default() -> Self {
        Text(Home::from(crate::Vector::new()))
    }
}

impl From<String> for Text {
    /// A string's bytes, its own, in the run's blocks (`rerail_vector`, decision 0628 §3): the string is the text as
    /// it was read or made, and this is where it is kept.
    fn from(s: String) -> Self {
        Text(Home::from(crate::Vector::from(s.as_bytes())))
    }
}

impl From<&str> for Text {
    fn from(s: &str) -> Self {
        // copy: not block data: a text made from a string slice (tests, edits)
        Text(Home::from(crate::Vector::from(s.as_bytes())))
    }
}

impl std::ops::Deref for Text {
    type Target = str;
    #[inline]
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for Text {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<[u8]> for Text {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl std::borrow::Borrow<str> for Text {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl std::hash::Hash for Text {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        self.as_str().hash(h)
    }
}

impl std::fmt::Debug for Text {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self.as_str(), f)
    }
}

impl std::fmt::Display for Text {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl PartialEq<str> for Text {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for Text {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<String> for Text {
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other
    }
}

#[cfg(feature = "ast-grep")]
impl ast_grep_core::source::Content for Text {
    type Underlying = u8;
    fn get_range(&self, range: std::ops::Range<usize>) -> &[u8] {
        &self.as_bytes()[range]
    }
    fn decode_str(src: &str) -> std::borrow::Cow<'_, [u8]> {
        std::borrow::Cow::Borrowed(src.as_bytes())
    }
    fn encode_bytes(bytes: &[u8]) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(bytes)
    }
    fn get_char_column(&self, _col: usize, offset: usize) -> usize {
        // As `String`'s: the characters back to the line's start.
        self.as_bytes()[..offset].iter().rev().take_while(|&&b| b != b'\n').filter(|&&b| b & 0b1100_0000 != 0b1000_0000).count()
    }
}
