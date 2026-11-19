//! Deferrable slabs (decision 0295 §1): a block in a file-backed shared mapping, written by the host or by a kernel
//! through a no-copy wrap, and handed at a wave's end to the store's writer, which makes the OS write it back.
//!
//! The hand-off has two halves, on two threads:
//!
//! - **`Deferrable::seal`**, on the wave's thread once the wave's command buffers have completed: the wrap is let
//!   go, the length written, the block made read-only to its readers. Its state stays open. Nothing here waits for
//!   the disk: on macOS `msync` and `munmap` of the host's dirty pages both wait for the writes (0295 §7).
//! - **`Sealed::write_back`**, on the writer: `msync` of the mapping (the host's dirty pages), the mapping unmapped
//!   once its readers let go (the GPU driver holds the pages a kernel wrote wired until then, and a wired page is
//!   never written back), the file truncated to the length, mapped again read-only and `msync`ed with `MS_SYNC` (the
//!   kernel's pages, dirty since they were wired), and only then the header's state written, frozen. A segment whose
//!   state says frozen has every page written back; one that does not is not trusted (`Block::open_file`).
//!
//! A block no device wrapped (the host's writes alone) is written back in place: one `MS_SYNC`, the truncation and
//! the state, and its mapping and readers stay.
//!
//! Nothing else writes a kernel's pages back: not `sync`, not the OS's flush, not the process's exit (0295 §7). So
//! the state written last is what makes them safe.

use crate::block::{Backing, Block, FROZEN, Frozen, Header};
use std::fs::OpenOptions;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::sync::Mutex;
use std::sync::{Arc, Weak};
use std::time::Duration;

/// A file's shared mapping: unmapped when the last of its holders (the block's backing, the hand-off) lets go.
struct Mapping {
    base: *mut u8,
    len: usize,
    /// Wrapped for a device: a kernel may have written it, so its write-back unmaps it first (module doc).
    wrapped: bool,
    /// The no-copy wrap the GPU binds the block through, made once with the mapping; let go at the seal.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    gpu: Mutex<Option<rerail_device::device::Buffer>>,
}

unsafe impl Send for Mapping {}
unsafe impl Sync for Mapping {}

impl Drop for Mapping {
    fn drop(&mut self) {
        unsafe { libc::munmap(self.base.cast(), self.len) };
    }
}

/// The block's view of the mapping.
struct Shared(Arc<Mapping>);

impl Backing for Shared {
    fn base(&self) -> *mut u8 {
        self.0.base
    }

