//! The device layer's primitives on every backend (`RERAIL_GPU` chooses: the target's GPU, `cpu`, `opencl`):
//! what the engine's kernels need of it beside launching (decision 0440 step B).

use crate::kernels::{launch, program, read};

const KERNELS: &str = r#"
#include <ukl/backend.h>

typedef struct Args {
    uint32_t n;
    uint32_t add;
} Args;
UKL_REFLECT_STRUCT(Args, n, add)

// Each word of the host's memory, read and written where it lies: `w[i] = w[i] * 2 + add`.
DEFINE_KERNEL2(device_tests, d_in_place,
               ARGS,     Args,     args, ,
               RWBUFFER, uint32_t, w,    "[n]",
               GID0,     uint32_t, i,    )
{
    if (i < args.n)
        w[i] = w[i] * 2u + args.add;
}
"#;

/// Page-aligned host memory of `words` u32s, a whole number of pages, the test's own (as a block's pages are).
struct Pages {
    ptr: *mut u8,
    layout: std::alloc::Layout,
}

impl Pages {
    fn new(words: usize) -> Pages {
        let page = crate::os::page_bytes();
        let layout = std::alloc::Layout::from_size_align((words * 4).next_multiple_of(page), page).unwrap();
        // SAFETY: a non-zero size.
        let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
        Pages { ptr, layout }
    }
    fn words(&mut self, n: usize) -> &mut [u32] {
        // SAFETY: the pages hold at least `n` words.
        unsafe { std::slice::from_raw_parts_mut(self.ptr.cast(), n) }
    }
}

impl Drop for Pages {
    fn drop(&mut self) {
        unsafe { std::alloc::dealloc(self.ptr, self.layout) };
    }
}

/// Memory the host owns, wrapped without a copy (`wrap_host_memory`), is what the kernels read and write: the host's
/// words before the launch are the kernel's input, its writes are in the host's pages after it, and the host's
/// write between two launches is the second's input. Memory a backend cannot take in place is an error, not a copy.
#[test]
fn wrapped_host_memory_is_read_and_written_in_place() {
    let Some(shared) = crate::shared::get() else { return };
    let p = program("device_tests.uglykl", KERNELS).unwrap_or_else(|e| panic!("{e}"));
    let f = p.function("d_in_place").unwrap();
    const N: usize = 3000;
    let mut pages = Pages::new(N);
    for (i, w) in pages.words(N).iter_mut().enumerate() {
        *w = i as u32;
    }
    // SAFETY: `pages` outlives the buffer, which is dropped first, and the work, which is awaited.
    let b = unsafe { crate::device::wrap_host_memory(&shared.device, pages.ptr, pages.layout.size()) }.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(b.contents().cast::<u8>(), pages.ptr, "the buffer is the host's memory");
    let run = |add: u32| {
        let mut q = crate::device::ComputeQueue::new(&shared.queue);
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct Args {
            n: u32,
            add: u32,
        }
        unsafe impl crate::Pod for Args {}
        launch(&q, f).value("args", &Args { n: N as u32, add }).buffer("w", &b).run(&[N.div_ceil(64) as u64], &[64]).unwrap();
        q.flush().await_().unwrap();
    };
    run(1);
    assert!(pages.words(N).iter().enumerate().all(|(i, &w)| w == 2 * i as u32 + 1), "the kernel's writes are in the host's pages");
    pages.words(N)[7] = 100;
    run(0);
    assert_eq!(pages.words(N)[7], 200, "the host's write is the next launch's input");
    assert_eq!(read::<u32>(&b, 2), [2, 6]);
    drop(b);
    // Not page-aligned: Metal and OpenCL refuse it (a driver would copy it); the CPU and CUDA take any memory.
    let odd = unsafe { crate::device::wrap_host_memory(&shared.device, pages.ptr.add(4), 64) };
    if matches!(crate::device::backend(), "Metal" | "OpenCL") {
        assert!(odd.is_err(), "{} took unaligned memory", crate::device::backend());
    }
}

