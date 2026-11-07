// The explorer's engine, off the page's thread: loads the WASI shim and the runner, downloads and compiles the module
// once, then runs each request on a fresh instance. Every stage is reported, so the page can show where loading is,
// and a failure (a blocked import, a failed download) reaches the page instead of leaving it waiting.
let shim = null, runner = null, module = null;

self.postMessage({ type: "stage", stage: "worker" });

self.onmessage = async ({ data }) => {
  try {
    if (data.type === "load") {
      self.postMessage({ type: "stage", stage: "shim" });
      shim = await import(data.shim);
      runner = await import(data.engine);
      module = await runner.loadEngine(data.wasm, data.bytes, (stage, received, total) =>
        self.postMessage({ type: "stage", stage, received, total }));
      self.postMessage({ type: "loaded" });
    } else if (data.type === "run") {
      const result = await runner.explore(shim, module, data.request);
      self.postMessage({ type: "result", seq: data.seq, result });
    }
  } catch (error) {
    self.postMessage({ type: "failed", seq: data.seq, message: String(error && error.message ? error.message : error) });
  }
};