    fn len(&self) -> usize {
        self.0.len
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn buffer(&self) -> Option<rerail_device::device::Buffer> {
        self.0.gpu.lock().unwrap().clone()
    }
}

/// An open deferrable block: written by the wave, then sealed.
pub struct Deferrable {
    /// Taken at the seal.
    block: Option<Block>,
    mapping: Arc<Mapping>,
    path: PathBuf,
    kind: u32,
    /// Told once, at the seal or when dropped unsealed: the slab has left its owner's budget (the pool's live leases).
    on_release: Option<Box<dyn FnOnce() + Send>>,
}

impl Drop for Deferrable {
    fn drop(&mut self) {
        if let Some(f) = self.on_release.take() {
            f();
        }
    }
}

impl Deferrable {
    /// A block of `kind` in the file at `path`, `len` bytes (a page multiple) mapped shared. `fresh`: the file is
    /// new, so it reads zero; else it holds an earlier segment's bytes and every allocation is zeroed as it is made.
    /// With a device, the mapping is wrapped for it once, here.
    pub fn create(
        path: &Path,
        kind: u32,
        len: usize,
        fresh: bool,
        #[cfg(any(target_os = "macos", target_os = "linux"))] device: Option<&rerail_device::device::Device>,
    ) -> io::Result<Deferrable> {
        let len = len.max(crate::block::PAGE).div_ceil(crate::block::PAGE) * crate::block::PAGE;
        let f = OpenOptions::new().read(true).write(true).create(true).truncate(fresh).open(path)?;
        f.set_len(len as u64)?;
        let p = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, f.as_raw_fd(), 0) };
        if p == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the mapping outlives the wrap, which it holds and lets go at the seal, before it is unmapped.
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        let gpu = match device.map(|d| unsafe { rerail_device::device::wrap_host_memory(d, p.cast(), len) }).transpose() {
            Ok(g) => g,
            Err(e) => {
                unsafe { libc::munmap(p, len) };
                return Err(io::Error::other(e));
            }
        };
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        let wrapped = gpu.is_some();
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        let wrapped = false;
        let mapping = Arc::new(Mapping {
            base: p.cast(),
            len,
            wrapped,
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            gpu: Mutex::new(gpu),
        });
        let block = Block::over_with(kind, Box::new(Shared(mapping.clone())), !fresh);
        Ok(Deferrable { block: Some(block), mapping, path: path.to_owned(), kind, on_release: None })
    }

    pub fn block(&self) -> &Block {
        self.block.as_ref().expect("an open deferrable block")
    }

    /// The mapping's length: the slab's size class, as the pool knows it.
    pub fn slab_len(&self) -> usize {
        self.mapping.len
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Sets what the seal (or a drop unsealed) tells its owner, once.
    pub fn on_release(&mut self, f: impl FnOnce() + Send + 'static) {
        self.on_release = Some(Box::new(f));
    }

    /// The wave's half (module doc): every command buffer that used the block has completed. Does not wait for the
    /// disk.
    pub fn seal(mut self) -> Sealed {
        let block = self.block.take().expect("an open deferrable block");
        let (mapping, path, kind) = (self.mapping.clone(), std::mem::take(&mut self.path), self.kind);
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            block.release_gpu_buffer();
            mapping.gpu.lock().unwrap().take();
        }
        let length = block.used();
        let frozen = block.seal();
        // `self` dropped here tells the owner (`on_release`).
        drop(self);
        Sealed { frozen, mapping, path, kind, length }
    }
}

/// A sealed deferrable block, on its way to the writer: read in place meanwhile.
pub struct Sealed {
    frozen: Frozen,
    mapping: Arc<Mapping>,
    path: PathBuf,
    kind: u32,
    length: usize,
}

