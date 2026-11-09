#![allow(unexpected_cfgs)] // objc 0.2's msg_send! tests a cfg the compiler does not know.

//! Core ML from Rust, through the Objective-C runtime: compile a package, check where its operations run, load it
//! for the CPU and the Neural Engine, and run a batch of predictions on fp16 arrays.
//!
//! Every call runs inside an autorelease pool. Objects kept across calls are retained ([`Obj`]) and released on
//! drop.

use objc::rc::autoreleasepool;
use objc::runtime::{Class, Object, BOOL, YES};
use objc::{class, msg_send, sel, sel_impl};
use std::ffi::{c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[link(name = "CoreML", kind = "framework")]
unsafe extern "C" {}
#[link(name = "Foundation", kind = "framework")]
unsafe extern "C" {}

type CvBuffer = *mut c_void;

#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    static kCVPixelBufferIOSurfacePropertiesKey: Id;
    fn CVPixelBufferCreate(alloc: *const c_void, width: usize, height: usize, format: u32, attrs: Id, out: *mut CvBuffer) -> i32;
    fn CVPixelBufferLockBaseAddress(pb: CvBuffer, flags: u64) -> i32;
    fn CVPixelBufferUnlockBaseAddress(pb: CvBuffer, flags: u64) -> i32;
    fn CVPixelBufferGetBaseAddress(pb: CvBuffer) -> *mut c_void;
    fn CVPixelBufferGetBytesPerRow(pb: CvBuffer) -> usize;
}
#[link(name = "IOSurface", kind = "framework")]
unsafe extern "C" {
    static kIOSurfaceWidth: Id;
    static kIOSurfaceHeight: Id;
    fn IOSurfaceGetPropertyMaximum(property: Id) -> usize;
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *const c_void);
}

/// `kCVPixelFormatType_OneComponent16Half` (`'L00h'`): one fp16 a pixel.
const ONE_COMPONENT_16_HALF: u32 = u32::from_be_bytes(*b"L00h");
/// `kCVPixelBufferLock_ReadOnly`.
const LOCK_READ_ONLY: u64 = 1;

type Id = *mut Object;

/// `MLComputeUnitsCPUAndNeuralEngine`.
const CPU_AND_NE: isize = 3;
/// `MLMultiArrayDataTypeFloat16`.
const FLOAT16: isize = 0x10000 | 16;

/// A retained Objective-C object.
pub struct Obj(Id);

// Core ML's models and configurations may be used from any thread.
unsafe impl Send for Obj {}
unsafe impl Sync for Obj {}

impl Obj {
    /// Retains `id` (which must not be nil).
    fn retain(id: Id) -> Obj {
        assert!(!id.is_null());
        let id: Id = unsafe { msg_send![id, retain] };
        Obj(id)
    }
}

impl Drop for Obj {
    fn drop(&mut self) {
        let _: () = unsafe { msg_send![self.0, release] };
    }
}

fn nsstring(s: &str) -> Id {
    let c = CString::new(s).expect("no NUL in a Core ML string");
    unsafe { msg_send![class!(NSString), stringWithUTF8String: c.as_ptr()] }
}

fn string(ns: Id) -> String {
    if ns.is_null() {
        return String::new();
    }
    let p: *const std::os::raw::c_char = unsafe { msg_send![ns, UTF8String] };
    if p.is_null() { String::new() } else { unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned() }
}

fn error(err: Id, what: &str) -> String {
    if err.is_null() {
        return format!("{what}: failed without an error");
    }
    let d: Id = unsafe { msg_send![err, localizedDescription] };
    format!("{what}: {}", string(d))
}

fn file_url(p: &Path) -> Id {
    unsafe { msg_send![class!(NSURL), fileURLWithPath: nsstring(&p.to_string_lossy())] }
}

fn number(v: i64) -> Id {
    unsafe { msg_send![class!(NSNumber), numberWithLongLong: v] }
}

fn array(items: &[Id]) -> Id {
    unsafe { msg_send![class!(NSArray), arrayWithObjects: items.as_ptr() count: items.len()] }
}

fn ints(ns: Id) -> Vec<i64> {
    let n: usize = unsafe { msg_send![ns, count] };
    (0..n)
        .map(|i| {
            let x: Id = unsafe { msg_send![ns, objectAtIndex: i] };
            unsafe { msg_send![x, longLongValue] }
        })
        .collect()
}

