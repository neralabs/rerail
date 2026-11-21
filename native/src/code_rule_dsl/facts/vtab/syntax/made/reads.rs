//! The reads layer of a batch's views' directory (decision 0353 step 3 on the CPU): each document's row written on
//! its document's first read through the views, beside its search layer (`made::directory`). A row names its entry
//! (`ViewDoc`), its tape and its lines where they lie (its homes, `2i` and `2i + 1`) and its tape's view; the host's
//! reads (`rerail_tape_format::view_reads::Doc`) and the device's (`device_chunk::views_in_place`) read the same row.
//! Nothing here holds a view's rows: a row is read off the tape.

use super::Docs;
use crate::code_rule_dsl::facts::vtab::ZCache;
use crate::code_rule_dsl::facts::vtab::syntax::view_docs::ViewEntry;
use crate::code_rule_dsl::tape::placed;
use rerail_clif::view_reads::{ViewDir, ViewDoc, ViewHome, ViewRow};
use rerail_tape_format::view_reads::{Doc, Tape};
use std::sync::{Arc, OnceLock};

/// A document's row of the reads layer, once written.
pub struct RowReads {
    /// Its entry, in the batch's block: none for a document with no tape the views read (no nodes, and its `Loc`
    /// rows not the reads').
    pub entry: Option<ViewEntry>,
    /// Whether it has nodes (else its rows are its `Loc` views' alone).
    pub nodes: bool,
    /// Whether its `Loc` rows are the reads' (`ViewEntry::locs`, its tape read).
    pub locs: bool,
    /// Whether the device reads its tape, and its lines, where they lie (each in a slab).
    pub tape_device: bool,
    pub lines_device: bool,
    /// The slabs its tape and its lines lie in, which a pass that reads them makes resident.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub slabs: Vec<rerail_device::device::Buffer>,
}

/// The batch's reads layer: each row's, once written (or why it could not be), and the 16z cache the rows share.
pub(super) struct Layer {
    rows: Box<[OnceLock<Result<RowReads, &'static str>>]>,
    /// The 16z documents' cache (decision 0353 step 4b, `view::cache`), in a block of its own (home `2n`), made on
    /// the first 16z row: the host's and the device's reads share it.
    cache: OnceLock<Option<ZCache>>,
}

impl Layer {
    pub(super) fn new(n: usize) -> Layer {
        Layer { rows: (0..n).map(|_| OnceLock::new()).collect(), cache: OnceLock::new() }
    }
}

/// The cache's counts in the profile when the batch goes (the statistics mode's too, where it is on).
impl Drop for Layer {
    fn drop(&mut self) {
        if let Some(Some(cache)) = self.cache.get() {
            for (name, n) in cache.counts().named() {
                crate::code_rule_dsl::prof::count(name, n as usize);
            }
        }
    }
}

impl Docs {
    /// Document `i`'s row of the reads layer, written on the first ask (its document made): `Err` with why where its
    /// views are not read through the directory (counted once, by why). Where `entry` is given (the eager directory,
    /// `device_chunk::views_in_place`), the 16z cache is made for a block count and an entry size of the whole
    /// batch's, else of the batch's node rows and this document's.
    pub fn reads(&self, i: usize) -> Result<&RowReads, &'static str> {
        self.reads_sized(i, None)
    }