const INDIRECT: &str = r#"
#include <ukl/backend.h>

typedef struct Counts {
    uint32_t n;
    uint32_t pad;
} Counts;
UKL_REFLECT_STRUCT(Counts, n, pad)

// The work before an indirect launch: the groups it takes, written on the device (`n` items, 32 a group, in x;
// 2 in y; 1 in z), after 3 words of padding (the counts' offset).
DEFINE_KERNEL2(device_tests, d_count,
               ARGS,     Counts,   args,   ,
               RWBUFFER, uint32_t, groups, "[6]",
               GID0,     uint32_t, i,      )
{
    if (i == 0u) {
        groups[3] = (args.n + 31u) / 32u;
        groups[4] = 2u;
        groups[5] = 1u;
    }
}

// Each thread of the indirect launch marks its item, with the grid's size and its group as it sees them.
DEFINE_KERNEL2(device_tests, d_mark,
               ARGS,     Counts,   args, ,
               RWBUFFER, uint32_t, out,  "[n * 2]",
               RWBUFFER, uint32_t, seen, "[4]",
               GID0,     uint32_t, x,    ,
               GID1,     uint32_t, y,    ,
               GSZ0,     uint32_t, sx,   ,
               GSZ1,     uint32_t, sy,   ,
               TGID0,    uint32_t, g,    )
{
    if (x == 0u && y == 0u) {
        seen[0] = sx;
        seen[1] = sy;
    }
    ukl_atomic_max(&seen[2], g);
    if (x < args.n)
        out[y * args.n + x] += 1u;
}
"#;

/// An indirect launch takes as many groups as its counts say when it runs, written by the work before it on the
/// device, whatever bound the caller gives: each item of the grid the counts give is marked once, nothing past it,
/// and the kernel sees the counts' grid size (on CUDA and OpenCL the bound's other groups return first).
#[test]
fn an_indirect_launch_runs_the_groups_its_counts_give() {
    let Some(shared) = crate::shared::get() else { return };
    let p = program("device_tests_indirect.uglykl", INDIRECT).unwrap_or_else(|e| panic!("{e}"));
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Counts {
        n: u32,
        pad: u32,
    }
    unsafe impl crate::Pod for Counts {}
    const N: u32 = 1000;
    let groups = crate::kernels::buffer_of(&[0u32; 6]).unwrap();
    let out = crate::kernels::buffer_of(&vec![0u32; 2 * N as usize]).unwrap();
    let seen = crate::kernels::buffer_of(&[0u32; 4]).unwrap();
    let mut q = crate::device::ComputeQueue::new(&shared.queue);
    launch(&q, p.function("d_count").unwrap()).value("args", &Counts { n: N, pad: 0 }).buffer("groups", &groups).run(&[1], &[32]).unwrap();
    launch(&q, p.function("d_mark").unwrap())
        .value("args", &Counts { n: N, pad: 0 })
        .buffer("out", &out)
        .buffer("seen", &seen)
        .run_indirect(&groups, 12, &[64, 4, 1], &[32, 1, 1])
        .unwrap_or_else(|e| panic!("{e}"));
    q.flush().await_().unwrap();
    let marks: Vec<u32> = read(&out, 2 * N as usize);
    assert!(marks.iter().all(|&m| m == 1), "every item of the counts' grid once: {:?}", marks.iter().enumerate().filter(|(_, m)| **m != 1).take(4).collect::<Vec<_>>());
    let s: Vec<u32> = read(&seen, 4);
    assert_eq!((s[0], s[1]), (N.div_ceil(32) * 32, 2), "the grid's size is the counts'");
    assert_eq!(s[2], N.div_ceil(32) - 1, "no group past the counts ran");
}

const POINTERS: &str = r#"
#include <ukl/backend.h>

// A directory of tables, as a view's directory holds its tapes: each a pointer the host wrote
// (`Buffer::device_address`) and a length.
typedef struct Table {
    UKL_GLOBAL const uint32_t* words;
    uint32_t n;
    uint32_t pad;
} Table;

