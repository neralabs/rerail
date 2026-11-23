//! A loaded tape's one home (decision 0353 step 4a): a room in a residency slab, a GPU-shareable chunk block, which
//! the store's load writes the tape into once (a format 15 frame decompressed, format 16 and 16z as stored) and every
//! reader reads in place: the host's facts and matchers, the JIT and the kernels. A host without a device, or a pool
//! that refuses the room, keeps the tape in a vector of its own.

use rerail_tape_format::home::{Home, Room};

/// A slab's room, held: the slab lives as long as a tape placed in it.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
struct SlabRoom {
    slab: std::sync::Arc<rerail_metal::slabs::Slab>,
    at: usize,
    len: usize,
}

#[cfg(any(target_os = "macos", target_os = "linux", windows))]
impl std::fmt::Debug for SlabRoom {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SlabRoom {{ slab: {}, at: {}, len: {} }}", self.slab.id, self.at, self.len)
    }
}

#[cfg(any(target_os = "macos", target_os = "linux", windows))]
impl Room for SlabRoom {
    #[inline]
    fn bytes(&self) -> &[u8] {
        // SAFETY: the room `tape_room` gave this tape alone, written by `place` before the room was shared and never
        // again; the slab is held.
        unsafe { std::slice::from_raw_parts(self.slab.host().add(self.at), self.len) }
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// The device the slabs are on, if there is one: the process's, whatever its backend (decision 0440 step C).
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
pub fn device() -> Option<&'static rerail_metal::lane::Device> {
    rerail_metal::lane::device().ok()
}

/// The home `bytes` lie in, as a views' directory names it (`view_reads::ViewHome`, decision 0353 step 3 on the CPU):
/// its identity (the same for every tape of one slab), its host address, the offset of `bytes` in it, and where a
/// device reads it (its device address and its slab's buffer, which a pass that reads it makes resident). A room that
/// is no slab's, or no room (a tape or lines in a vector of their own), is a home of its own at `bytes`, which no
/// device reads.
pub struct HomeOf {
    pub id: usize,
    pub host: u64,
    pub at: u64,
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    pub device: Option<(u64, rerail_metal::lane::Buffer)>,
}

pub fn home_of(room: Option<&std::sync::Arc<dyn Room>>, bytes: &[u8]) -> HomeOf {
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    if let Some(r) = room.and_then(|r| r.as_any().downcast_ref::<SlabRoom>()) {
        let slab = &r.slab;
        let host = slab.host() as usize;
        // Bytes outside the room are no slab's: a home of their own.
        let held = r.bytes().as_ptr_range();
        if let Some(at) = (bytes.as_ptr() as usize).checked_sub(host)
            && bytes.as_ptr() >= held.start
            && bytes.as_ptr_range().end <= held.end
        {
            let buffer = slab.device_buffer();
            let device = buffer.device_address().map(|address| (address, buffer.clone()));
            return HomeOf { id: host, host: host as u64, at: at as u64, device };
        }
    }
    let _ = room;
    let host = bytes.as_ptr() as usize;
    HomeOf {
        id: host,
        host: host as u64,
        at: 0,
        #[cfg(any(target_os = "macos", target_os = "linux", windows))]
        device: None,
    }
}

/// `len` bytes written by `fill` into their home: a slab's room where the device's pool gives one, else a vector.
pub fn place(len: usize, fill: impl FnOnce(&mut [u8]) -> std::io::Result<()>) -> std::io::Result<Home> {
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    if device().is_some()
        // A word of room past the tape: the device's reads load a whole word where a value ends (`clif_word`).
        && let Ok((slab, at)) = rerail_metal::slabs::tape_room(len + 16)
    {
        // SAFETY: `at..at + len` is this room's alone, inside the slab's mapping; nothing reads it before `fill`
        // returns.
        let out = unsafe { std::slice::from_raw_parts_mut(slab.host().add(at), len) };
        fill(out)?;
        crate::code_rule_dsl::prof::count("tape.placed", 1);
        return Ok(Home::Placed(std::sync::Arc::new(SlabRoom { slab, at, len })));
    }
    // A device that refused the room (its budget spent): the tape is read apart, and a batch of it makes its views
    // (`device_chunk::views_in_place`).
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    if device().is_some() {
        crate::code_rule_dsl::prof::count("tape.room_refused", 1);
    }
    let mut v = vec![0u8; len];
    fill(&mut v)?;
    Ok(Home::Own(v))
}

/// `place` as the store's reads take it (`snapshot::Placer`): a file read straight into its home.
pub fn placer(len: usize, fill: &mut dyn FnMut(&mut [u8]) -> std::io::Result<()>) -> std::io::Result<Home> {
    place(len, fill)
}

/// The slab, offset and length of `room`, where it is a slab's (`place`'s): where the device reads its bytes.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
pub fn slab_room(room: &std::sync::Arc<dyn Room>) -> Option<(std::sync::Arc<rerail_metal::slabs::Slab>, usize, usize)> {
    room.as_any().downcast_ref::<SlabRoom>().map(|r| (std::sync::Arc::clone(&r.slab), r.at, r.len))
}

/// A document's source read from `path` straight into its home (decision 0353 step 7, row 13): a slab's room where
/// the device's pool gives one, which every reader then reads where it lies, the device too. Nothing is read twice.
pub fn read_text(path: &std::path::Path) -> std::io::Result<crate::code_rule_dsl::tape::Text> {
    let home = crate::git_workspace::snapshot::read_placed(path, &placer)?;
    crate::code_rule_dsl::tape::Text::new(home).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// `source` where a device reads it: as it is where it lies in a room already (a loader's, `read_text`) or where
/// there is no device; else written into a room of its own, which the document that takes it reads (the host and the
/// device alike), and the residency takes as it lies (decision 0353 step 7, row 13).
pub fn housed(source: std::sync::Arc<crate::code_rule_dsl::tape::Text>) -> std::sync::Arc<crate::code_rule_dsl::tape::Text> {
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    if !source.home().placed() && device().is_some() {
        // copy (0353 step 4a): a source made in memory (a test's literal, an edit's text, a store's read into a
        // vector) written into its room, as a parse's tape is (`place_stored`). It goes when its maker writes it there.
        const INTO_ROOM: rerail_prof::copies::Site = rerail_prof::copies::Site::new(17, "4a", "a source in hand written into its room (placed::housed)");
        let len = source.len();
        let home = INTO_ROOM.time(|| {
            (
                place(len, |out| {
                    // copy (0353 step 4a): the source (row 17).
                    out.copy_from_slice(source.as_bytes());
                    Ok(())
                }),
                len,
            )
        });
        if let Ok(home) = home
            && home.placed()
            && let Ok(text) = crate::code_rule_dsl::tape::Text::new(home)
        {
            return std::sync::Arc::new(text);
        }
    }
    source
}