    pub(in crate::code_rule_dsl::facts::vtab::syntax) fn reads_sized(&self, i: usize, size: Option<(usize, usize)>) -> Result<&RowReads, &'static str> {
        self.layer.rows[i]
            .get_or_init(|| {
                let r = self.write_row(i, size);
                if let Err(why) = r {
                    crate::code_rule_dsl::prof::count(why, 1);
                }
                r
            })
            .as_ref()
            .map_err(|&w| w)
    }

    /// The 16z cache, where a row made it.
    pub fn zcache_made(&self) -> Option<&ZCache> {
        self.layer.cache.get().and_then(Option::as_ref)
    }

    /// The 16z cache, made on the first ask for `blocks` blocks of entries of `entry` bytes (`Spec::configured`).
    pub fn zcache(&self, blocks: usize, entry: usize) -> Option<&ZCache> {
        self.layer
            .cache
            .get_or_init(|| {
                let spec = rerail_clif::view::cache::Spec::configured(blocks, entry);
                crate::code_rule_dsl::prof::count("zcache.batches", 1);
                crate::code_rule_dsl::prof::count("zcache.blocks", blocks);
                crate::code_rule_dsl::prof::count("zcache.entries", spec.entries as usize);
                crate::code_rule_dsl::prof::count("zcache.entry_kb", spec.entry_bytes() >> 10);
                let block = Arc::new(rerail_blocks::Block::anon(rerail_blocks::block::kind::SCRATCH, spec.bytes() + 2 * 4096));
                let at = block.alloc(spec.bytes(), rerail_clif::view::cache::ALIGN);
                // SAFETY: the cache's bytes, just allocated in its block, read by no reader yet.
                spec.init(unsafe { std::slice::from_raw_parts_mut(block.ptr::<u8>(at), spec.bytes()) }).ok()?;
                #[cfg(any(target_os = "macos", target_os = "linux"))]
                let buffer = placed::device().and_then(|d| block.device_buffer(d).ok());
                #[cfg(not(any(target_os = "macos", target_os = "linux")))]
                let buffer = None;
                let cache = ZCache { block, at, len: spec.bytes(), buffer, entry: spec.entry_bytes() };
                // Its home, `2n`: the directory names it (`ViewDir::cache`, its number + 1).
                let n = self.len();
                let host = cache.block.ptr::<u8>(cache.at) as u64;
                let gpu = cache.buffer.as_ref().and_then(|b| b.device_address()).map_or(0, |a| a + cache.at as u64);
                let dir = self.dir().cast_mut();
                // SAFETY: the directory's home `2n` and its `cache` field, in the batch's block: the reads layer's,
                // which no reader reads before a 16z row (written after this) names it.
                unsafe {
                    ViewDir::homes_mut(dir).add(2 * n).write(ViewHome { host, gpu });
                    (&raw mut (*dir).cache).write(2 * n as u64 + 1);
                }
                Some(cache)
            })
            .as_ref()
    }

    /// Document `i`'s row written: its entry, its tape and lines in their homes, its view.
    fn write_row(&self, i: usize, size: Option<(usize, usize)>) -> Result<RowReads, &'static str> {
        let p = &self[i];
        let b = &self.context.block;
        if !Arc::ptr_eq(p.block(), b) {
            return Err("views.refused.block");
        }
        let nodes = p.len() > 0;
        // The `Loc` rows are the reads' where the positions are a file's own tree's or a placement in the block.
        let locs = p.extra.is_some() && (p.locs() == 0 || ViewEntry::locs(b, p).is_some());
        if !locs && p.locs() > 0 {
            let why = match p.extra.as_ref().and_then(|x| x.host.as_ref()).map(|(_, place)| place) {
                Some(crate::code_rule_dsl::facts::vtab::syntax::Placement::Copied { .. }) => "views.refused.locs.guest_copied",
                Some(crate::code_rule_dsl::facts::vtab::syntax::Placement::Mapped { .. }) => "views.refused.locs.guest_mapped",
                Some(crate::code_rule_dsl::facts::vtab::syntax::Placement::At(_)) => "views.refused.locs.guest_at",
                Some(crate::code_rule_dsl::facts::vtab::syntax::Placement::Segments(_)) => "views.refused.locs.guest_segments",
                None => "views.refused.locs",
            };
            crate::code_rule_dsl::prof::count(why, 1);
        }
        let empty = RowReads {
            entry: None,
            nodes,
            locs: false,
            tape_device: false,
            lines_device: false,
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            slabs: Vec::new(),
        };
        if !nodes && !(locs && p.locs() > 0) {
            return Ok(RowReads { locs, ..empty });
        }
        let tape = p.facts.tape.as_ref().ok_or("views.refused.no_tape");
        let in_place = tape.and_then(|t| t.in_place().map(|(bytes, view)| (t, bytes, view)).map_err(|_| "views.refused.not_in_place"));
        let (tape, bytes, mut view) = match in_place {
            Ok(t) => t,
            Err(why) if nodes => return Err(why),
            Err(why) => {
                // A document of `Loc` rows alone whose tape is not read: those are made.
                crate::code_rule_dsl::prof::count(why, 1);
                return Ok(empty);
            }
        };
        let tape_home = placed::home_of(tape.image().room(), bytes);
        let table = &p.facts.lines;
        let words = table.words();
        // SAFETY: the words as bytes, their own range.
        let line_bytes = unsafe { std::slice::from_raw_parts(words.as_ptr().cast::<u8>(), 4 * words.len()) };
        let lines_home = placed::home_of(table.room(), line_bytes);
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        let (tape_gpu, lines_gpu) = (tape_home.device.as_ref().map_or(0, |d| d.0), lines_home.device.as_ref().map_or(0, |d| d.0));
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        let (tape_gpu, lines_gpu) = (0u64, 0u64);
        // A 16z tape's reads decode its blocks into the batch's cache: its entry size in the view, its reads checked
        // (shared), and the device's offset of the cache from the tape (the kernels read the view where it lies).
        if view.format == rerail_clif::view::F16Z_FORMAT {
            let entry = view.columns[3] as usize;
            let (blocks, largest) = size.unwrap_or_else(|| (self.z_blocks(), entry));
            let cache = self.zcache(blocks, largest.max(entry)).ok_or("views.refused.zcache")?;
            let spec_bytes = cache.entry;
            if spec_bytes < entry {
                return Err("views.refused.zentry");
            }
            (view.columns[3], view.columns[4]) = (spec_bytes as u32, 1);
            let cache_gpu = cache.buffer.as_ref().and_then(|b| b.device_address()).map_or(0, |a| a + cache.at as u64);
            if cache_gpu != 0 && tape_gpu != 0 {
                let rel = cache_gpu.wrapping_sub(tape_gpu + tape_home.at) as i64;
                (view.side[0], view.side[1]) = (rel as u32, (rel >> 32) as u32);
            }
        }
        let entry = ViewEntry::new(b, p, &self.context.dialects[p.tag]);
        let dir = self.dir().cast_mut();
        // SAFETY: row `i` and its homes `2i`, `2i + 1`, in the batch's block: the reads layer's fields, which only this
        // write writes (once, in the row's `OnceLock`), and the search layer's readers do not read.
        unsafe {
            let homes = ViewDir::homes_mut(dir);
            homes.add(2 * i).write(ViewHome { host: tape_home.host, gpu: tape_gpu });
            homes.add(2 * i + 1).write(ViewHome { host: lines_home.host, gpu: lines_gpu });
            let at = ViewDir::block_rows(dir).add(i);
            (&raw mut (*at).entry).write(b.offset_of(entry.desc()) as u64);
            (&raw mut (*at).tape).write(tape_home.at);
            (&raw mut (*at).lines).write(lines_home.at);
            (&raw mut (*at).view).write(view);
            (&raw mut (*at).bytes).write(u32::try_from(bytes.len()).map_err(|_| "views.refused.tape_past_4g")?);
        }
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        let slabs: Vec<rerail_device::device::Buffer> = [&tape_home, &lines_home].into_iter().filter_map(|h| h.device.as_ref().map(|d| d.1.clone())).collect();
        Ok(RowReads {
            entry: Some(entry),
            nodes,
            locs,
            tape_device: tape_gpu != 0,
            lines_device: lines_gpu != 0,
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            slabs,
        })
    }

    /// The blocks a 16z cache of the batch is sized for, with no document read: its node rows' blocks, each document's
    /// last part block a whole one.
    fn z_blocks(&self) -> usize {
        let block = rerail_clif::view::F16Z_BLOCK as usize;
        (0..self.len()).map(|i| (self.counts(i)[0] as usize).div_ceil(block)).sum()
    }

    /// Document `i`'s views as the host's reads read them (`view_reads::Doc`), through its row: `None` where its row
    /// has no tape read (`reads`), or `locs` is asked and its `Loc` rows are not the reads'.
    pub fn read(&self, i: usize, locs: bool) -> Option<Doc<'_>> {
        let r = self.reads(i).ok()?;
        r.entry.as_ref()?;
        if locs && !r.locs {
            return None;
        }
        let dir = self.dir();
        // SAFETY: row `i`'s reads layer, written (its `OnceLock` is set): its entry in the batch's block, its tape and
        // lines at their homes' host addresses, which its document's facts hold, the 16z cache in its block, which
        // the layer holds; all live as long as `self`.
        unsafe {
            let row: &ViewRow = &*ViewDir::block_rows(dir.cast_mut()).add(i);
            let homes = ViewDir::homes_mut(dir.cast_mut());
            let base = self.context.block.ptr::<u8>(0).cast_const();
            let entry = &*base.add(row.entry as usize).cast::<ViewDoc>();
            let tape = std::slice::from_raw_parts(((*homes.add(row.homes[0] as usize)).host + row.tape) as *const u8, row.bytes as usize);
            let lines = ((*homes.add(row.homes[1] as usize)).host + row.lines) as *const u32;
            // Its words: `[n, starts[n], ends[n]]` (`LineTable::words`).
            let lines = std::slice::from_raw_parts(lines, 1 + 2 * *lines as usize);
            let cache = match (*dir).cache {
                0 => std::ptr::null(),
                k => (*homes.add(k as usize - 1)).host as *const u8,
            };
            Some(Doc { entry, tape: Tape::new(tape, &row.view, cache), lines })
        }
    }
}
