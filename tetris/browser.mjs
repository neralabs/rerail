// The browser captures facts and applies the engine's screen projection. No game rules.
import { EvaluationView } from './evaluation.mjs';
import { SourceBrowser } from './source-browser.mjs';
const grids = new Map(['board', 'preview'].map(area => [area, document.getElementById(area)]));
const cells = new Map();
const byId = id => document.getElementById(id);
const fact = (name, row) => `${name}(${row.map(value => JSON.stringify(value)).join(', ')}).`;
let lastSequence = 0;
let latest;
let tail = Promise.resolve();
let selectedRound;
const evaluation = new EvaluationView(result => { selectedRound = result; if (!byId('inspector').hidden) showTrace(); });
const sourceBrowser = new SourceBrowser();
const texts = result => Object.fromEntries(result.screen.screen_text);
const textElements = new Map();
for (const element of document.querySelectorAll('[data-text]')) {
  const name = element.dataset.text;
  if (!textElements.has(name)) textElements.set(name, []);
  textElements.get(name).push(element);
}
const modeButtons = [...document.querySelectorAll('[data-time-mode]')];
const manualButtons = [...document.querySelectorAll('.manual-step'), byId('time-step'), byId('gravity-step')];

function renderScreen(result) {
  const inspecting = texts(result).time_mode === 'manual' && (!byId('inspector').hidden || !byId('evaluation-panel').hidden);
  const changed = new Set(inspecting ? result.trace.changes.added.map(({ relation, row }) => fact(relation, row)) : []);
  for (const [area, x, y, color, style] of result.screen.screen_cell) {
    const key = `${area}:${x}:${y}`;
    let cell = cells.get(key);
    if (!cell) {
      cell = document.createElement('div'); cell.style.gridColumn = x + 1; cell.style.gridRow = y + 1;
      grids.get(area).append(cell); cells.set(key, cell);
    }
    const className = `cell ${color} ${style}${inspecting && result.sequence > 1 && changed.has(fact('screen_cell', [area, x, y, color, style])) ? ' fact-changed' : ''}`;
    if (cell.className !== className) cell.className = className;
  }
  for (const [id, value] of result.screen.screen_text) {
    for (const element of textElements.get(id) ?? []) {
      if (element.textContent !== String(value)) element.textContent = value;
      const highlight = inspecting && result.sequence > 1 && changed.has(fact('screen_text', [id, value]));
      if (element.classList.contains('fact-value-changed') !== highlight) element.classList.toggle('fact-value-changed', highlight);
    }
  }
}
function render(result) {
  if (result.sequence <= lastSequence) return;
  lastSequence = result.sequence;
  latest = result;
  renderScreen(result);
  const text = texts(result);
  byId('overlay').hidden = !text.overlay;
  for (const button of modeButtons) button.setAttribute('aria-pressed', String(button.dataset.timeMode === text.time_mode));
  for (const button of manualButtons) button.disabled = text.time_mode !== 'manual';
  byId('time-hint').textContent = text[text.time_mode === 'manual' ? 'manual_hint' : 'realtime_hint'];
  document.body.dataset.timeMode = text.time_mode;
  const controls = byId('controls');
  if (!controls.childElementCount) {
    for (const [, keys, label] of result.screen.screen_control.toSorted((a, b) => a[0] - b[0])) {
      const row = document.createElement('div'), key = document.createElement('kbd'), text = document.createElement('span');
      key.textContent = keys; text.textContent = label; row.append(key, text); controls.append(row);
    }
  }
  if (byId('evaluation-panel').hidden) selectedRound = null;
  evaluation.accept(result);
  if (!byId('inspector').hidden) showTrace();
}

