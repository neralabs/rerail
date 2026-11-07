// `rerail-engine` commands over an in-memory filesystem: the browser build (native/wasm/README.md) under WASI
// preview 1, as `@bjorn3/browser_wasi_shim` provides it. `shim` is that module, passed in so the page (served beside the page)
// and a test (from a local copy) run the same code.
//
// A mount is a plain object: a name maps to a string (a text file), a Uint8Array (a file) or another object (a
// directory). After a run, a directory is read back the same way, so the caller sees what the engine wrote.

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

// The engine, instantiated once (native/wasm/README.md, "A long-lived engine": the reactor build) and called once per
// command line, so what one run leaves the next finds: the rule programs it read, and its corpus session (decision
// 0056: the store, the program compiled, each file's file-local rows), so a run after an edit evaluates what the
// edit changed. Its filesystem is its own for its life: `/w` (the run's rule and files), `/lib` (the rule library)
// and `/s` (the session's store); `mount` replaces a directory's contents and `read` reads them back.
export class Engine {
  constructor(shim, module, env) {
    this.shim = shim;
    this.module = module;
    this.out = [];
    this.err = [];
    this.dirs = { "/w": new shim.PreopenDirectory("/w", new Map()), "/lib": new shim.PreopenDirectory("/lib", new Map()), "/s": new shim.PreopenDirectory("/s", new Map()) };
    const fds = [
      new shim.OpenFile(new shim.File([])),
      new shim.ConsoleStdout((bytes) => this.out.push(bytes.slice())),
      new shim.ConsoleStdout((bytes) => this.err.push(bytes.slice())),
      ...Object.values(this.dirs),
    ];
    this.wasi = new shim.WASI(["rerail-engine"], Object.entries({ RERAIL_SESSION: "/s", ...env }).map(([k, v]) => `${k}=${v}`), fds);
    // The in-memory filesystem has no symbolic links, so readlink answers what a disk does for a file that is not
    // one (EINVAL), rather than the shim's "not supported": libc's realpath, behind Rust's canonicalize, reads every
    // component's link and stops at the first error that is not EINVAL.
    const ERRNO_INVAL = 28;
    this.wasi.wasiImport.path_readlink = () => ERRNO_INVAL;
    this.instance = null;
    this.dead = false;
  }

  async start() {
    // The few `napi_*` imports still linked come from napi types' drops, which a command never calls.
    const imports = { wasi_snapshot_preview1: this.wasi.wasiImport };
    for (const wanted of WebAssembly.Module.imports(this.module)) {
      if (wanted.module === "wasi_snapshot_preview1" || wanted.kind !== "function") continue;
      (imports[wanted.module] ??= {})[wanted.name] = () => {
        throw new Error(`the engine called ${wanted.module}.${wanted.name}, which the browser build does not provide`);
      };
    }
    this.instance = await WebAssembly.instantiate(this.module, imports);
    if (!this.instance.exports.rerail_command) throw new Error("this engine build is not the long-lived (reactor) build");
    this.wasi.initialize(this.instance);
    return this;
  }

  // Replaces the contents of `dir` ("/w" or "/lib") with `entries`.
  mount(dir, entries) {
    const d = this.dirs[dir].dir;
    d.contents = toTree(this.shim, entries);
    for (const entry of d.contents.values()) if (entry instanceof this.shim.Directory) entry.parent = d;
  }

  read(dir) {
    return fromTree(this.shim, this.dirs[dir].dir);
  }

  // Runs `args` (after the program name). Returns the exit code, stdout and stderr lines and the run's wall time. A
  // trap leaves the instance unusable (`dead`): the caller starts another.
  command(args) {
    this.out = [];
    this.err = [];
    const e = this.instance.exports;
    const line = encoder.encode(args.map((a) => a + "\0").join(""));
    const at = e.rerail_alloc(line.length);
    new Uint8Array(e.memory.buffer, at, line.length).set(line);
    const started = performance.now();
    let code;
    try {
      code = e.rerail_command(at, line.length);
      e.rerail_free(at, line.length);
    } catch (error) {
      this.dead = true;
      this.err.push(encoder.encode(`\n${error && error.message ? error.message : error}\n`));
      code = -1;
    }
    const ms = performance.now() - started;
    return { code, stdout: lines(this.out), stderr: lines(this.err), ms };
  }
}

function lines(chunks) {
  const all = new Uint8Array(chunks.reduce((n, c) => n + c.length, 0));
  let at = 0;
  for (const c of chunks) { all.set(c, at); at += c.length; }
  const text = new TextDecoder().decode(all);
  return text === "" ? [] : text.replace(/\n$/, "").split("\n");
}

// Downloads and compiles the engine, telling `progress` how far it got: ("download", bytes so far, bytes in all),
// then ("compile"). `expected` is the size the build wrote, for a response that does not say its length.
export async function loadEngine(url, expected, progress) {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`the engine did not download (HTTP ${response.status})`);
  const total = Number(response.headers.get("content-length")) || expected;
  const chunks = [];
  let received = 0, told = 0;
  progress("download", 0, total);
  if (response.body && response.body.getReader) {
    const reader = response.body.getReader();
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      chunks.push(value);
      received += value.length;
      if (received - told > 256 * 1024) { told = received; progress("download", received, total); }
    }
  } else {
    const all = new Uint8Array(await response.arrayBuffer());
    chunks.push(all);
    received = all.length;
  }
  progress("download", received, Math.max(total, received));
  const bytes = new Uint8Array(received);
  let at = 0;
  for (const c of chunks) { bytes.set(c, at); at += c.length; }
  progress("compile", received, received);
  return WebAssembly.compile(bytes);
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
// /w/src, outputs under /w/out and the run's profile under /w/prof; then the program it ran, flattened (kept while
// the rule's files are the same: the library does not change under a page).
export function explore(engine, { id, rr, parts, sources, lib: libFiles }) {
  if (engine.lib !== libFiles) {
    engine.mount("/lib", nest(libFiles));
    engine.lib = libFiles;
  }
  const rule = { [`${id}.rr`]: rr, ...parts };
  // A few rules include their part files as the repository lays them out ("../packs/python/<id>.<part>.rr",
  // from the rule's folder), so the parts are also mounted there.
  const packs = { python: { ...parts } };
  engine.mount("/w", { rule, packs, src: sources, out: {}, prof: {} });
  const run = engine.command(["logic", `/w/rule/${id}.rr`, "/w/src", "/w/out"]);
  const key = JSON.stringify([id, rr, parts]);
  if (engine.flat?.key !== key && !engine.dead) {
    const flat = engine.command(["flatten", `/w/rule/${id}.rr`]);
    engine.flat = { key, program: flat.code === 0 ? flat.stdout.join("\n") : "", error: flat.stderr.join("\n") };
  }
  const w = engine.read("/w");
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
    program: engine.flat?.program ?? "", programError: engine.flat?.error ?? "",
  };
}

// An engine for `explore`, started on `module`: the same one while it lives, another after a trap.
export async function engineFor(shim, module, current) {
  if (current && !current.dead) return current;
  return new Engine(shim, module, { RERAIL_RR_LIB: "/lib", RERAIL_PROFILE: "/w/prof" }).start();
}
