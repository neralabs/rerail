import * as shim from './shim/index.js';

const encode = new TextEncoder(), decode = new TextDecoder();

// A persistent Rust/WASI reactor, with SQLite entirely in WASM memory.
// This module handles bytes and host services; it does not interpret any rules.
export class WasmEngine {
  static async load(url, progress = () => {}) {
    progress('Loading Rerail WASM…');
    let bytes;
    if (globalThis.FALLING_FACTS_EMBEDDED) {
      bytes = Uint8Array.from(atob(globalThis.FALLING_FACTS_EMBEDDED.wasm), char => char.charCodeAt(0));
    } else {
      const response = await fetch(url);
      if (!response.ok) throw new Error(`Could not load Rerail WASM (HTTP ${response.status})`);
      bytes = await response.arrayBuffer();
    }
    progress('Compiling Rerail WASM…');
    return WasmEngine.fromModule(await WebAssembly.compile(bytes), progress);
  }
  static async fromModule(module, progress = () => {}) {
    progress('Starting browser Rerail…');
    let stderr = '';
    const collect = bytes => { stderr = (stderr + decode.decode(bytes)).slice(-8000); };
    const wasi = new shim.WASI(['falling-facts'], [], [
      new shim.OpenFile(new shim.File([])), new shim.ConsoleStdout(collect), new shim.ConsoleStdout(collect),
    ], { debug: false });
    const imports = { wasi_snapshot_preview1: wasi.wasiImport };
    for (const wanted of WebAssembly.Module.imports(module)) {
      if (wanted.module === 'wasi_snapshot_preview1') continue;
      if (wanted.kind !== 'function') throw new Error(`Unsupported WASM import: ${wanted.module}.${wanted.name}`);
      (imports[wanted.module] ??= {})[wanted.name] = () => { throw new Error(`Unsupported WASM call: ${wanted.module}.${wanted.name}`); };
    }
    const instance = await WebAssembly.instantiate(module, imports);
    wasi.initialize(instance);
    const engine = new WasmEngine();
    engine.instance = instance;
    engine.stderr = () => stderr;
    return engine;
  }
  async evaluateDetailed(source, { trace = false } = {}) {
    const request = encode.encode(JSON.stringify({ source, trace }));
    const exports = this.instance.exports;
    const pointer = exports.facts_allocate(request.length);
    new Uint8Array(exports.memory.buffer, pointer, request.length).set(request);
    let packed;
    try { packed = exports.facts_evaluate(pointer, request.length); }
    catch (error) { throw new Error(`Rerail WASM stopped: ${this.stderr() || error.message}`); }
    const responsePointer = Number(packed >> 32n), length = Number(packed & 0xffffffffn);
    // Read the current memory buffer: evaluating a request can grow linear memory.
    const result = JSON.parse(decode.decode(new Uint8Array(exports.memory.buffer, responsePointer, length)));
    if (result.error) throw new Error(result.error);
    return result;
  }
  evaluate(source, options) { return this.evaluateDetailed(source, options).then(result => result.outputs); }
}