// Thread `i` sums table `i`'s words, following the pointer the directory holds.
DEFINE_KERNEL2(device_tests, d_follow,
               ROBUFFER, Table,    dir, "[2]",
               RWBUFFER, uint32_t, out, "[2]",
               GID0,     uint32_t, i,   )
{
    if (i < 2u) {
        uint32_t s = 0u;
        for (uint32_t k = 0u; k < dir[i].n; k++)
            s += dir[i].words[k];
        out[i] = s;
    }
}
"#;

/// A kernel follows a pointer a buffer holds (`Buffer::device_address`) to a buffer it uses (`uses`), as the view
/// directories point at their tapes. OpenCL 1.2 has no such pointer (`device_address` is `None`): not run there.
#[test]
fn a_kernel_follows_the_device_address_a_buffer_holds() {
    let Some(shared) = crate::shared::get() else { return };
    let a = crate::kernels::buffer_of(&(1..=100u32).collect::<Vec<_>>()).unwrap();
    let b = crate::kernels::buffer_of(&[7u32; 30]).unwrap();
    let (Some(pa), Some(pb)) = (a.device_address(), b.device_address()) else {
        assert_eq!(crate::device::backend(), "OpenCL", "only OpenCL has no device addresses");
        return;
    };
    let p = program("device_tests_pointers.uglykl", POINTERS).unwrap_or_else(|e| panic!("{e}"));
    let dir = crate::kernels::buffer_of(&[pa, 100, pb, 30]).unwrap();
    let out = crate::kernels::alloc::<u32>(2).unwrap();
    let mut q = crate::device::ComputeQueue::new(&shared.queue);
    launch(&q, p.function("d_follow").unwrap()).buffer("dir", &dir).buffer("out", &out).uses(&a).uses(&b).run(&[1], &[32]).unwrap();
    q.flush().await_().unwrap();
    assert_eq!(read::<u32>(&out, 2), [5050, 210]);
}

const SPECIALIZED: &str = r#"
#include <ukl/backend.h>

#ifndef SCALE
#  error "a specialization: SCALE is defined by the program's caller"
#endif

DEFINE_KERNEL2(device_tests, d_scale,
               RWBUFFER, uint32_t, out, "[32]",
               GID0,     uint32_t, i,   )
{
    out[i] = i * SCALE;
}
"#;

/// A program specialized by a value (`program_with`, Metal's function constants' portable form): each value its own
/// program, compiled once, the value folded into it.
#[test]
fn a_specialized_program_is_one_program_a_value() {
    let Some(shared) = crate::shared::get() else { return };
    let mut got = Vec::new();
    for scale in ["3u", "5u", "3u"] {
        let p = crate::kernels::program_with("device_tests_specialized.uglykl", SPECIALIZED, &[("SCALE", scale)]).unwrap_or_else(|e| panic!("{e}"));
        let out = crate::kernels::alloc::<u32>(32).unwrap();
        let mut q = crate::device::ComputeQueue::new(&shared.queue);
        launch(&q, p.function("d_scale").unwrap()).buffer("out", &out).run(&[1], &[32]).unwrap();
        q.flush().await_().unwrap();
        got.push((std::ptr::from_ref(p) as usize, read::<u32>(&out, 32)[31]));
    }
    assert_eq!(got.iter().map(|g| g.1).collect::<Vec<_>>(), [93, 155, 93]);
    assert!(got[0].0 == got[2].0 && got[0].0 != got[1].0, "one program a value, made once");
    assert!(crate::kernels::program("device_tests_specialized.uglykl", SPECIALIZED).is_err(), "unspecialized, it does not compile");
}

