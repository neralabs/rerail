import { WasmEngine } from './wasm-engine.mjs';
import { GameSession } from './game-session.mjs';

const progress = message => self.postMessage({ type: 'progress', message });
const ready = (async () => {
  const embedded = globalThis.FALLING_FACTS_EMBEDDED;
  const [engine, sources] = await Promise.all([
    WasmEngine.load(embedded ? null : new URL('./rerail-facts.wasm', import.meta.url), progress),
    embedded ? embedded.sources : fetch(new URL('./sources.json', import.meta.url)).then(response => {
      if (!response.ok) throw new Error('Could not load bundled rule sources');
      return response.json();
    }),
  ]);
  const game = new GameSession(engine, { rules: sources.contents['game.rr'] });
  await game.dispatch('keypress', 'KeyR');
  return { game, sources };
})();
// Messages remain ordered even during startup. The page never runs an evaluator.
let tail = Promise.resolve();
self.onmessage = ({ data: { id, method, args = [] } }) => {
  const operation = tail.then(async () => {
    const { game, sources } = await ready;
    let result;
    if (method === 'latest') result = game.latest;
    else if (method === 'dispatch') result = await game.dispatch(...args);
    else if (method === 'setTracing') { await game.setTracing(args[0]); result = { enabled: game.tracing }; }
    else if (method === 'sourceList') result = { revision: sources.revision, files: sources.files };
    else if (method === 'readSource') {
      const file = sources.files.find(file => file.path === args[0]);
      if (!file) throw new Error('Unknown source file');
      result = { ...file, text: sources.contents[file.path] };
    } else throw new Error(`Unknown engine operation: ${method}`);
    self.postMessage({ id, result });
  });
  tail = operation.catch(error => self.postMessage({ id, error: error.message }));
};
// Keep startup errors handled until the first request can report them to the page.
ready.catch(() => {});
