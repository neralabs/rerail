// The explorer's engine, off the page's thread: compiles the module once, then runs each request on a fresh instance.
import * as shim from "https://cdn.jsdelivr.net/npm/@bjorn3/browser_wasi_shim@0.4.2/dist/index.js";
import { explore } from "./engine.js";

let module = null;

self.onmessage = async ({ data }) => {
  try {
    if (data.type === "load") {
      const response = await fetch(data.wasm);
      if (!response.ok) throw new Error(`the engine did not load: HTTP ${response.status}`);
      // Compiled while it downloads where the file is served as application/wasm; else from its bytes.
      module = await WebAssembly.compileStreaming(response.clone())
        .catch(async () => WebAssembly.compile(await response.arrayBuffer()));
      self.postMessage({ type: "loaded" });
    } else if (data.type === "run") {
      const result = await explore(shim, module, data.request);
      self.postMessage({ type: "result", seq: data.seq, result });
    }
  } catch (error) {
    self.postMessage({ type: "failed", seq: data.seq, message: String(error && error.message ? error.message : error) });
  }
};
