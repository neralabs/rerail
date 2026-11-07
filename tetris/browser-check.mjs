// Optional local browser QA, using the source checkout's existing Playwright installation.
import assert from 'node:assert/strict';
import { resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
const source = resolve(process.env.RERAIL_SOURCE ?? fileURLToPath(new URL('../../wolfeel', import.meta.url)));
const { chromium } = await import(pathToFileURL(resolve(source, 'node_modules/@playwright/test/index.mjs')));
const baseURL = process.env.TETRIS_URL ?? 'http://127.0.0.1:4317';
const browser = await chromium.launch({ headless: true });
const page = await browser.newPage({ viewport: { width: 1280, height: 1100 } });
const errors = [];
page.on('pageerror', error => errors.push(error.message));
const external = [];
page.on('request', request => { if (!request.url().startsWith(baseURL)) external.push(request.url()); });
const text = name => page.locator(`[data-text="${name}"]`).first();
async function key(code) {
  const request = page.waitForResponse(response => response.url().endsWith('/facts') && response.request().method() === 'POST' && JSON.parse(response.request().postData()).value === code);
  await page.keyboard.press(code.replace('Key', ''));
  assert.equal((await request).status(), 200);
}
try {
  await page.goto(baseURL + '/');
  await page.waitForFunction(() => document.querySelectorAll('#board .cell').length === 200);
  assert.equal(await page.locator('#preview .cell').count(), 16);
  assert.equal(await text('title').textContent(), 'FALLING FACTS');
  await page.locator('#browse-source').click();
  await page.waitForFunction(() => document.querySelectorAll('#source-browser-code .source-line').length > 200);
  assert.equal(await page.locator('#source-browser').isVisible(), true);
  assert.ok(parseInt(await page.locator('#source-browser-file-count').textContent()) > 100);
  for (const kind of ['directive', 'relation', 'variable', 'number', 'string', 'comment', 'operator']) {
    assert.ok(await page.locator(`#source-browser-code .syntax-${kind}`).count() > 0, `Missing syntax color for ${kind}`);
  }
  await page.locator('#source-text-search').fill('next_active');
  assert.ok(await page.locator('#source-browser-code .source-search-match').count() > 0);
  await page.locator('#source-go-line').fill('150');
  await page.locator('#source-line-go').click();
  assert.ok(await page.locator('#source-browser-code .source-browser-selected[data-line="150"]').count() > 0);
  await page.locator('#source-file-filter').fill('core.rr');
  await page.locator('.source-file[data-path="docs/rule-syntax/lib/core.rr"]').click();
  await page.waitForFunction(() => document.getElementById('source-browser-path').textContent.endsWith('/core.rr'));
  assert.match(await page.locator('#source-browser-code').textContent(), /\.decl/);
  assert.equal(await page.locator('#source-browser-kind').textContent(), 'PINNED ENGINE REFERENCE');
  await page.screenshot({ path: fileURLToPath(new URL('./.cache/source-browser.png', import.meta.url)), fullPage: true });
  await page.locator('#close-source-browser').click();
  await key('KeyR');
  await key('KeyP');
  await page.waitForFunction(() => !document.getElementById('overlay').hidden);
  assert.equal(await text('overlay').textContent(), 'PAUSED');
  await page.screenshot({ path: fileURLToPath(new URL('./.cache/desktop.png', import.meta.url)), fullPage: true });
  await key('KeyP');
  await page.waitForFunction(() => document.getElementById('overlay').hidden);
  await key('ArrowLeft');
  await key('ArrowUp');
  await key('Space');
  await page.waitForFunction(() => Number(document.querySelector('[data-text="pieces"]').textContent) === 1);
  assert.ok(Number(await text('score').textContent()) > 0);
  await page.locator('#inspect').click();
  await page.waitForFunction(() => document.getElementById('trace').textContent.includes('screen_cell('));
  assert.match(await page.locator('#trace').textContent(), /active\(/);
  await page.locator('#inspect').click();
  await key('KeyP');
  await page.screenshot({ path: fileURLToPath(new URL('./.cache/desktop.png', import.meta.url)), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: fileURLToPath(new URL('./.cache/mobile.png', import.meta.url)), fullPage: true });
  assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
  assert.equal(await page.locator('.touch-controls').isVisible(), true);
  await page.locator('.touch-controls [data-key="KeyP"]').click();
  await page.waitForFunction(() => document.getElementById('overlay').hidden);
  await page.locator('#restart').click();
  await page.waitForFunction(() => document.querySelector('[data-text="pieces"]').textContent === '0');
  await page.locator('#manual-mode').click();
  await page.waitForFunction(() => document.getElementById('manual-mode').getAttribute('aria-pressed') === 'true');
  await key('KeyR');
  const manual = await page.request.get(baseURL + '/facts').then(response => response.json());
  await page.waitForTimeout(300);
  const stationary = await page.request.get(baseURL + '/facts').then(response => response.json());
  assert.equal(stationary.sequence, manual.sequence, 'manual mode must not submit automatic timesteps');
  await key('KeyN');
  await page.waitForFunction(() => document.querySelector('[data-text="clock"]').textContent === '100');
  await page.locator('#inspect').click();
  assert.match(await page.locator('#trace .added').allTextContents().then(lines => lines.join('\n')), /clock\(100\)/);
  assert.match(await page.locator('#trace .removed').allTextContents().then(lines => lines.join('\n')), /clock\(0\)/);
  await page.locator('#changes-only').check();
  assert.equal(await page.locator('#trace .unchanged').count(), 0);
  await page.locator('#changes-only').uncheck();
  await key('KeyG');
  assert.equal((await page.request.get(baseURL + '/facts').then(response => response.json())).trace.input, 'keypress("KeyG").');

  await page.setViewportSize({ width: 1560, height: 1150 });
  await page.locator('#watch-engine').click();
  await page.waitForFunction(() => document.querySelectorAll('.fact-node').length > 30);
  assert.equal(await page.locator('#evaluation-panel').isVisible(), true);
  assert.ok(await page.locator('.rule-edge').count() > 50);
  assert.equal(await page.locator('.node-count').count(), 0);
  assert.equal(await page.locator('.node-sigil').count(), await page.locator('.fact-node').count());
  assert.match(await page.locator('.fact-node[data-relation="active"] .node-tuple').textContent(), /\([0-9]/);
  await page.locator('#expand-graph').click();
  assert.ok(await page.evaluate(() => document.body.classList.contains('graph-expanded')));
  await page.locator('#expand-graph').click();
  const traced = await page.request.get(baseURL + '/facts').then(response => response.json());
  assert.ok(traced.trace.evaluation.waves.length >= 1);
  assert.equal(await page.locator('#wave-markers .wave-band').count(), traced.trace.evaluation.waves.length);
  assert.ok(traced.trace.evaluation.events.some(event => event.cache_hit));
  if ((await page.locator('#wave-play').textContent()).includes('Pause')) await page.locator('#wave-play').click();
  await page.locator('#wave-position').evaluate(input => { input.value = 0; input.dispatchEvent(new Event('input', { bubbles: true })); });
  assert.equal(await page.locator('.fact-node[data-relation="screen_cell"] .screen-mini-grid[data-area="board"] .svg-screen-cell').count(), 200, 'screen must already be populated before the first bubble');
  assert.equal(await page.locator('.fact-node[data-relation="screen_cell"] .screen-mini-grid[data-area="preview"] .svg-screen-cell').count(), 16);
  assert.equal(await page.locator('.fact-node[data-relation="screen_cell"] .sigil-count').textContent(), '◆216');
  assert.equal(await page.locator('.fact-node[data-relation="screen_cell"] .screen-mini-grid[data-area="board"] .svg-screen-cell.active').count(), 4);
  assert.equal(await page.locator('.fact-node[data-wave-state="current"]').count(), 0);
  assert.equal(await page.locator('.fact-node[data-wave-state="previous"]').count(), 0);
  const position = await page.locator('#wave-position').inputValue();
  await page.waitForTimeout(150);
  assert.equal(await page.locator('#wave-position').inputValue(), position, 'paused playback must stay still');
  await page.locator('#wave-next').click();
  assert.ok(Number(await page.locator('#wave-position').inputValue()) > Number(position));
  const bounce = traced.trace.evaluation.events.findIndex(event => event.cache_hit && event.candidates.length);
  assert.ok(bounce >= 0);
  await page.locator('#wave-position').evaluate((input, position) => { input.value = position; input.dispatchEvent(new Event('input', { bubbles: true })); }, bounce);
  await page.locator('#wave-speed').selectOption('0.5');
  await page.locator('#wave-play').click();
  await page.waitForFunction(() => document.querySelectorAll('.node-bounce').length > 0 && document.getElementById('wave-status').dataset.outcome === 'cached');
  assert.ok(await page.locator('.wave-packet[fill="#f5bf70"]').count() > 0);
  await page.waitForFunction(() => [...document.querySelectorAll('.wave-packet[data-bounce="true"]')].some(packet => Number(packet.dataset.progress) > 0.86));
  const retreat = await page.locator('.wave-packet[data-bounce="true"]').first().getAttribute('data-distance');
  assert.ok(Number(retreat) > 0.9 && Number(retreat) < 1, 'rejection must retreat only a little from the receiver');
  assert.match(await page.locator('#firing-rule-source').textContent(), /:-/);
  assert.match(await page.locator('#firing-rule-location').textContent(), /game\.rr:[0-9]+/);
  await page.screenshot({ path: fileURLToPath(new URL('./.cache/evaluation.png', import.meta.url)), fullPage: true });
  await page.locator('#wave-play').click();
  await page.locator('#wave-speed').selectOption('1');
  await page.locator('#wave-position').evaluate(input => { input.value = input.max; input.dispatchEvent(new Event('input', { bubbles: true })); });
  assert.equal(await page.locator('.fact-node[data-relation="screen_cell"] .screen-mini-grid[data-area="board"] .svg-screen-cell').count(), 200);
  assert.equal(await page.locator('.fact-node[data-relation="screen_cell"] .screen-mini-grid[data-area="preview"] .svg-screen-cell').count(), 16);
  await page.locator('.fact-node[data-relation="screen_cell"]').click();
  assert.equal(await page.locator('#detail-visual .fact-grid[data-area="board"] .cell').count(), 200);
  assert.equal(await page.locator('#detail-visual .fact-grid[data-area="preview"] .cell').count(), 16);

  const edgePoint = await page.evaluate(() => {
    const svg = document.getElementById('evaluation-svg').getBoundingClientRect();
    for (const edge of document.querySelectorAll('.edge-hit')) for (const fraction of [0.3, 0.5, 0.7]) {
      const point = edge.getPointAtLength(edge.getTotalLength() * fraction);
      const client = new DOMPoint(point.x, point.y).matrixTransform(edge.getScreenCTM());
      if (client.x <= svg.left || client.x >= svg.right || client.y <= svg.top || client.y >= svg.bottom || client.y < 0 || client.y > innerHeight) continue;
      const hit = document.elementFromPoint(client.x, client.y);
      if (hit?.classList.contains('edge-hit')) return { x: client.x, y: client.y, rule: Number(hit.dataset.rule) };
    }
  });
  assert.ok(edgePoint, 'graph edges need usable mouse targets');
  await page.mouse.move(edgePoint.x, edgePoint.y);
  await page.locator('#rule-tooltip').waitFor({ state: 'visible' });
  const edgeRule = traced.trace.evaluation.rules.find(rule => rule.id === edgePoint.rule);
  assert.match(await page.locator('#rule-tooltip-location').textContent(), new RegExp(`game\\.rr:${edgeRule.source_line}`));
  assert.match(await page.locator('#rule-tooltip-source').textContent(), /:-/);
  await page.mouse.click(edgePoint.x, edgePoint.y);
  assert.match(await page.locator('#source-caption').textContent(), new RegExp(`Pinned rule #${edgeRule.id} `));
  assert.ok(await page.locator(`#rule-source .selected-source-line[data-line="${edgeRule.source_line}"]`).count() > 0);
  assert.equal(await page.locator('#follow-rule-source').getAttribute('aria-pressed'), 'false');
  await page.locator('#follow-rule-source').click();
  assert.equal(await page.locator('#follow-rule-source').getAttribute('aria-pressed'), 'true');
  await page.locator('.fact-node[data-relation="active"]').click();
  assert.equal(await page.locator('#detail-title').textContent(), 'active');
  assert.match(await page.locator('#detail-facts').textContent(), /active\(/);
  const gameStep = page.waitForResponse(response => response.url().endsWith('/facts') && response.request().method() === 'POST' && JSON.parse(response.request().postData()).value === 'KeyN');
  await page.locator('.graph-time-steps [data-key="KeyN"]').click();
  assert.equal((await gameStep).status(), 200);
  await page.waitForFunction(() => document.querySelectorAll('#round-history option').length >= 2);
  const updated = await page.request.get(baseURL + '/facts').then(response => response.json());
  if ((await page.locator('#wave-play').textContent()).includes('Pause')) await page.locator('#wave-play').click();
  const accepted = updated.trace.evaluation.events.findIndex(event => event.display_outcome === 'changed');
  assert.ok(accepted >= 0);
  await page.locator('#wave-position').evaluate((input, position) => { input.value = position; input.dispatchEvent(new Event('input', { bubbles: true })); }, accepted);
  await page.locator('#wave-next').click();
  assert.ok(await page.locator(`.fact-node[data-relation="${updated.trace.evaluation.events[accepted].head}"].node-accepted`).count() > 0, 'an accepted update must glow');
  const glow = await page.locator(`.fact-node[data-relation="${updated.trace.evaluation.events[accepted].head}"] .node-background`).evaluate(element => {
    const style = getComputedStyle(element); return { width: parseFloat(style.strokeWidth), filter: style.filter };
  });
  assert.ok(glow.width >= 3.5 && glow.filter.includes('drop-shadow'), 'accepted changes need a strong, visible glow');
  const recorded = updated.trace.evaluation;
  assert.ok(recorded.waves.length < 10, 'a small time step must not inflate into dozens of unchanged waves');
  assert.ok(recorded.waves.every(wave => wave.updated.length > 0), 'a changing round must not contain empty waves');
  const frontier = recorded.waves.find(wave => {
    const last = new Map();
    for (const event of recorded.events.slice(0, wave.end - 1)) {
      if (event.changes.added.length || event.changes.removed.length) last.set(event.head, event.wave);
    }
    return [...last.values()].includes(wave.id) && [...last.values()].some(id => id < wave.id);
  });
  assert.ok(frontier, 'recorded round needs a visible frontier before a wave ends');
  await page.locator('#wave-position').evaluate((input, position) => { input.value = position; input.dispatchEvent(new Event('input', { bubbles: true })); }, frontier.end - 1);
  assert.equal(await page.locator('#evaluation-panel').getAttribute('data-current-wave'), String(frontier.id));
  assert.match(await page.locator('#wave-label').textContent(), new RegExp(`Wave ${frontier.id} /`));
  assert.ok(await page.locator('.fact-node[data-wave-state="current"]').count() > 0);
  assert.ok(await page.locator('.fact-node[data-wave-state="previous"]').count() > 0);
  assert.ok(await page.locator('.fact-node[data-wave-state="unchanged"]').count() > 0);
  assert.equal(await page.locator('.node-accepted').count(), 0, 'seeking must clear transient glows from future steps');
  const beforeAge = await page.locator('.fact-node[data-wave-state="current"]').first().getAttribute('data-relation');
  const currentCount = await page.locator('.fact-node[data-wave-state="current"]').count();
  await page.waitForTimeout(250);
  assert.equal(await page.locator('.fact-node[data-wave-state="current"]').count(), currentCount, 'frontier persists when playback is paused');
  await page.screenshot({ path: fileURLToPath(new URL('./.cache/wave-frontier.png', import.meta.url)), fullPage: true });
  const nextWave = recorded.waves.find(wave => wave.id === frontier.id + 1);
  if (nextWave) {
    await page.locator('#wave-position').evaluate((input, position) => { input.value = position; input.dispatchEvent(new Event('input', { bubbles: true })); }, nextWave.start);
    assert.equal(await page.locator(`.fact-node[data-relation="${beforeAge}"]`).getAttribute('data-wave-state'), 'previous');
  }
  await page.locator(`#wave-markers .wave-band[data-wave="${frontier.id}"]`).click();
  assert.equal(Number(await page.locator('#wave-position').inputValue()), frontier.end - 1);
  assert.equal(await page.locator(`#wave-markers .wave-band[data-wave="${frontier.id}"]`).getAttribute('aria-current'), 'true');
  await page.locator('#wave-position').evaluate(input => { input.value = 0; input.dispatchEvent(new Event('input', { bubbles: true })); });
  assert.equal(await page.locator('.fact-node[data-wave-state="current"]').count(), 0);
  assert.equal(await page.locator('.fact-node[data-wave-state="previous"]').count(), 0);
  await page.locator('#round-history').selectOption(String(traced.sequence));
  assert.match(await page.locator('#trace-summary').textContent(), new RegExp(`Round ${traced.sequence} `));
  await page.setViewportSize({ width: 390, height: 844 });
  assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
  await page.locator('#browse-current-source').click();
  await page.locator('#source-wrap').check();
  await page.waitForFunction(() => document.getElementById('source-browser-path').textContent === 'game.rr');
  assert.equal(await page.locator('.source-file[data-path="game.rr"]').getAttribute('aria-current'), 'true');
  assert.ok(await page.evaluate(() => document.getElementById('source-browser').getBoundingClientRect().width <= innerWidth));
  await page.screenshot({ path: fileURLToPath(new URL('./.cache/source-browser-mobile.png', import.meta.url)), fullPage: true });
  await page.locator('#close-source-browser').click();
  await page.screenshot({ path: fileURLToPath(new URL('./.cache/evaluation-mobile.png', import.meta.url)), fullPage: true });
  await page.locator('#close-evaluation').click();
  await page.waitForFunction(() => document.getElementById('evaluation-panel').hidden);
  await key('KeyN');
  assert.equal((await page.request.get(baseURL + '/facts').then(response => response.json())).trace.evaluation, null);

  // Normal play must not retain the inspection outline on vacated cells.
  await page.locator('#inspect').click();
  await key('KeyR');
  await key('ArrowLeft');
  assert.equal(await page.locator('#board .fact-changed, .fact-value-changed').count(), 0);
  await page.evaluate(() => {
    window.cellMutations = 0;
    window.cellObserver = new MutationObserver(records => { window.cellMutations += records.length; });
    window.cellObserver.observe(document.getElementById('board'), { subtree: true, attributes: true, childList: true });
  });
  await key('KeyQ');
  assert.equal(await page.evaluate(() => window.cellMutations), 0, 'unchanged screen facts must not rewrite board cells');
  await page.evaluate(() => window.cellObserver.disconnect());

  // A slow response must not turn held-key repeats into delayed movements.
  let repeatedRequests = 0;
  await page.route('**/facts', async route => {
    if (route.request().method() === 'POST' && JSON.parse(route.request().postData()).value === 'ArrowDown') {
      repeatedRequests++;
      await new Promise(resolve => setTimeout(resolve, 150));
    }
    await route.continue();
  });
  const repeatResponse = page.waitForResponse(response => response.url().endsWith('/facts') && response.request().postDataJSON()?.value === 'ArrowDown');
  await page.evaluate(() => {
    for (let i = 0; i < 40; i++) document.body.dispatchEvent(new KeyboardEvent('keydown', { code: 'ArrowDown', repeat: i > 0, bubbles: true }));
  });
  await repeatResponse;
  await key('ArrowLeft');
  assert.equal(repeatedRequests, 1, 'OS repeats must respect transport backpressure');
  await page.unroute('**/facts');

  await page.setViewportSize({ width: 1560, height: 1150 });
  await page.locator('#realtime-mode').click();
  await page.waitForFunction(() => document.body.dataset.timeMode === 'realtime');
  await key('KeyR');
  await key('ArrowLeft');
  assert.equal(await page.locator('#board .fact-changed, .fact-value-changed').count(), 0);
  await page.locator('#watch-engine').click();
  await page.waitForFunction(() => document.getElementById('evaluation-panel').dataset.playback === 'live');
  assert.equal(await page.evaluate(() => document.body.dataset.timeMode), 'realtime', 'opening the graph must preserve real-time mode');
  await page.evaluate(() => { window.liveCard = document.querySelector('.fact-node[data-relation="screen_cell"]'); });
  const liveRounds = [];
  for (let i = 0; i < 8; i++) {
    const response = await page.waitForResponse(response => response.url().endsWith('/facts') && response.request().postDataJSON()?.relation === 'timestep');
    const round = await response.json();
    await page.waitForFunction(sequence => Number(document.getElementById('evaluation-panel').dataset.round) >= sequence, round.sequence);
    assert.ok(await page.evaluate(() => window.liveCard === document.querySelector('.fact-node[data-relation="screen_cell"]')), 'live updates should reuse the graph');
    assert.ok(await page.locator('#wave-position').evaluate(input => input.value === input.max), 'live view must show the completed round, not old replay steps');
    liveRounds.push(round.trace.ms);
  }
  await page.locator('#wave-play').click();
  const heldRound = await page.locator('#evaluation-panel').getAttribute('data-round');
  const nextLive = await page.waitForResponse(response => response.url().endsWith('/facts') && response.request().postDataJSON()?.relation === 'timestep').then(response => response.json());
  assert.ok(nextLive.sequence > Number(heldRound));
  assert.equal(await page.locator('#evaluation-panel').getAttribute('data-round'), heldRound, 'Hold must preserve the selected round while the game continues');
  await page.locator('#follow-latest').click();
  await page.waitForFunction(sequence => Number(document.getElementById('evaluation-panel').dataset.round) >= sequence && document.getElementById('evaluation-panel').dataset.playback === 'live', nextLive.sequence);
  await page.locator('#wave-position').evaluate(input => { input.value = 0; input.dispatchEvent(new Event('input', { bubbles: true })); });
  assert.equal(await page.locator('#evaluation-panel').getAttribute('data-following'), 'false');
  assert.equal(await page.locator('.fact-node[data-relation="screen_cell"] .svg-screen-cell').count(), 216);
  await page.locator('#follow-latest').click();
  await page.locator('#close-evaluation').click();
  await page.waitForFunction(() => document.getElementById('evaluation-panel').hidden);
  const closed = await page.waitForResponse(response => response.url().endsWith('/facts') && response.request().postDataJSON()?.relation === 'timestep').then(response => response.json());
  assert.equal(closed.trace.evaluation, null, 'closing the graph must disable trace collection');
  await key('ArrowLeft');
  assert.equal(await page.locator('#board .fact-changed').count(), 0);
  const inputLatency = await page.evaluate(async () => {
    const samples = [];
    for (const code of ['ArrowRight', 'ArrowLeft', 'ArrowRight', 'ArrowLeft']) {
      const started = performance.now();
      await new Promise((resolve, reject) => {
        const timeout = setTimeout(() => { observer.disconnect(); reject(new Error('Movement did not render')); }, 1500);
        const observer = new MutationObserver(() => { clearTimeout(timeout); observer.disconnect(); resolve(); });
        observer.observe(document.getElementById('board'), { subtree: true, attributes: true });
        document.body.dispatchEvent(new KeyboardEvent('keydown', { code, bubbles: true }));
      });
      samples.push(Math.round(performance.now() - started));
    }
    return samples;
  });
  console.log(`Key fact to board update: ${inputLatency.join(', ')} ms.`);
  console.log(`Live rounds: ${liveRounds.join(', ')} ms; closed panel: ${closed.trace.ms} ms. No stale replay, stable graph, working hold/resume, bounded key repeats.`);
  assert.deepEqual(errors, []);
  assert.deepEqual(external, []);
  console.log('Browser checks passed: gameplay, manual time, fact colors, receiver effects, source browsing, wave markers, frontier persistence and rewind, playback, history, desktop/mobile, offline assets.');
} finally { await browser.close(); }
