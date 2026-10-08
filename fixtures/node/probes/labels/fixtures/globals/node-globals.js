setImmediate(() => {});
const buf = Buffer.from('x');
global.answer = 42;
queueMicrotask(() => {});
function local(Buffer) {
  return Buffer;
}
globalThis.answer = 43;