/// Compiles a `.mlpackage` (`MLModel compileModelAtURL:error:`); returns the compiled `.mlmodelc` directory, in a
/// temporary place of Core ML's choosing (the caller moves or removes it).
pub fn compile(package: &Path) -> Result<PathBuf, String> {
    autoreleasepool(|| {
        let mut err: Id = std::ptr::null_mut();
        let url: Id = unsafe { msg_send![class!(MLModel), compileModelAtURL: file_url(package) error: &mut err] };
        if url.is_null() {
            return Err(error(err, "compiling the model"));
        }
        let p: Id = unsafe { msg_send![url, path] };
        Ok(PathBuf::from(string(p)))
    })
}

/// A configuration for the CPU and the Neural Engine.
pub fn config() -> Obj {
    autoreleasepool(|| {
        let c: Id = unsafe { msg_send![class!(MLModelConfiguration), new] };
        let _: () = unsafe { msg_send![c, setComputeUnits: CPU_AND_NE] };
        let o = Obj::retain(c);
        let _: () = unsafe { msg_send![c, release] };
        o
    })
}

/// Checks that every computing operation of a compiled model runs on the Neural Engine (`MLComputePlan`'s
/// preferred device). Constants and the constants' expansions (`const`, `constexpr_*`) are folded into the ops that
/// read them and have no device. `Err` names each operation placed elsewhere.
pub fn check_placement(compiled: &Path, cfg: &Obj) -> Result<(), String> {
    let (tx, rx) = std::sync::mpsc::channel::<Result<usize, String>>();
    autoreleasepool(|| {
        let tx = std::sync::Mutex::new(Some(tx));
        let handler = block::ConcreteBlock::new(move |plan: Id, err: Id| {
            let r = if plan.is_null() {
                Err(error(err, "loading the compute plan"))
            } else {
                let p: Id = unsafe { msg_send![plan, retain] };
                Ok(p as usize)
            };
            if let Some(tx) = tx.lock().unwrap().take() {
                let _ = tx.send(r);
            }
        })
        .copy();
        let _: () = unsafe {
            msg_send![class!(MLComputePlan), loadContentsOfURL: file_url(compiled) configuration: cfg.0 completionHandler: &*handler]
        };
    });
    let plan = rx.recv_timeout(Duration::from_secs(120)).map_err(|_| "the compute plan did not load within 120 s".to_string())??;
    let plan = Obj(plan as Id);
    autoreleasepool(|| {
        let structure: Id = unsafe { msg_send![plan.0, modelStructure] };
        let program: Id = unsafe { msg_send![structure, program] };
        if program.is_null() {
            return Err("the compiled model is not an ML program".into());
        }
        let funcs: Id = unsafe { msg_send![program, functions] };
        let main: Id = unsafe { msg_send![funcs, objectForKey: nsstring("main")] };
        if main.is_null() {
            return Err("the compiled model has no main function".into());
        }
        let ane = Class::get("MLNeuralEngineComputeDevice").ok_or("Core ML has no Neural Engine device class")?;
        let mut off = Vec::new();
        let mut on = 0;
        let mut stack: Vec<Id> = vec![unsafe { msg_send![main, block] }];
        while let Some(b) = stack.pop() {
            let ops: Id = unsafe { msg_send![b, operations] };
            let n: usize = unsafe { msg_send![ops, count] };
            for i in 0..n {
                let op: Id = unsafe { msg_send![ops, objectAtIndex: i] };
                let name = string(unsafe { msg_send![op, operatorName] });
                let blocks: Id = unsafe { msg_send![op, blocks] };
                let nb: usize = unsafe { msg_send![blocks, count] };
                for j in 0..nb {
                    stack.push(unsafe { msg_send![blocks, objectAtIndex: j] });
                }
                // Names carry their opset (`ios18.matmul`).
                let base = name.rsplit('.').next().unwrap_or(&name);
                if base == "const" || base.starts_with("constexpr_") {
                    continue;
                }
                let usage: Id = unsafe { msg_send![plan.0, computeDeviceUsageForMLProgramOperation: op] };
                let dev: Id = if usage.is_null() { std::ptr::null_mut() } else { unsafe { msg_send![usage, preferredComputeDevice] } };
                let is_ane: BOOL = if dev.is_null() { objc::runtime::NO } else { unsafe { msg_send![dev, isKindOfClass: ane] } };
                if is_ane == YES {
                    on += 1;
                } else {
                    let d = if dev.is_null() { "no device".to_string() } else { string(unsafe { msg_send![dev, description] }) };
                    off.push(format!("{name} on {d}"));
                }
            }
        }
        if !off.is_empty() {
            return Err(format!("Core ML would not run every operation on the Neural Engine: {}", off.join(", ")));
        }
        if on == 0 {
            return Err("the compute plan has no operation on the Neural Engine".into());
        }
        Ok(())
    })
}

