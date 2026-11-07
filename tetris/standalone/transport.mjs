// Browser-only transport. There are no HTTP game or source endpoints.
const embeddedWorker = globalThis.FALLING_FACTS_WORKER;
const workerURL = embeddedWorker ? URL.createObjectURL(new Blob([embeddedWorker], { type: 'text/javascript' })) : new URL('./worker.mjs', import.meta.url);
// Module workers cannot load from an opaque file:// origin in Chromium. The
// single-file build bundles all imports into one classic worker for that case.
const worker = new Worker(workerURL, { type: embeddedWorker ? 'classic' : 'module' });
if (embeddedWorker) worker.addEventListener('message', () => URL.revokeObjectURL(workerURL), { once: true });
const pending = new Map();
let nextId = 0, failed;
const loading = document.getElementById('engine-loading');
worker.addEventListener('message', ({ data }) => {
  if (data.type === 'progress') { if (loading) loading.textContent = data.message; return; }
  const request = pending.get(data.id);
  if (!request) return;
  pending.delete(data.id);
  if (data.error) request.reject(new Error(data.error));
  else request.resolve(data.result);
});
function fail(error) {
  failed = error;
  if (loading) loading.textContent = error.message;
  for (const request of pending.values()) request.reject(error);
  pending.clear();
}
worker.addEventListener('error', event => fail(new Error(event.message || 'The Rerail worker could not start. Reload the page.')));
worker.addEventListener('messageerror', () => fail(new Error('Could not read the Rerail worker response.')));
function call(method, ...args) {
  if (failed) return Promise.reject(failed);
  return new Promise((resolve, reject) => {
    const id = ++nextId;
    pending.set(id, { resolve, reject });
    worker.postMessage({ id, method, args });
  });
}
export const driver = {
  async latest() {
    try {
      const result = await call('latest');
      if (loading) loading.hidden = true;
      return result;
    } catch (error) { if (loading) loading.textContent = error.message; throw error; }
  },
  dispatch: (...args) => call('dispatch', ...args),
  setTracing: (...args) => call('setTracing', ...args),
  sourceList: (...args) => call('sourceList', ...args),
  readSource: (...args) => call('readSource', ...args),
};
// Cache every packaged asset so a later reload works without a connection.
if (!embeddedWorker && 'serviceWorker' in navigator) {
  navigator.serviceWorker.register(new URL('./offline-worker.js', import.meta.url), { scope: './' }).catch(error => {
    console.info('Offline reload cache unavailable:', error.message);
  });
}
