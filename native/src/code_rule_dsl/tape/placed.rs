//! A loaded tape's one home (decision 0353 step 4a): a room in a residency slab, a GPU-shareable chunk block, which
//! the store's load writes the tape into once (a format 15 frame decompressed, format 16 and 16z as stored) and every
//! reader reads in place: the host's facts and matchers, the JIT and the kernels. A host without a device, or a pool
//! that refuses the room, keeps the tape in a vector of its own.

use rerail_tape_format::home::{Home, Room};

/// A slab's room, held: the slab lives as long as a tape placed in it.
#[cfg(any(target_os = "macos", target_os = "linux"))]
struct SlabRoom {
    slab: std::sync::Arc<rerail_metal::slabs::Slab>,
    at: usize,
    len: usize,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl std::fmt::Debug for SlabRoom {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SlabRoom {{ slab: {}, at: {}, len: {} }}", self.slab.id, self.at, self.len)
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
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
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn device() -> Option<&'static rerail_metal::lane::Device> {
    rerail_metal::lane::device().ok()
}

/// Where the device reads `bytes` of a tape placed in `room` (decision 0353 step 4d): their device address, and the
/// slab's buffer, which a pass that reads them makes resident. `None` for a room that is no slab's, bytes outside
/// it, or a device whose buffers have no addresses (OpenCL 1.2).
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn on_device(room: &std::sync::Arc<dyn Room>, bytes: &[u8]) -> Option<(u64, rerail_metal::lane::Buffer)> {
    let slab = &room.as_any().downcast_ref::<SlabRoom>()?.slab;
    let held = room.bytes().as_ptr_range();
    if bytes.as_ptr() < held.start || bytes.as_ptr_range().end > held.end {
        return None;
    }
    let buffer = slab.device_buffer();
    let at = bytes.as_ptr() as usize - slab.host() as usize;
    Some((buffer.device_address()? + at as u64, buffer.clone()))
}

/// `len` bytes written by `fill` into their home: a slab's room where the device's pool gives one, else a vector.
pub fn place(len: usize, fill: impl FnOnce(&mut [u8]) -> std::io::Result<()>) -> std::io::Result<Home> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
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
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    if device().is_some() {
        crate::code_rule_dsl::prof::count("tape.room_refused", 1);
    }
    let mut v = vec![0u8; len];
    fill(&mut v)?;
    Ok(Home::Own(v))
}