/// A block's device buffer (`rerail_blocks::Block::device_buffer`) is the block's own pages, on every backend: a
/// kernel's writes through it are in the block where it lies, at an allocation's offset, with no copy back
/// (decision 0353), and the wrap is made once while the block does not outgrow it.
#[test]
fn a_blocks_device_buffer_is_its_pages() {
    let Some(shared) = crate::shared::get() else { return };
    let p = program("device_tests.uglykl", KERNELS).unwrap_or_else(|e| panic!("{e}"));
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Args {
        n: u32,
        add: u32,
    }
    unsafe impl crate::Pod for Args {}
    let block = rerail_blocks::alloc::host(rerail_blocks::block::kind::SCRATCH, 4 << 20);
    const N: usize = 4096;
    let at = block.alloc(N * 4, 4096);
    // SAFETY: the allocation is this test's, `N` words at `at`.
    let words = unsafe { std::slice::from_raw_parts_mut(block.base_ptr().add(at).cast::<u32>(), N) };
    for (i, w) in words.iter_mut().enumerate() {
        *w = i as u32;
    }
    let b = block.device_buffer(&shared.device).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(b.contents().cast::<u8>(), block.base_ptr(), "the buffer is over the block's pages");
    let again = block.device_buffer(&shared.device).unwrap();
    assert_eq!(again.contents(), b.contents(), "the wrap is kept");
    let mut q = crate::device::ComputeQueue::new(&shared.queue);
    launch(&q, p.function("d_in_place").unwrap())
        .value("args", &Args { n: N as u32, add: 7 })
        .buffer_at("w", &b, at as u64)
        .run(&[(N / 256) as u64], &[256])
        .unwrap_or_else(|e| panic!("{e}"));
    q.flush().await_().unwrap();
    assert!(words.iter().enumerate().all(|(i, &w)| w == 2 * i as u32 + 7), "the kernel wrote the block where it lies");
}

/// A kernel compiled once is loaded from the store by the next compile of the same source (another process's, in
/// the engine: `rerail_device::store`), and runs; an entry that does not load is compiled again and replaced.
/// CUDA, OpenCL and the CPU (Metal's pipelines are in the engine's archive store until step C).
#[test]
fn a_kernel_compiled_once_is_loaded_from_the_store() {
    use crate::device::ComputeFunctionLibrary;
    use rerail_device::store;
    let Some(shared) = crate::shared::get() else { return };
    if crate::device::backend() == "Metal" || store::dir().is_none() {
        return;
    }
    // A source no earlier run compiled: the first compile is a compile.
    let nonce = format!("{}-{:?}", std::process::id(), std::time::SystemTime::now());
    let text = format!("// {nonce}\n{KERNELS}");
    let source = crate::kernels::preprocess("device_tests_store.uglykl", &text).unwrap();
    let run = |lib: &ComputeFunctionLibrary| {
        let f = Box::leak(Box::new(lib.get_function(&shared.device, "d_in_place").unwrap()));
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct Args {
            n: u32,
            add: u32,
        }
        unsafe impl crate::Pod for Args {}
        let b = crate::kernels::buffer_of(&[5u32; 64]).unwrap();
        let mut q = crate::device::ComputeQueue::new(&shared.queue);
        launch(&q, f).value("args", &Args { n: 64, add: 1 }).buffer("w", &b).run(&[1], &[64]).unwrap();
        q.flush().await_().unwrap();
        assert_eq!(read::<u32>(&b, 64), [11u32; 64]);
    };
    let compile = || {
        let before = store::here();
        let lib = ComputeFunctionLibrary::compile_from_source(&shared.device, &source, "device_tests_store.uglykl").unwrap_or_else(|e| panic!("{e}"));
        let after = store::here();
        (lib, (after.0 - before.0, after.1 - before.1))
    };
    let (first, counts) = compile();
    assert_eq!(counts, (0, 1), "compiled the first time");
    run(&first);
    // The entry the second compile loaded (`store::recording`: the entries a compile looked up).
    let ((second, counts), entries) = store::recording(compile);
    assert_eq!(counts, (1, 0), "loaded from the store the second time");
    run(&second);
    let entry = entries.last().cloned().expect("the entry");
    // The CPU's entry is a library this process has open: `dlopen` gives it again whatever the file holds now, so a
    // damaged one is a later process's to compile again. CUDA's and OpenCL's are bytes each compile loads.
    if shared.device.is_cpu() {
        let _ = std::fs::remove_file(&entry);
        let _ = std::fs::remove_dir_all(entry.with_extension("so.dSYM"));
        return;
    }
    std::fs::write(&entry, b"not a kernel").unwrap();
    let (third, counts) = compile();
    assert_eq!(counts, (0, 1), "a damaged entry is compiled again");
    run(&third);
    assert_ne!(std::fs::read(&entry).unwrap(), b"not a kernel", "and replaced");
    let _ = std::fs::remove_file(&entry);
}