impl Sealed {
    /// A reader's handle: the block as it was sealed, in the wave's mapping. The writer's unmap waits for every
    /// reader to let go, so a reader holds it for a read, not for a run.
    pub fn reader(&self) -> Frozen {
        self.frozen.clone()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn length(&self) -> usize {
        self.length
    }

    /// The mapping's length: the slab's size class, for `return_deferrable` once archived.
    pub fn slab_len(&self) -> usize {
        self.mapping.len
    }

    /// The writer's half (module doc). Returns the block mapped again read-only, frozen, every page written back.
    pub fn write_back(self) -> io::Result<Frozen> {
        let Sealed { frozen, mapping, path, kind, length } = self;
        // Written by the host alone: no page is wired, so one `MS_SYNC` writes every one, and the mapping stays,
        // its readers reading on (the block a queued write's readers hold).
        if !mapping.wrapped {
            msync(mapping.base, length, libc::MS_SYNC)?;
            let f = OpenOptions::new().read(true).write(true).open(&path)?;
            f.set_len(length as u64)?;
            f.write_all_at(&FROZEN.to_ne_bytes(), std::mem::offset_of!(Header, state) as u64)?;
            let _ = kind;
            return Ok(frozen);
        }
        // The host's dirty pages: written now, on this thread. A kernel's are wired and skipped.
        msync(mapping.base, length, libc::MS_ASYNC)?;
        // The unmap, once the readers let go: the driver's wire on a kernel's pages ends with it.
        let gone: Weak<Mapping> = Arc::downgrade(&mapping);
        drop(mapping);
        drop(frozen);
        let mut wait = Duration::from_micros(50);
        while gone.strong_count() > 0 {
            std::thread::sleep(wait);
            wait = (wait * 2).min(Duration::from_millis(10));
        }
        let f = OpenOptions::new().read(true).write(true).open(&path)?;
        f.set_len(length as u64)?;
        // The kernel's pages, dirty since their wire, written back through a mapping of their own. The driver lets go
        // of them some time after the unmap (190–260 ms measured, whatever the size), and an `msync` meanwhile skips
        // them and still succeeds: so it is repeated until no page of the range is dirty.
        {
            let p = unsafe { libc::mmap(std::ptr::null_mut(), length, libc::PROT_READ, libc::MAP_SHARED, f.as_raw_fd(), 0) };
            if p == libc::MAP_FAILED {
                return Err(io::Error::last_os_error());
            }
            let r = written_back(p.cast(), length);
            unsafe { libc::munmap(p, length) };
            r?;
        }
        // The state, last.
        f.write_all_at(&FROZEN.to_ne_bytes(), std::mem::offset_of!(Header, state) as u64)?;
        drop(f);
        Block::open_file(&path, kind).map_err(io::Error::other)
    }
}

/// How long the writer waits for the driver to let go of a range's pages before it gives the segment up.
#[cfg(target_os = "macos")]
const LET_GO: Duration = Duration::from_secs(10);

/// `msync`s the range with `MS_SYNC`: elsewhere than macOS no page is wired for a GPU, and one `msync` writes the
/// range back (Linux's `mincore` reports no dirty pages to wait on).
#[cfg(not(target_os = "macos"))]
fn written_back(base: *mut u8, len: usize) -> io::Result<()> {
    msync(base, len, libc::MS_SYNC)
}

/// `msync`s the range with `MS_SYNC` until no page of it is dirty (`mincore`): a page still wired for the GPU is
/// skipped by each `msync` until the driver lets go of it.
#[cfg(target_os = "macos")]
fn written_back(base: *mut u8, len: usize) -> io::Result<()> {
    let pages = len.div_ceil(crate::block::PAGE);
    let mut v = vec![0 as libc::c_char; pages];
    let start = std::time::Instant::now();
    let mut wait = Duration::from_millis(1);
    loop {
        msync(base, len, libc::MS_SYNC)?;
        if unsafe { libc::mincore(base as *const libc::c_void, len, v.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let dirty = v.iter().filter(|&&c| c as i32 & (libc::MINCORE_MODIFIED | libc::MINCORE_MODIFIED_OTHER) != 0).count();
        if dirty == 0 {
            return Ok(());
        }
        if start.elapsed() > LET_GO {
            return Err(io::Error::other(format!("{dirty} of {pages} pages still dirty after {LET_GO:?}: not written back")));
        }
        std::thread::sleep(wait);
        wait = (wait * 2).min(Duration::from_millis(20));
    }
}

fn msync(base: *mut u8, len: usize, flags: i32) -> io::Result<()> {
    if len == 0 {
        return Ok(());
    }
    if unsafe { libc::msync(base.cast(), len, flags) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// The bytes of `path` the file system holds on disk: a sparse file's written-back extent.
pub fn on_disk(path: &Path) -> u64 {
    std::fs::metadata(path).map_or(0, |m| std::os::unix::fs::MetadataExt::blocks(&m) * 512)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::kind;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rerail-deferrable-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn create(path: &Path, len: usize, fresh: bool) -> Deferrable {
        Deferrable::create(
            path,
            kind::JOURNAL,
            len,
            fresh,
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            None,
        )
        .unwrap()
    }

    /// The host's writes: read in place once sealed, written back whole, frozen only by the writer.
    #[test]
    fn a_sealed_slab_is_written_back_and_then_frozen() {
        let d = dir("host");
        let path = d.join("seg-1");
        let mut slab = create(&path, 4 << 20, true);
        let sealed_told = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let told = sealed_told.clone();
        slab.on_release(move || told.store(true, std::sync::atomic::Ordering::Relaxed));
        let off = slab.block().alloc(1 << 20, 64);
        let p = slab.block().ptr::<u32>(off);
        for i in 0..(1 << 18) {
            unsafe { *p.add(i) = i as u32 ^ 0x5a5a };
        }
        let sealed = slab.seal();
        assert!(sealed_told.load(std::sync::atomic::Ordering::Relaxed), "the seal tells the owner");
        let r = sealed.reader();
        assert_eq!(unsafe { *r.ptr::<u32>(off).add(7) }, 7 ^ 0x5a5a, "a reader reads the sealed block in place");
        drop(r);
        assert!(Block::open_file(&path, kind::JOURNAL).is_err(), "not frozen before the writer has written it back");
        let length = sealed.length();
        let back = sealed.write_back().unwrap();
        assert_eq!(back.used(), length);
        assert_eq!(unsafe { *back.ptr::<u32>(off).add(1000) }, 1000 ^ 0x5a5a);
        assert!(on_disk(&path) >= (1 << 20), "written back: {} bytes on disk", on_disk(&path));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A file reused (an archived segment's) holds the old bytes: the block zeroes what it allocates.
    #[test]
    fn a_reused_file_reads_zero_where_allocated() {
        let d = dir("reuse");
        let path = d.join("seg-1");
        let slab = create(&path, 1 << 20, true);
        let off = slab.block().alloc(64 << 10, 64);
        unsafe { std::ptr::write_bytes(slab.block().ptr::<u8>(off), 0xff, 64 << 10) };
        slab.seal().write_back().unwrap();
        let again = create(&path, 1 << 20, false);
        let off = again.block().alloc(64 << 10, 64);
        let bytes = again.block().bytes(off, off + (64 << 10));
        assert!(bytes.iter().all(|&b| b == 0), "an allocation in a reused file reads zero");
        drop(again);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A kernel's writes through the wrap reach the disk (0295 §7: without the unmap, none did).
    #[cfg(target_os = "macos")]
    #[test]
    fn a_kernels_writes_are_written_back() {
        use metal::*;
        let Some(device) = Device::system_default() else { return };
        let d = dir("gpu");
        let path = d.join("seg-1");
        let len = 16usize << 20;
        let slab = Deferrable::create(&path, kind::JOURNAL, len + (1 << 16), true, Some(&rerail_device::device::Device::Native(device.clone()))).unwrap();
        let off = slab.block().alloc(len, crate::block::PAGE);
        const SRC: &str = "kernel void fill(device uint *p [[buffer(0)]], uint i [[thread_position_in_grid]]) { p[i] = i * 2654435761u; }";
        metal::objc::rc::autoreleasepool(|| {
            let lib = device.new_library_with_source(SRC, &CompileOptions::new()).unwrap();
            let pipe = device.new_compute_pipeline_state_with_function(&lib.get_function("fill", None).unwrap()).unwrap();
            let queue = device.new_command_queue();
            let buf = slab.block().gpu_buffer(&device);
            let cmd = queue.new_command_buffer();
            let enc = cmd.new_compute_command_encoder();
            enc.set_compute_pipeline_state(&pipe);
            enc.set_buffer(0, Some(&buf), off as u64);
            enc.dispatch_threads(MTLSize::new((len / 4) as u64, 1, 1), MTLSize::new(256, 1, 1));
            enc.end_encoding();
            cmd.commit();
            cmd.wait_until_completed();
        });
        let back = slab.seal().write_back().unwrap();
        let p = back.ptr::<u32>(off);
        assert!((0..(len / 4) as u32).all(|i| unsafe { *p.add(i as usize) } == i.wrapping_mul(2654435761)), "the kernel's words");
        assert!(on_disk(&path) >= len as u64, "a kernel's pages on disk: {} of {len} bytes", on_disk(&path));
        drop(back);
        let _ = std::fs::remove_dir_all(&d);
    }
}
