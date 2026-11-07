import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { Engine, Game, rules } from './engine.mjs';
import { sourceCatalog } from './sources.mjs';

const engine = new Engine();
const game = new Game(engine);
const sources = sourceCatalog(rules);
await game.dispatch('keypress', 'KeyR');
const assets = new Map([
  ['/', ['index.html', 'text/html']],
  ['/style.css', ['style.css', 'text/css']],
  ['/browser.mjs', ['browser.mjs', 'text/javascript']],
  ['/transport.mjs', ['transport.mjs', 'text/javascript']],
  ['/evaluation.mjs', ['evaluation.mjs', 'text/javascript']],
  ['/rr-source.mjs', ['rr-source.mjs', 'text/javascript']],
  ['/source-browser.mjs', ['source-browser.mjs', 'text/javascript']],
  ['/waves.mjs', ['waves.mjs', 'text/javascript']],
]);
const server = createServer(async (req, res) => {
  try {
    res.setHeader('Cache-Control', 'no-store');
    const url = new URL(req.url, 'http://localhost');
    const pathname = url.pathname;
    const asset = assets.get(pathname);
    if (req.method === 'GET' && asset) {
      res.setHeader('Content-Type', asset[1]);
      res.end(await readFile(new URL(`./${asset[0]}`, import.meta.url)));
    } else if (req.method === 'GET' && pathname === '/sources') {
      const result = url.searchParams.has('path') ? await sources.source(url.searchParams.get('path')) : await sources.list();
      res.setHeader('Content-Type', 'application/json');
      res.writeHead(result ? 200 : 404).end(JSON.stringify(result ?? { error: 'Unknown source file' }));
    } else if (req.method === 'GET' && pathname === '/facts') {
      res.setHeader('Content-Type', 'application/json');
      res.end(JSON.stringify(game.latest));
    } else if (req.method === 'POST' && (pathname === '/facts' || pathname === '/trace')) {
      let body = '';
      for await (const chunk of req) {
        body += chunk;
        if (body.length > 1024) { res.writeHead(413).end(); return; }
      }
      const payload = JSON.parse(body);
      if (pathname === '/trace') {
        if (typeof payload.enabled !== 'boolean') { res.writeHead(400).end('enabled must be a boolean'); return; }
        await game.setTracing(payload.enabled);
        res.setHeader('Content-Type', 'application/json');
        res.end(JSON.stringify({ enabled: game.tracing }));
        return;
      }
      const { relation, value } = payload;
      // Transport validation only. Key interpretation and timing belong to the rules.
      if (!((relation === 'keypress' && typeof value === 'string' && /^[A-Za-z0-9]{1,24}$/.test(value)) ||
        (relation === 'timestep' && Number.isInteger(value) && value >= 0 && value <= 1000))) {
        res.writeHead(400).end('Invalid event fact'); return;
      }
      res.setHeader('Content-Type', 'application/json');
      res.end(JSON.stringify(await game.dispatch(relation, value)));
    } else res.writeHead(404).end();
  } catch (error) {
    console.error(error);
    res.writeHead(500, { 'Content-Type': 'application/json' }).end(JSON.stringify({ error: error.message }));
  }
});
const port = Number(process.env.PORT ?? 4317);
server.listen(port, '127.0.0.1', () => console.log(`Falling Facts: http://127.0.0.1:${port}`));
for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => {
  server.close(); engine.close(); process.exit(0);
});