/// Decision 0645 §4: a kernel compiled ahead of time for a target the device runs (`Target::runs`: its own, then
/// lower ones) is loaded from where it ships, and runs, where it was compiled from the device's own text; another
/// target's text is not taken.
#[test]
fn a_kernel_shipped_for_a_target_the_device_runs_is_loaded() {
    use crate::Preprocessing;
    let Some(shared) = crate::shared::get() else { return };
    let own = shared.device.target();
    if own.form().is_none() {
        return;
    }
    let name = "device_tests_ship.uglykl";
    let source = crate::kernels::preprocess(name, KERNELS).unwrap();
    let dir = std::env::temp_dir().join(format!("rerail-ship-test-{}-{}", std::process::id(), rerail_device::store::unique()));
    let run = |lib: &crate::device::ComputeFunctionLibrary| {
        let f = Box::leak(Box::new(lib.get_function(&shared.device, "d_in_place").unwrap()));
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct Args {
            n: u32,
            add: u32,
        }
        unsafe impl crate::Pod for Args {}
        let b = crate::kernels::buffer_of(&[5u32; 64]).unwrap();
        let mut q = crate::device::ComputeQueue::new(&shared.queue);
        launch(&q, f).value("args", &Args { n: 64, add: 1 }).buffer("w", &b).run(&[1], &[64]).unwrap();
        q.flush().await_().unwrap();
        assert_eq!(read::<u32>(&b, 64), [11u32; 64]);
    };
    assert!(crate::kernels::shipped(&shared.device, &dir, &source).is_none(), "nothing ships yet");
    for t in own.runs().iter().map(|n| rerail_ukl::Target::named(n).unwrap()) {
        // The target's own text, compiled for it, as a build ships it.
        let text = crate::kernels::preprocess_for(&t, name, KERNELS, &[]).unwrap();
        let version = match t.backend {
            rerail_ukl::Backend::Metal { version } => Some(version),
            _ => None,
        };
        // The device's own target compiles with its own compiler; a lower one may need a toolkit this machine lacks
        // (CUDA 13 compiles no sm_70: a build names a 12.x one; the HIP SDK compiles no generic AMD target).
        let code = match rerail_device::device::compile_offline(&t.name, version, &text, name) {
            Ok(code) => code,
            Err(e) if t != own && (e.contains("no NVRTC found compiles for it") || e.contains("cannot find ROCm device library")) => {
                eprintln!("{} under {}: not compiled here: {e}", t.name, own.name);
                continue;
            }
            Err(e) => panic!("{}: {e}", t.name),
        };
        let path = crate::kernels::shipped_path(&dir, &t, &text).unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &code).unwrap();
        let found = crate::kernels::shipped(&shared.device, &dir, &source);
        eprintln!("{} under {}: its text the device's: {}; taken: {}", t.name, own.name, text == source, found.is_some());
        assert_eq!(found.is_some(), text == source, "{}: taken exactly where its text is the device's", t.name);
        if let Some(lib) = found {
            run(&lib);
        }
        std::fs::remove_file(&path).unwrap();
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Decision 0645 step 3: every CUDA and AMD target a release ships compiles without a GPU, from its own text
/// (Windows' with its 32-bit `long`): CUDA's by the newest of the build's toolkits that compiles for it, where they
/// are named (`RERAIL_SHIP_CUDA_HOMES`, as the release job names NVIDIA's NVRTC 13 and 12); AMD's by hiprtc, where
/// ROCm or the HIP SDK is named (`ROCM_PATH`, `HIP_PATH`).
#[test]
fn every_shipped_cuda_and_amd_target_compiles_without_a_gpu() {
    let named = |v: &str| std::env::var_os(v).is_some_and(|v| !v.is_empty());
    let cuda = named("RERAIL_SHIP_CUDA_HOMES");
    let amd = named("ROCM_PATH") || named("HIP_PATH");
    let name = "device_tests_ship.uglykl";
    for n in rerail_ukl::target::SHIPPED {
        let t = rerail_ukl::Target::named(n).unwrap();
        let form = t.form().unwrap();
        if !(cuda && matches!(form, "cubin" | "ptx") || amd && form == "hsaco") {
            continue;
        }
        // The clock's text too: its CUDA form (PTX's `globaltimer`) once reached HIP.
        for source in [KERNELS, CLOCK] {
            let text = crate::kernels::preprocess_for(&t, name, source, &[]).unwrap();
            let code = rerail_device::device::compile_offline(n, None, &text, name).unwrap_or_else(|e| panic!("{n}: {e}"));
            let ok = if form == "ptx" { code.starts_with(b"//") } else { code.starts_with(b"\x7fELF") };
            assert!(ok, "{n}: {} bytes, not {form}", code.len());
            eprintln!("{n}: {} bytes", code.len());
        }
    }
}

const CLOCK: &str = r#"
#include <ukl/backend.h>

typedef struct Slot {
    uint32_t i;
    uint32_t pad;
} Slot;
UKL_REFLECT_STRUCT(Slot, i, pad)

// The device's clock (`ukl_clock_ns`), into the slot's two words, low first.
DEFINE_KERNEL2(device_tests, d_clock,
               ARGS,     Slot,     args, ,
               RWBUFFER, uint32_t, t,    "[4]",
               GID0,     uint32_t, g,    )
{
    if (g == 0u) {
        uint64_t now = ukl_clock_ns();
        t[2u * args.i] = (uint32_t)now;
        t[2u * args.i + 1u] = (uint32_t)(now >> 32);
    }
}
"#;

/// The device's clock (`ukl_clock_ns`) counts nanoseconds: two launches a measured host sleep apart read times as far
/// apart as the host's clock says, within the launches' own time. CUDA's and HIP's kernels have a clock; Metal's,
/// OpenCL's and the CPU's read 0.
#[test]
fn the_device_clock_counts_nanoseconds() {
    let Some(_) = crate::shared::get() else { return };
    let p = program("device_tests_clock.uglykl", CLOCK).unwrap_or_else(|e| panic!("{e}"));
    let f = p.function("d_clock").unwrap();
    let t = crate::kernels::buffer_of(&[0u32; 4]).unwrap();
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Slot {
        i: u32,
        pad: u32,
    }
    unsafe impl crate::Pod for Slot {}
    let run = |i: u32| {
        let mut q = crate::device::ComputeQueue::new(&crate::shared::get().unwrap().queue);
        launch(&q, f).value("args", &Slot { i, pad: 0 }).buffer("t", &t).run(&[1], &[1]).unwrap();
        q.flush().await_().unwrap();
    };
    run(0);
    let start = std::time::Instant::now();
    std::thread::sleep(std::time::Duration::from_millis(200));
    run(1);
    let host = start.elapsed().as_nanos() as u64;
    let w = read::<u32>(&t, 4);
    let (a, b) = (w[0] as u64 | (w[1] as u64) << 32, w[2] as u64 | (w[3] as u64) << 32);
    if !matches!(crate::device::backend(), "CUDA" | "HIP") {
        assert_eq!((a, b), (0, 0), "{} has no kernel clock", crate::device::backend());
        return;
    }
    let dt = b.saturating_sub(a);
    eprintln!("{}: the device's clock advanced {dt} ns over the host's {host} ns", crate::device::backend());
    assert!(dt >= 190_000_000 && dt <= host + 20_000_000, "{dt} ns on the device against {host} ns on the host");
}
