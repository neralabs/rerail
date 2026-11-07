import { build } from 'esbuild';
import { readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';

export async function bundleSingleFile(here, dist) {
  const embedded = {
    wasm: (await readFile(join(dist, 'rerail-facts.wasm'))).toString('base64'),
    sources: JSON.parse(await readFile(join(dist, 'sources.json'), 'utf8')),
  };
  const options = { bundle: true, platform: 'browser', format: 'esm', target: 'es2022', minify: true, write: false, legalComments: 'inline' };
  const worker = await build({ ...options, format: 'iife', entryPoints: [join(dist, 'worker.mjs')],
    define: { 'import.meta.url': JSON.stringify('file:///falling-facts.html') },
    banner: { js: 'globalThis.FALLING_FACTS_EMBEDDED=' + JSON.stringify(embedded) + ';' },
  });
  const page = await build({ ...options, entryPoints: [join(dist, 'browser.mjs')] });
  const script = text => text.replaceAll(/<\/script/gi, '<\\/script');
  let html = await readFile(join(dist, 'index.html'), 'utf8');
  const css = await readFile(join(dist, 'style.css'), 'utf8');
  html = html.replace('<link rel="stylesheet" href="./style.css">', () => `<style>${css}</style>`);
  // A source file can contain script tags or HTML comments. Escape every less-than
  // in JSON so none of those bytes can change the HTML parser's script state.
  const workerJSON = JSON.stringify(worker.outputFiles[0].text).replaceAll('<', '\\u003c');
  html = html.replace(/<script type="module" src="\.\/browser.mjs"><\/script>/,
    () => `<script>globalThis.FALLING_FACTS_WORKER=${workerJSON};</script>\n<script type="module">${script(page.outputFiles[0].text)}</script>`);
  if (html.includes('src="./browser.mjs"')) throw new Error('Single-file script injection failed');
  html = html.replaceAll('href="./"', 'href=""');
  const notices = JSON.stringify(await readFile(join(dist, 'NOTICES.txt'), 'utf8')).replaceAll('<', '\\u003c');
  html = html.replace('</body>', () => `<script type="application/json" id="bundled-notices">${notices}</script>\n</body>`);
  const target = join(here, 'falling-facts.html');
  await writeFile(target, html);
  console.log(`Open directly: ${target} (${(Buffer.byteLength(html) / 1024 / 1024).toFixed(1)} MB).`);
}