function showTrace() {
  const result = selectedRound ?? latest;
  if (!result) return;
  const { trace } = result;
  const { added, removed, unchanged } = trace.changes;
  byId('trace-summary').textContent = `Round ${result.sequence} · ${trace.ms} ms · +${added.length} / −${removed.length} / ${unchanged} same`;
  const changed = new Set(added.map(({ relation, row }) => fact(relation, row)));
  const container = byId('trace'); container.replaceChildren();
  const line = (text, className = '') => {
    const element = document.createElement('span'); element.className = `fact-line ${className}`; element.textContent = text; container.append(element);
  };
  const heading = text => line(`\n// ${text}`, 'fact-heading');
  heading('INPUT');
  for (const input of trace.inputs) line(`→ ${input}`, 'input-fact');
  if (removed.length) {
    heading('REMOVED THIS ROUND');
    for (const { relation, row } of removed) line(`− ${fact(relation, row)}`, 'removed');
  }
  const current = (text, isNew) => { if (isNew || !byId('changes-only').checked) line(`${isNew ? '+ ' : '  '}${text}`, isNew ? 'added' : 'unchanged'); };
  heading('RETAINED STATE');
  for (const row of trace.state.split('\n').filter(Boolean)) current(row, changed.has(row));
  heading('SCREEN');
  for (const [name, rows] of Object.entries(result.screen)) for (const row of rows) {
    const text = fact(name, row); current(text, changed.has(text));
  }
}
function report(error) {
  const banner = byId('error'); banner.hidden = false; banner.textContent = error.message;
}
function enqueue(callback) {
  const request = tail.then(callback);
  tail = request.catch(report);
  return request;
}
function send(relation, value) {
  return enqueue(async () => {
    const response = await fetch('/facts', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ relation, value }) });
    const result = await response.json();
    if (!response.ok) throw new Error(result.error ?? 'Could not evaluate event');
    render(result);
    return result;
  });
}
let pendingKeys = 0;
function sendKey(code, repeat = false) {
  // Backpressure on OS repeats prevents stale movement facts accumulating.
  // Distinct presses still arrive in order, and the engine interprets every key.
  if (repeat && pendingKeys) return;
  pendingKeys++;
  send('keypress', code).catch(() => {}).finally(() => { pendingKeys--; });
}
async function setGraph(open) {
  byId('watch-engine').disabled = true;
  try {
    await enqueue(async () => {
      const response = await fetch('/trace', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ enabled: open }) });
      if (!response.ok) throw new Error('Could not change native tracing');
    });
    byId('evaluation-panel').hidden = !open;
    byId('watch-engine').setAttribute('aria-expanded', String(open));
    document.body.classList.toggle('graph-open', open);
    evaluation.setOpen(open);
    if (open) {
      (matchMedia('(min-width: 1151px)').matches ? document.querySelector('.workspace') : byId('evaluation-panel')).scrollIntoView({ block: 'start' });
      await send('timestep', 0);
    } else { document.body.classList.remove('graph-expanded'); selectedRound = null; if (!byId('inspector').hidden) showTrace(); }
    renderScreen(latest);
    byId('watch-engine').blur();
  } catch (error) { report(error); }
  finally { byId('watch-engine').disabled = false; }
}

document.addEventListener('keydown', event => {
  if (byId('source-browser').open) return;
  if (event.ctrlKey || event.metaKey || event.altKey) return;
  const form = event.target.closest('input, textarea, select');
  if (form && !(form.tagName === 'INPUT' && ['checkbox', 'range'].includes(form.type) &&
      !['Space', 'ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.code))) return;
  if (event.target.closest('#evaluation-panel, #watch-engine, #inspect') && ['Space', 'Enter', 'ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(event.code)) return;
  if (['Space', 'ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(event.code)) event.preventDefault();
  if (event.repeat && ['KeyP', 'KeyR', 'Space', 'KeyM', 'KeyN', 'KeyG'].includes(event.code)) return;
  sendKey(event.code, event.repeat);
});
for (const button of document.querySelectorAll('[data-key]')) {
  button.addEventListener('click', () => { send('keypress', button.dataset.key).catch(() => {}); button.blur(); });
}
for (const button of document.querySelectorAll('[data-time-mode]')) {
  button.addEventListener('click', async () => {
    const desired = button.dataset.timeMode;
    for (const mode of document.querySelectorAll('[data-time-mode]')) mode.disabled = true;
    try { if (texts(latest).time_mode !== desired) await send('keypress', 'KeyM'); }
    catch { /* The request queue displays the error. */ }
    finally { for (const mode of document.querySelectorAll('[data-time-mode]')) mode.disabled = false; button.blur(); }
  });
}
byId('inspect').addEventListener('click', event => {
  const panel = byId('inspector'); panel.hidden = !panel.hidden;
  event.currentTarget.setAttribute('aria-expanded', String(!panel.hidden));
  event.currentTarget.blur();
  renderScreen(latest);
  if (!panel.hidden) showTrace();
});
byId('changes-only').addEventListener('change', showTrace);
byId('watch-engine').addEventListener('click', () => setGraph(byId('evaluation-panel').hidden));
byId('close-evaluation').addEventListener('click', () => setGraph(false));
byId('browse-source').addEventListener('click', () => sourceBrowser.open());
byId('browse-current-source').addEventListener('click', () => {
  const rule = evaluation.pinnedRule ?? evaluation.firingRule;
  sourceBrowser.open({ path: 'game.rr', line: evaluation.sourceSpans?.get(rule)?.start });
});

render(await fetch('/facts').then(response => response.json()));
// Manual time is also enforced by the rules, so stale timer facts cannot advance it.
let previous = performance.now();
async function tick() {
  const now = performance.now(), elapsed = Math.min(1000, Math.round(now - previous)); previous = now;
  if (!document.hidden && texts(latest).time_mode === 'realtime') await send('timestep', elapsed).catch(() => {});
  setTimeout(tick, 60);
}
document.addEventListener('visibilitychange', () => { previous = performance.now(); });
setTimeout(tick, 60);