/// Loads a compiled model.
pub fn load(compiled: &Path, cfg: &Obj) -> Result<Obj, String> {
    autoreleasepool(|| {
        let mut err: Id = std::ptr::null_mut();
        let m: Id = unsafe { msg_send![class!(MLModel), modelWithContentsOfURL: file_url(compiled) configuration: cfg.0 error: &mut err] };
        if m.is_null() {
            return Err(error(err, "loading the model"));
        }
        Ok(Obj::retain(m))
    })
}

/// One fp16 input: its feature name, the buffer, its rows and columns (rows contiguous).
pub struct Input<'a> {
    pub name: String,
    pub data: &'a mut [u16],
    pub rows: usize,
    pub cols: usize,
}

/// Runs one prediction per item (`predictionsFromBatch:`), every item's inputs given; for each item, each output
/// in `outputs` is handed to `read` as `(item, output index, data, rows, cols, row stride)` (strides in elements).
/// Returns the wall seconds of the prediction call alone, and the process's CPU (user + sys) over it.
pub fn predict(
    model: &Obj,
    items: &mut [Vec<Input<'_>>],
    outputs: &[String],
    mut read: impl FnMut(usize, usize, &[u16], usize, usize, usize) -> Result<(), String>,
) -> Result<(f64, f64), String> {
    autoreleasepool(|| {
        let mut providers = Vec::with_capacity(items.len());
        for item in items.iter_mut() {
            let dict: Id = unsafe { msg_send![class!(NSMutableDictionary), dictionary] };
            for inp in item.iter_mut() {
                assert_eq!(inp.data.len(), inp.rows * inp.cols);
                let shape = array(&[number(inp.rows as i64), number(inp.cols as i64)]);
                let strides = array(&[number(inp.cols as i64), number(1)]);
                let mut err: Id = std::ptr::null_mut();
                let alloc: Id = unsafe { msg_send![class!(MLMultiArray), alloc] };
                let arr: Id = unsafe {
                    msg_send![alloc, initWithDataPointer: inp.data.as_mut_ptr() as *mut c_void shape: shape dataType: FLOAT16
                        strides: strides deallocator: std::ptr::null::<c_void>() error: &mut err]
                };
                if arr.is_null() {
                    return Err(error(err, "making an input array"));
                }
                let fv: Id = unsafe { msg_send![class!(MLFeatureValue), featureValueWithMultiArray: arr] };
                let _: () = unsafe { msg_send![arr, release] };
                let _: () = unsafe { msg_send![dict, setObject: fv forKey: nsstring(&inp.name)] };
            }
            let mut err: Id = std::ptr::null_mut();
            let alloc: Id = unsafe { msg_send![class!(MLDictionaryFeatureProvider), alloc] };
            let p: Id = unsafe { msg_send![alloc, initWithDictionary: dict error: &mut err] };
            if p.is_null() {
                return Err(error(err, "making a feature provider"));
            }
            providers.push(p);
        }
        let list = array(&providers);
        for p in providers {
            let _: () = unsafe { msg_send![p, release] };
        }
        let alloc: Id = unsafe { msg_send![class!(MLArrayBatchProvider), alloc] };
        let batch: Id = unsafe { msg_send![alloc, initWithFeatureProviderArray: list] };
        let mut err: Id = std::ptr::null_mut();
        let t0 = std::time::Instant::now();
        let c0 = crate::exec::process_cpu();
        let out: Id = unsafe { msg_send![model.0, predictionsFromBatch: batch error: &mut err] };
        let cpu = crate::exec::process_cpu() - c0;
        let wall = t0.elapsed().as_secs_f64();
        let _: () = unsafe { msg_send![batch, release] };
        if out.is_null() {
            return Err(error(err, "predicting"));
        }
        let n: isize = unsafe { msg_send![out, count] };
        if n as usize != items.len() {
            return Err(format!("predicting: {n} results for {} items", items.len()));
        }
        for i in 0..items.len() {
            let fp: Id = unsafe { msg_send![out, featuresAtIndex: i as isize] };
            for (o, name) in outputs.iter().enumerate() {
                let fv: Id = unsafe { msg_send![fp, featureValueForName: nsstring(name)] };
                if fv.is_null() {
                    return Err(format!("predicting: no output {name}"));
                }
                let ma: Id = unsafe { msg_send![fv, multiArrayValue] };
                if ma.is_null() {
                    return Err(format!("predicting: output {name} is not an array"));
                }
                let dt: isize = unsafe { msg_send![ma, dataType] };
                let shape = ints(unsafe { msg_send![ma, shape] });
                let strides = ints(unsafe { msg_send![ma, strides] });
                if dt != FLOAT16 || shape.len() != 2 || strides.len() != 2 || strides[1] != 1 {
                    return Err(format!("predicting: output {name} is not a 2-d fp16 array with unit column stride (type {dt:#x}, shape {shape:?}, strides {strides:?})"));
                }
                let (rows, cols, stride) = (shape[0] as usize, shape[1] as usize, strides[0] as usize);
                let ptr: *const u16 = unsafe { msg_send![ma, dataPointer] };
                let len = if rows == 0 { 0 } else { (rows - 1) * stride + cols };
                let data = unsafe { std::slice::from_raw_parts(ptr, len) };
                read(i, o, data, rows, cols, stride)?;
            }
        }
        Ok((wall, cpu))
    })
}

// ---------------------------------------------------------------------------------------------------------------
// IOSurface-backed arrays.
//
// An array that Core ML is handed as plain memory is copied, on the host, into memory the Neural Engine can read,
// at every prediction (and its outputs copied back out). An array backed by an IOSurface is that memory already:
// the host writes the inputs into it in place, the engine reads them there, and writes its outputs into a surface
// given as the output's backing (`MLPredictionOptions.outputBackings`). The spike measured host CPU a call falling
// about 4× with them (`tools/matrix-offload/ane/RESULTS-ane.md`, "IOSurface I/O").

/// Whether an fp16 array of `rows` × `cols` can be IOSurface-backed: a surface is one pixel per element, `cols`
/// wide and `rows` high, within IOSurface's guaranteed maxima.
pub fn surface_fits(rows: usize, cols: usize) -> bool {
    let (w, h) = unsafe { (IOSurfaceGetPropertyMaximum(kIOSurfaceWidth), IOSurfaceGetPropertyMaximum(kIOSurfaceHeight)) };
    rows > 0 && cols > 0 && cols <= w && rows <= h
}

/// An fp16 matrix in an IOSurface (a `CVPixelBuffer` of `OneComponent16Half`), and the `MLMultiArray` over it.
/// Rows are `stride` elements apart: the surface pads each row to its alignment.
pub struct Surface {
    pb: CvBuffer,
    array: Obj,
    pub rows: usize,
    pub cols: usize,
    pub stride: usize,
}

// The buffer is written only between `write`'s lock and unlock, by one owner at a time (the pool hands it out).
unsafe impl Send for Surface {}
unsafe impl Sync for Surface {}

impl Surface {
    /// A new surface of `rows` × `cols` fp16. `Err` names the IOSurface that could not be made.
    pub fn new(rows: usize, cols: usize) -> Result<Surface, String> {
        autoreleasepool(|| {
            let empty: Id = unsafe { msg_send![class!(NSDictionary), dictionary] };
            let attrs: Id = unsafe { msg_send![class!(NSDictionary), dictionaryWithObject: empty forKey: kCVPixelBufferIOSurfacePropertiesKey] };
            let mut pb: CvBuffer = std::ptr::null_mut();
            let st = unsafe { CVPixelBufferCreate(std::ptr::null(), cols, rows, ONE_COMPONENT_16_HALF, attrs, &mut pb) };
            if st != 0 || pb.is_null() {
                return Err(format!("creating an IOSurface pixel buffer of {rows} x {cols} fp16: CVPixelBufferCreate returned {st}"));
            }
            let bytes = unsafe { CVPixelBufferGetBytesPerRow(pb) };
            let shape = array(&[number(rows as i64), number(cols as i64)]);
            let alloc: Id = unsafe { msg_send![class!(MLMultiArray), alloc] };
            // The array retains the buffer; this keeps its own reference too, released on drop.
            let arr: Id = unsafe { msg_send![alloc, initWithPixelBuffer: pb shape: shape] };
            if arr.is_null() {
                unsafe { CFRelease(pb) };
                return Err(format!("wrapping an IOSurface of {rows} x {cols} fp16 as an MLMultiArray"));
            }
            let strides = ints(unsafe { msg_send![arr, strides] });
            let stride = bytes / 2;
            if strides != [stride as i64, 1] {
                let _: () = unsafe { msg_send![arr, release] };
                unsafe { CFRelease(pb) };
                return Err(format!("an IOSurface of {rows} x {cols} fp16: strides {strides:?}, expected [{stride}, 1]"));
            }
            Ok(Surface { pb, array: Obj(arr), rows, cols, stride })
        })
    }

    /// Bytes the surface holds.
    pub fn bytes(&self) -> usize {
        self.rows * self.stride * 2
    }

    /// Writes the surface in place: `f` gets its elements, `rows × stride` of them, row `r` at `r * stride`.
    pub fn write<R>(&self, f: impl FnOnce(&mut [u16]) -> R) -> Result<R, String> {
        self.locked(0, |p| f(unsafe { std::slice::from_raw_parts_mut(p, self.rows * self.stride) }))
    }

    /// Reads the surface: `f` gets its elements as `write`'s.
    pub fn read<R>(&self, f: impl FnOnce(&[u16]) -> R) -> Result<R, String> {
        self.locked(LOCK_READ_ONLY, |p| f(unsafe { std::slice::from_raw_parts(p, self.rows * self.stride) }))
    }

    fn locked<R>(&self, flags: u64, f: impl FnOnce(*mut u16) -> R) -> Result<R, String> {
        let st = unsafe { CVPixelBufferLockBaseAddress(self.pb, flags) };
        if st != 0 {
            return Err(format!("locking an IOSurface of {} x {}: {st}", self.rows, self.cols));
        }
        let p = unsafe { CVPixelBufferGetBaseAddress(self.pb) } as *mut u16;
        let r = if p.is_null() { None } else { Some(f(p)) };
        unsafe { CVPixelBufferUnlockBaseAddress(self.pb, flags) };
        r.ok_or_else(|| format!("an IOSurface of {} x {} has no base address", self.rows, self.cols))
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        // This reference; the array (dropped next) holds its own.
        unsafe { CFRelease(self.pb) };
    }
}

/// Runs one prediction on IOSurface-backed arrays: `inputs` by feature name, written already, and `outputs` by
/// feature name as the outputs' backings, which Core ML writes. Returns the wall seconds of the prediction call, and
/// the process's CPU (user + sys) over it.
pub fn predict_surfaces(model: &Obj, inputs: &[(&str, &Surface)], outputs: &[(&str, &Surface)]) -> Result<(f64, f64), String> {
    autoreleasepool(|| {
        let dict: Id = unsafe { msg_send![class!(NSMutableDictionary), dictionary] };
        for (name, s) in inputs {
            let fv: Id = unsafe { msg_send![class!(MLFeatureValue), featureValueWithMultiArray: s.array.0] };
            let _: () = unsafe { msg_send![dict, setObject: fv forKey: nsstring(name)] };
        }
        let mut err: Id = std::ptr::null_mut();
        let alloc: Id = unsafe { msg_send![class!(MLDictionaryFeatureProvider), alloc] };
        let provider: Id = unsafe { msg_send![alloc, initWithDictionary: dict error: &mut err] };
        if provider.is_null() {
            return Err(error(err, "making a feature provider"));
        }
        let backs: Id = unsafe { msg_send![class!(NSMutableDictionary), dictionary] };
        for (name, s) in outputs {
            let _: () = unsafe { msg_send![backs, setObject: s.array.0 forKey: nsstring(name)] };
        }
        let opts: Id = unsafe { msg_send![class!(MLPredictionOptions), new] };
        let _: () = unsafe { msg_send![opts, setOutputBackings: backs] };
        let t0 = std::time::Instant::now();
        let c0 = crate::exec::process_cpu();
        let out: Id = unsafe { msg_send![model.0, predictionFromFeatures: provider options: opts error: &mut err] };
        let cpu = crate::exec::process_cpu() - c0;
        let wall = t0.elapsed().as_secs_f64();
        let _: () = unsafe { msg_send![provider, release] };
        let _: () = unsafe { msg_send![opts, release] };
        if out.is_null() {
            return Err(error(err, "predicting on IOSurfaces"));
        }
        // Each output must be the backing it was given: Core ML wrote into the surface, not somewhere else.
        for (name, s) in outputs {
            let fv: Id = unsafe { msg_send![out, featureValueForName: nsstring(name)] };
            let ma: Id = if fv.is_null() { std::ptr::null_mut() } else { unsafe { msg_send![fv, multiArrayValue] } };
            let pb: CvBuffer = if ma.is_null() { std::ptr::null_mut() } else { unsafe { msg_send![ma, pixelBuffer] } };
            if pb != s.pb {
                return Err(format!("predicting on IOSurfaces: output {name} was not written into its backing"));
            }
        }
        Ok((wall, cpu))
    })
}
