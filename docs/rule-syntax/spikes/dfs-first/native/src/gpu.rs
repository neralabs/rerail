//! Steps 1-3 (`live`, `next`, `first_of`) as Metal waves: gpu/dfs_gpu.mm, linked by build.rs.
//! `live` is one wave per level over the reverse CSR, with the next frontier appended through an
//! atomic counter and dispatched indirectly; `next` is one thread per node; `first_of` is pointer
//! jumping. The host does not wait between the waves of a batch.

use crate::{Graph, NONE, Prep};

unsafe extern "C" {
    fn dfs_gpu_init(name: *mut u8, cap: i32) -> i32;
    #[allow(clippy::too_many_arguments)]
    fn dfs_gpu_prepare(n: u32, m: u32, off: *const u32, dst: *const u32, roff: *const u32, rsrc: *const u32,
                       sink: *const u32, live: *mut u32, nxt: *mut u32, first: *mut u32, times: *mut f64) -> i32;
}

/// The device name, or None if Metal is unavailable.
pub(crate) fn init() -> Option<String> {
    let mut name = [0u8; 256];
    if unsafe { dfs_gpu_init(name.as_mut_ptr(), name.len() as i32) } != 0 {
        return None;
    }
    let end = name.iter().position(|&b| b == 0).unwrap_or(0);
    Some(String::from_utf8_lossy(&name[..end]).into_owned())
}

pub(crate) struct GpuOut {
    pub(crate) live: Vec<bool>,
    pub(crate) nxt: Vec<u32>,
    pub(crate) first: Vec<u32>,
    pub(crate) gpu_seconds: f64,
    pub(crate) waves: u32,
    pub(crate) command_buffers: u32,
}

pub(crate) fn prepare(g: &Graph) -> GpuOut {
    let n = g.n;
    let (roff, rsrc) = crate::par::reverse(g);
    let off: Vec<u32> = g.off.iter().map(|&x| x as u32).collect();
    let roff: Vec<u32> = roff.iter().map(|&x| x as u32).collect();
    let sink: Vec<u32> = g.sink.iter().map(|s| s.is_some() as u32).collect();
    let (mut live, mut nxt, mut first) = (vec![0u32; n], vec![NONE; n], vec![NONE; n]);
    let mut times = [0f64; 3];
    let rc = unsafe {
        dfs_gpu_prepare(n as u32, g.dst.len() as u32, off.as_ptr(), g.dst.as_ptr(), roff.as_ptr(), rsrc.as_ptr(),
                        sink.as_ptr(), live.as_mut_ptr(), nxt.as_mut_ptr(), first.as_mut_ptr(), times.as_mut_ptr())
    };
    assert_eq!(rc, 0, "dfs_gpu_prepare failed: {rc}");
    GpuOut {
        live: live.into_iter().map(|x| x != 0).collect(),
        nxt,
        first,
        gpu_seconds: times[0],
        waves: times[1] as u32,
        command_buffers: times[2] as u32,
    }
}

/// Where the GPU arrays differ from the CPU's, if anywhere.
pub(crate) fn differs(o: &GpuOut, p: &Prep) -> Option<String> {
    for (name, ok) in [("live", o.live == p.live), ("next", o.nxt == p.nxt), ("first_of", o.first == p.first)] {
        if !ok {
            return Some(name.to_string());
        }
    }
    None
}
