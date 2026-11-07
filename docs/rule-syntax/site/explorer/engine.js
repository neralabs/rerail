// One `rerail-engine` command over an in-memory filesystem: the browser build (native/wasm/README.md) under WASI
// preview 1, as `@bjorn3/browser_wasi_shim` provides it. `shim` is that module, passed in so the page (from the CDN)
// and a test (from a local copy) run the same code.
//
// A mount is a plain object: a name maps to a string (a text file), a Uint8Array (a file) or another object (a
// directory). After the run, every mount is read back the same way, so the caller sees what the engine wrote.

const encoder = new TextEncoder();

function toTree(shim, entries) {
  const map = new Map();
  for (const [name, value] of Object.entries(entries)) {
    if (typeof value === "string") map.set(name, new shim.File(encoder.encode(value)));
    else if (value instanceof Uint8Array) map.set(name, new shim.File(value));
    else map.set(name, new shim.Directory(toTree(shim, value)));
  }
  return map;
}

function fromTree(shim, directory) {
  const entries = {};
  for (const [name, inode] of directory.contents) {
    entries[name] = inode instanceof shim.Directory ? fromTree(shim, inode) : inode.data;
  }
  return entries;
}

// Runs `args` (after the program name) with `env` and `mounts` ({"/w": {...}}), on a fresh instance of `module`
// (a compiled WebAssembly.Module): each run starts from clean memory. Returns the exit code, stdout and stderr lines,
// the run's wall time and the mounts as the run left them.
export async function command(shim, module, args, env, mounts) {
  const outBytes = [], errBytes = [];
  const collect = (chunks) => new shim.ConsoleStdout((bytes) => chunks.push(bytes.slice()));
  const lines = (chunks) => {
    const all = new Uint8Array(chunks.reduce((n, c) => n + c.length, 0));
    let at = 0;
    for (const c of chunks) { all.set(c, at); at += c.length; }
    const text = new TextDecoder().decode(all);
    return text === "" ? [] : text.replace(/\n$/, "").split("\n");
  };
  const preopens = Object.entries(mounts).map(([path, entries]) => new shim.PreopenDirectory(path, toTree(shim, entries)));
  const fds = [
    new shim.OpenFile(new shim.File([])),
    collect(outBytes),
    collect(errBytes),
    ...preopens,
  ];
  const wasi = new shim.WASI(["rerail-engine", ...args], Object.entries(env).map(([k, v]) => `${k}=${v}`), fds);
  // The in-memory filesystem has no symbolic links, so readlink answers what a disk does for a file that is not one
  // (EINVAL), rather than the shim's "not supported": libc's realpath, behind Rust's canonicalize, reads every
  // component's link and stops at the first error that is not EINVAL.
  const ERRNO_INVAL = 28;
  wasi.wasiImport.path_readlink = () => ERRNO_INVAL;
  // The few `napi_*` imports still linked come from napi types' drops, which a CLI run never calls.
  const imports = { wasi_snapshot_preview1: wasi.wasiImport };
  for (const wanted of WebAssembly.Module.imports(module)) {
    if (wanted.module === "wasi_snapshot_preview1" || wanted.kind !== "function") continue;
    (imports[wanted.module] ??= {})[wanted.name] = () => {
      throw new Error(`the engine called ${wanted.module}.${wanted.name}, which the browser build does not provide`);
    };
  }
  const instance = await WebAssembly.instantiate(module, imports);
  const started = performance.now();
  let code;
  try {
    code = wasi.start(instance);
  } catch (error) {
    // A trap (a Rust panic aborts) or an import above: the run stopped; what it wrote so far is still read back.
    errBytes.push(encoder.encode(`\n${error && error.message ? error.message : error}\n`));
    code = -1;
  }
  const ms = performance.now() - started;
  const after = {};
  for (const preopen of preopens) after[preopen.prestat_name] = fromTree(shim, preopen.dir);
  return { code, stdout: lines(outBytes), stderr: lines(errBytes), ms, mounts: after };
}

// Files named by path ("python/ast.rr") as the nested directories a mount takes.
export function nest(flat) {
  const root = {};
  for (const [path, text] of Object.entries(flat)) {
    const parts = path.split("/");
    let dir = root;
    for (const part of parts.slice(0, -1)) dir = dir[part] ??= {};
    dir[parts[parts.length - 1]] = text;
  }
  return root;
}

// A rule run as the explorer shows it: the rule (with its part files) beside its includes in /lib, the sources under
// /w/src, outputs under /w/out and the run's profile under /w/prof; then the program it ran, flattened.
export async function explore(shim, module, { id, rr, parts, sources, lib: libFiles }) {
  const lib = nest(libFiles);
  const rule = { [`${id}.rr`]: rr, ...parts };
  const env = { RERAIL_RR_LIB: "/lib", RERAIL_PROFILE: "/w/prof" };
  const run = await command(shim, module, ["logic", `/w/rule/${id}.rr`, "/w/src", "/w/out"], env,
    { "/w": { rule, src: sources, out: {}, prof: {} }, "/lib": lib });
  const flat = await command(shim, module, ["flatten", `/w/rule/${id}.rr`], { RERAIL_RR_LIB: "/lib" },
    { "/w": { rule }, "/lib": lib });
  const w = run.mounts["/w"];
  const text = (bytes) => (bytes instanceof Uint8Array ? new TextDecoder().decode(bytes) : "");
  const out = {};
  for (const [name, bytes] of Object.entries(w.out || {})) {
    if (name.endsWith(".csv")) out[name.slice(0, -4)] = text(bytes);
  }
  const prof = {};
  for (const [name, bytes] of Object.entries(w.prof || {})) prof[name] = text(bytes);
  return {
    code: run.code, ms: run.ms, report: run.stdout.join("\n"), error: run.stderr.join("\n"),
    relations: out, profile: prof,
    program: flat.code === 0 ? flat.stdout.join("\n") : "", programError: flat.stderr.join("\n"),
  };
}
