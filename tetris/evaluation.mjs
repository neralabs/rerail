// Playback of native SQL rule events. This module does not evaluate rules.
import { renderSourceLines, highlightSnippet } from './rr-source.mjs';
const ns = 'http://www.w3.org/2000/svg';
const node = (name, attrs = {}, text) => {
  const el = document.createElementNS(ns, name);
  for (const [key, value] of Object.entries(attrs)) el.setAttribute(key, value);
  if (text !== undefined) el.textContent = text;
  return el;
};
const byId = id => document.getElementById(id);
const key = row => JSON.stringify(row);
const literal = (relation, row) => `${relation}(${row.map(value => JSON.stringify(value)).join(', ')}).`;
const cardWidth = 174;
const outcome = event => event.display_outcome ?? event.outcome;
const updates = event => event.changes ?? event;
const hasChanges = event => updates(event).added.length + updates(event).removed.length > 0;
const isEmpty = event => !event.candidates.length && !hasChanges(event);
const colors = { changed: '#baf183', unchanged: '#f5bf70', cached: '#f5bf70', no_match: '#77898d' };

// Find the end of a clause from the source line reported by the native lowerer.
export function sourceSpan(text, line) {
  if (!Number.isInteger(line) || line < 1) return null;
  const lines = text.split('\n');
  if (line > lines.length) return null;
  const start = lines.slice(0, line - 1).reduce((offset, value) => offset + value.length + 1, 0);
  let quote = null, escaped = false, comment = false, depth = 0, endLine = line;
  for (let i = start; i < text.length; i++) {
    const ch = text[i];
    if (ch === '\n') { endLine++; comment = false; }
    if (comment) continue;
    if (quote) {
      if (escaped) escaped = false;
      else if (ch === '\\') escaped = true;
      else if (ch === quote) quote = null;
      continue;
    }
    if (ch === '/' && text[i + 1] === '/') { comment = true; continue; }
    if (ch === '"' || ch === "'" || ch === '`') { quote = ch; continue; }
    if ('({['.includes(ch)) depth++;
    if (')}]'.includes(ch)) depth--;
    if (ch === '.' && depth === 0 && !( /[0-9]/.test(text[i - 1] ?? '') && /[0-9]/.test(text[i + 1] ?? '') )) {
      return { start: line, end: endLine, text: text.slice(start, i + 1).trim() };
    }
  }
  return null;
}

export class EvaluationView {
  constructor(onSelect) {
    this.onSelect = onSelect;
    this.history = [];
    this.position = 0;
    this.playing = false;
    this.open = false;
    this.follow = true;
    this.speed = 1;
    this.svg = byId('evaluation-svg');
    this.frame = null;
    this.acceptedTimers = new Map();
    byId('follow-rule-source').addEventListener('click', () => { this.pinnedRule = null; this.updateSourceSelection(true); });
    byId('wave-play').addEventListener('click', () => this.playing ? this.pause() : this.play());
    byId('wave-next').addEventListener('click', () => { this.pause(); this.advanceOne(); });
    byId('wave-restart').addEventListener('click', () => { this.pause(); this.seek(0); this.play(); });
    byId('wave-position').addEventListener('input', event => { this.pause(); this.seek(Number(event.target.value)); });
    byId('wave-speed').addEventListener('change', event => { this.speed = Number(event.target.value); });
    byId('show-constants').addEventListener('change', () => { this.pause(); this.drawGraph(); this.drawState(); });
    byId('graph-fit').addEventListener('click', () => this.fit());
    byId('graph-zoom-in').addEventListener('click', () => this.zoom(1.25));
    byId('graph-zoom-out').addEventListener('click', () => this.zoom(1 / 1.25));
    byId('expand-graph').addEventListener('click', () => {
      const expanded = document.body.classList.toggle('graph-expanded');
      byId('expand-graph').setAttribute('aria-pressed', String(expanded));
    });
    byId('round-history').addEventListener('change', event => {
      const result = this.history.find(result => result.sequence === Number(event.target.value));
      this.follow = result === this.history.at(-1);
      if (result) this.load(result, false);
    });
    this.attachNavigation();
  }
  setOpen(open) {
    this.open = open;
    if (!open) { this.pause(); this.hideRuleTooltip(); }
    else if (this.result) requestAnimationFrame(() => this.fit());
  }
  accept(result) {
    if (!result.trace.evaluation) return;
    if (!this.history.some(round => round.sequence === result.sequence)) this.history.push(result);
    if (this.history.length > 32) this.history.shift();
    this.updateHistory();
    if (this.open && this.follow && !this.playing) this.load(result, true);
  }
  updateHistory() {
    const select = byId('round-history');
    select.replaceChildren(...this.history.toReversed().map(result => {
      const option = document.createElement('option');
      option.value = result.sequence;
      option.textContent = `#${result.sequence} · ${result.trace.input}`;
      option.selected = result.sequence === this.result?.sequence;
      return option;
    }));
  }
  load(result, autoplay) {
    this.pause();
    this.result = result;
    this.evaluation = result.trace.evaluation;
    this.events = this.evaluation.events;
    this.rules = new Map(this.evaluation.rules.map(rule => [rule.id, rule]));
    this.selected = null;
    this.pinnedRule = null;
    this.firingRule = null;
    this.sourceSpans = new Map([...this.rules.values()].map(rule => [rule.id, sourceSpan(this.evaluation.source?.text ?? '', rule.source_line)]));
    this.renderSource();
    this.position = 0;
    this.updateHistory();
    byId('round-input').textContent = `${result.trace.ms} ms native + transport`;
    byId('graph-empty').hidden = true;
    byId('wave-position').max = this.events.length;
    this.onSelect(result);
    this.resetState();
    this.drawGraph();
    this.drawState();
    if (autoplay && !matchMedia('(prefers-reduced-motion: reduce)').matches) this.play();
  }
  resetState() {
    this.state = new Map(Object.entries(this.evaluation.initial ?? this.evaluation.seeds).map(([name, rows]) => [name, new Map(rows.map(row => [key(row), row]))]));
  }
  apply(event) {
    if (!this.state.has(event.head)) this.state.set(event.head, new Map());
    const rows = this.state.get(event.head);
    for (const row of updates(event).removed) rows.delete(key(row));
    for (const row of updates(event).added) rows.set(key(row), row);
  }
  seek(position) {
    if (!this.events) return;
    this.position = Math.min(this.events.length, Math.max(0, position));
    this.resetState();
    for (const event of this.events.slice(0, this.position)) this.apply(event);
    this.clearActivity();
    this.drawState();
  }
  nextIndex() {
    let index = this.position;
    if (!byId('show-empty').checked) {
      while (index < this.events.length && isEmpty(this.events[index])) {
        this.apply(this.events[index++]);
      }
    }
    this.position = index;
    return index;
  }
  advanceOne() {
    if (!this.events) return;
    const index = this.nextIndex();
    if (index < this.events.length) {
      const event = this.events[index];
      this.apply(event);
      if (hasChanges(event)) this.pulseAccepted(event.head);
      this.position = index + 1;
    }
    this.clearActivity();
    this.drawState();
  }
  play() {
    if (!this.events?.length || !this.open) return;
    if (this.position >= this.events.length) this.seek(0);
    this.playing = true;
    byId('wave-play').textContent = 'Ⅱ Pause';
    this.animateNext();
  }
  pause() {
    this.playing = false;
    cancelAnimationFrame(this.frame);
    this.frame = null;
    byId('wave-play').textContent = '▶ Play';
    this.clearActivity();
  }
  animateNext() {
    if (!this.playing) return;
    const index = this.nextIndex();
    if (index >= this.events.length) {
      this.pause(); this.drawState();
      const latest = this.history.at(-1);
      if (this.follow && latest && latest.sequence !== this.result.sequence) this.load(latest, true);
      return;
    }
    const event = this.events[index];
    const rule = this.rules.get(event.rule);
    const activeEdges = this.edges.filter(edge => edge.rule === event.rule);
    this.clearActivity();
    for (const edge of activeEdges) edge.element.classList.add('edge-active');
    for (const read of rule.reads) this.cards.get(read.relation)?.classList.add('node-reading');
    this.showEvent(event);
    const added = new Set(updates(event).added.map(key));
    const duplicates = event.candidates.filter(row => !added.has(key(row)));
    const packets = activeEdges.flatMap(edge => {
      const entries = [];
      if (hasChanges(event) || !duplicates.length) entries.push({ edge, bounce: !hasChanges(event) && event.candidates.length > 0, color: colors[outcome(event)] });
      if (duplicates.length && hasChanges(event)) entries.push({ edge, bounce: true, color: colors.unchanged });
      if (!hasChanges(event) && event.candidates.length) entries.push({ edge, bounce: true, color: colors.unchanged });
      return entries;
    }).map(packet => {
      const element = node('circle', { r: 4, fill: packet.color, class: 'wave-packet' });
      this.packetLayer.append(element);
      return { ...packet, element };
    });
    const started = performance.now();
    let arrived = false;
    const animate = now => {
      if (!this.playing) return;
      const progress = Math.min(1, (now - started) / (650 / this.speed));
      for (const packet of packets) {
        const length = packet.edge.element.getTotalLength();
        const arrival = 0.58;
        // A rejected packet retreats only a few world pixels from its receiver.
        const retreat = Math.min(9 / Math.max(1, length), 0.1);
        let distance = progress <= arrival ? 1 - Math.pow(1 - progress / arrival, 2) :
          packet.bounce ? 1 - retreat * Math.sin(Math.min(1, (progress - arrival) / 0.32) * Math.PI / 2) : 1;
        if (isEmpty(event)) distance *= 0.6;
        const point = packet.edge.element.getPointAtLength(length * Math.max(0, distance));
        packet.element.setAttribute('cx', point.x); packet.element.setAttribute('cy', point.y);
        packet.element.dataset.progress = progress.toFixed(3);
        packet.element.dataset.distance = distance.toFixed(5);
        packet.element.dataset.bounce = String(packet.bounce);
        packet.element.setAttribute('opacity', isEmpty(event) ? 1 - progress : progress > 0.86 ? (1 - progress) / 0.14 : 1);
      }
      if (progress >= 0.58 && !arrived) {
        arrived = true;
        this.cards.get(event.head)?.classList.add(hasChanges(event) ? 'node-written' : event.candidates.length ? 'node-bounce' : 'node-empty');
        if (hasChanges(event)) this.pulseAccepted(event.head);
        else if (event.candidates.length) {
          clearTimeout(this.acceptedTimers.get(event.head));
          this.acceptedTimers.delete(event.head);
          this.cards.get(event.head)?.classList.remove('node-accepted');
        }
      }
      if (progress < 1) this.frame = requestAnimationFrame(animate);
      else {
        this.apply(event); this.position = index + 1;
        this.clearActivity(); this.drawState();
        this.animateNext();
      }
    };
    this.frame = requestAnimationFrame(animate);
  }
  clearActivity() {
    if (!this.cards) return;
    for (const card of this.cards.values()) card.classList.remove('node-reading', 'node-written', 'node-bounce', 'node-empty');
    for (const edge of this.edges) edge.element.classList.remove('edge-active');
    this.packetLayer.replaceChildren();
  }
  drawGraph() {
    if (!this.evaluation) return;
    this.svg.replaceChildren();
    const defs = node('defs');
    const marker = node('marker', { id: 'rule-arrow', viewBox: '0 0 10 10', refX: 9, refY: 5, markerWidth: 5, markerHeight: 5, orient: 'auto-start-reverse' });
    marker.append(node('path', { d: 'M 0 0 L 10 5 L 0 10 z', fill: '#5c7073' })); defs.append(marker); this.svg.append(defs);
    this.scene = node('g'); this.svg.append(this.scene);
    const edgeLayer = node('g'); this.packetLayer = node('g'); const cardLayer = node('g');
    this.scene.append(edgeLayer, cardLayer, this.packetLayer);
    const heads = new Set(this.evaluation.rules.map(rule => rule.head));
    const external = new Set(this.evaluation.external);
    const names = new Set([...Object.keys(this.evaluation.seeds), ...heads, ...this.evaluation.rules.flatMap(rule => rule.reads.map(read => read.relation))]);
    const showConstants = byId('show-constants').checked;
    const visible = [...names].filter(name => showConstants || heads.has(name) || external.has(name));
    const parents = new Map(visible.map(name => [name, this.evaluation.rules.filter(rule => rule.head === name).flatMap(rule => rule.reads.map(read => read.relation)).filter(name => visible.includes(name))]));
    const levels = new Map();
    const level = (name, visiting = new Set()) => {
      if (levels.has(name)) return levels.get(name);
      if (visiting.has(name)) return 0;
      const next = new Set(visiting).add(name);
      const result = Math.max(0, ...parents.get(name).map(parent => level(parent, next) + 1));
      levels.set(name, result); return result;
    };
    for (const name of visible) level(name);
    const columns = Array.from({ length: 5 }, () => []);
    visible.toSorted((a, b) => levels.get(a) - levels.get(b) || a.localeCompare(b)).forEach((name, index) => columns[Math.min(4, Math.floor(index / Math.ceil(visible.length / 5)))].push(name));
    // Reserve enough room for the largest fact set seen during this recorded round.
    const allRows = new Map(Object.entries(this.evaluation.initial ?? this.evaluation.seeds).map(([name, rows]) => [name, new Map(rows.map(row => [key(row), row]))]));
    for (const event of this.events) {
      if (!allRows.has(event.head)) allRows.set(event.head, new Map());
      for (const row of updates(event).added) allRows.get(event.head).set(key(row), row);
    }
    this.cardHeights = new Map(visible.map(name => {
      const count = allRows.get(name)?.size ?? 0;
      return [name, name === 'screen_cell' ? 158 : name === 'screen_text' ? 89 : name === 'screen_control' ? 82 : 30 + Math.max(1, Math.min(4, count)) * 12];
    }));
    const columnHeights = columns.map(column => column.reduce((sum, name) => sum + this.cardHeights.get(name) + 15, 0));
    const height = Math.max(480, ...columnHeights.map(height => height + 35));
    this.world = { width: 1115, height };
    this.points = new Map();
    columns.forEach((column, x) => {
      let y = (height - columnHeights[x]) / 2;
      for (const name of column) { this.points.set(name, { x: 20 + x * 220, y, name }); y += this.cardHeights.get(name) + 15; }
    });
    this.edges = [];
    for (const rule of this.evaluation.rules) for (const read of rule.reads) {
      const source = this.points.get(read.relation), target = this.points.get(rule.head);
      if (!source || !target) continue;
      const sx = source.x + cardWidth, sy = source.y + 17, tx = target.x, ty = target.y + 17;
      const span = Math.max(24, Math.abs(tx - sx) * 0.45);
      const d = target.x > source.x ? `M${sx},${sy} C${sx + span},${sy} ${tx - span},${ty} ${tx},${ty}` :
        `M${sx},${sy} C${sx + 27},${sy} ${tx + cardWidth + 27},${ty} ${tx + cardWidth},${ty}`;
      const element = node('path', { d, class: `rule-edge ${read.kind}`, 'marker-end': 'url(#rule-arrow)', 'data-rule': rule.id });
      const hit = node('path', { d, class: 'edge-hit', 'data-rule': rule.id, tabindex: 0, role: 'button', 'aria-label': `Open source of rule ${rule.id}` });
      const edge = { rule: rule.id, source, target, element, hit };
      hit.addEventListener('pointerenter', event => { element.classList.add('edge-hovered'); this.showRuleTooltip(rule, event); });
      hit.addEventListener('pointermove', event => this.positionRuleTooltip(event.clientX, event.clientY));
      hit.addEventListener('pointerleave', () => { element.classList.remove('edge-hovered'); this.hideRuleTooltip(); });
      hit.addEventListener('focus', () => this.showRuleTooltip(rule, null, edge));
      hit.addEventListener('blur', () => this.hideRuleTooltip());
      hit.addEventListener('click', event => { event.stopPropagation(); this.openRuleSource(rule); });
      hit.addEventListener('keydown', event => {
        if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); event.stopPropagation(); this.openRuleSource(rule); }
      });
      edgeLayer.append(element, hit); this.edges.push(edge);
    }
    this.cards = new Map();
    for (const [name, point] of this.points) {
      const card = node('g', { transform: `translate(${point.x},${point.y})`, class: `fact-node${external.has(name) ? ' input-node' : ''}`, tabindex: 0, role: 'button', 'aria-label': `Inspect ${name}`, 'data-relation': name });
      card.append(node('rect', { width: cardWidth, height: this.cardHeights.get(name), rx: 5, class: 'node-background' }));
      card.append(node('text', { x: 8, y: 15, class: 'node-name' }, name.length > 20 ? name.slice(0, 19) + '…' : name));
      const sigil = node('g', { transform: `translate(${cardWidth - 29},5)`, class: 'node-sigil' });
      sigil.append(node('rect', { width: 23, height: 15, rx: 4 }));
      sigil.append(node('text', { x: 11.5, y: 10.5, 'text-anchor': 'middle', class: 'sigil-count' }, '∅'));
      sigil.append(node('title', {}, 'No facts'));
      card.append(sigil, node('g', { class: 'node-fact-preview' }));
      card.append(node('title', {}, name));
      const select = () => { this.selected = this.selected === name ? null : name; this.drawState(); };
      card.addEventListener('click', select);
      card.addEventListener('keydown', event => { if (event.key === 'Enter') select(); });
      cardLayer.append(card); this.cards.set(name, card);
    }
    this.fit();
  }
  fit() {
    if (!this.world) return;
    const bounds = this.svg.getBoundingClientRect();
    if (!bounds.width || !bounds.height) return;
    const scale = Math.min(bounds.width / this.world.width, bounds.height / this.world.height) * 0.94;
    this.transform = { x: (bounds.width - this.world.width * scale) / 2, y: (bounds.height - this.world.height * scale) / 2, scale };
    this.transformScene();
  }
  transformScene() {
    if (this.scene && this.transform) this.scene.setAttribute('transform', `translate(${this.transform.x},${this.transform.y}) scale(${this.transform.scale})`);
  }
  zoom(factor) {
    if (!this.transform) return;
    const bounds = this.svg.getBoundingClientRect(), x = bounds.width / 2, y = bounds.height / 2;
    const next = Math.min(3.5, Math.max(0.15, this.transform.scale * factor));
    const ratio = next / this.transform.scale;
    this.transform.x = x - (x - this.transform.x) * ratio;
    this.transform.y = y - (y - this.transform.y) * ratio;
    this.transform.scale = next;
    this.transformScene();
  }
  attachNavigation() {
    let drag;
    this.svg.addEventListener('pointerdown', event => {
      if (event.target.closest('.fact-node, .edge-hit') || !this.transform) return;
      drag = { x: event.clientX, y: event.clientY, tx: this.transform.x, ty: this.transform.y };
      this.svg.setPointerCapture(event.pointerId);
    });
    this.svg.addEventListener('pointermove', event => {
      if (!drag) return;
      this.transform.x = drag.tx + event.clientX - drag.x; this.transform.y = drag.ty + event.clientY - drag.y; this.transformScene();
    });
    this.svg.addEventListener('pointerup', () => { drag = null; });
    new ResizeObserver(() => this.fit()).observe(this.svg);
  }
  drawState() {
    if (!this.state) return;
    for (const [name, card] of this.cards) {
      const rows = [...(this.state.get(name)?.values() ?? [])];
      const count = rows.length;
      card.querySelector('.sigil-count').textContent = count ? `◆${count}` : '∅';
      card.querySelector('.node-sigil title').textContent = `${count} fact${count === 1 ? '' : 's'}`;
      this.drawNodeFacts(card, name, rows);
      card.classList.toggle('has-facts', count > 0); card.classList.toggle('selected', this.selected === name);
    }
    byId('wave-position').value = this.position;
    const hidden = byId('show-empty').checked ? '' : ` · ${this.events.filter(isEmpty).length} empty checks not animated`;
    byId('wave-counter').textContent = `${this.position} / ${this.events.length} evaluations${hidden}`;
    const event = this.events[this.position - 1];
    if (event) this.showEvent(event);
    else {
      byId('wave-status').textContent = `Current facts loaded in ${this.state.size} relations. No updates applied yet.`;
      if (!this.selected) {
        byId('detail-title').textContent = 'Select a fact set';
        byId('detail-count').textContent = '';
        byId('detail-rule').textContent = '';
        byId('detail-facts').replaceChildren();
        byId('detail-visual').hidden = true;
        this.firingRule = null;
        byId('firing-rule-location').textContent = '';
        byId('firing-rule-source').textContent = 'Start playback to follow the source.';
        this.updateSourceSelection();
      }
    }
    this.showDetails(this.selected ?? event?.head, this.selected ? [...this.events.slice(0, this.position)].reverse().find(event => event.head === this.selected) : event);
  }
  showEvent(event) {
    const rule = this.rules.get(event.rule);
    this.firingRule = rule.id;
    const source = this.sourceSpans.get(rule.id);
    byId('firing-rule-location').textContent = `#${rule.id} · ${this.evaluation.source?.name ?? 'native'}${source ? `:${source.start}` : ''}`;
    highlightSnippet(byId('firing-rule-source'), source?.text ?? rule.text);
    this.updateSourceSelection();
    const added = new Set(updates(event).added.map(key));
    const duplicates = event.candidates.filter(row => !added.has(key(row))).length;
    const message = isEmpty(event) ? 'no matching candidates' : hasChanges(event) ?
      `${updates(event).added.length} added · ${updates(event).removed.length} removed` :
      `${event.candidates.length} existing facts unchanged`;
    byId('wave-status').textContent = `Rule ${event.rule} → ${event.head}: ${event.cache_hit ? `unchanged inputs → rule query skipped · ${event.candidates.length} facts retained` : message}.`;
    byId('wave-status').dataset.outcome = outcome(event);
    if (!this.selected) this.showDetails(event.head, event);
    for (const edge of this.edges) edge.element.classList.toggle('edge-selected', edge.rule === rule.id);
  }
  showDetails(relation, event) {
    if (!relation) return;
    const rows = [...(this.state.get(relation)?.values() ?? [])];
    byId('detail-title').textContent = relation;
    byId('detail-count').textContent = `${rows.length} facts${!rows.length && event?.candidates.length ? ' · candidate preview' : ' at this point'}`;
    highlightSnippet(byId('detail-rule'), event ? this.sourceSpans.get(event.rule)?.text ?? this.rules.get(event.rule).text : 'Literal input facts');
    this.drawDetailVisual(relation, rows);
    const container = byId('detail-facts'); container.replaceChildren();
    const added = new Set(event ? updates(event).added.map(key) : []);
    const candidates = new Set(event?.candidates.map(key) ?? []);
    const displayRows = rows.length ? rows : event?.candidates ?? [];
    if (!displayRows.length) container.textContent = '∅ No facts in this relation.';
    for (const row of displayRows) {
      const line = document.createElement('span');
      line.className = `fact-line ${event?.cache_hit && candidates.has(key(row)) ? 'duplicate' : added.has(key(row)) ? 'added' : candidates.has(key(row)) ? 'duplicate' : ''}`;
      line.textContent = `${event?.cache_hit && candidates.has(key(row)) ? '↶ ' : added.has(key(row)) ? '+ ' : candidates.has(key(row)) ? '↶ ' : '  '}${literal(relation, row)}`; container.append(line);
    }
  }

  pulseAccepted(relation) {
    const card = this.cards.get(relation);
    if (!card) return;
    clearTimeout(this.acceptedTimers.get(relation));
    card.classList.remove('node-accepted');
    void card.getBoundingClientRect();
    card.classList.add('node-accepted');
    this.acceptedTimers.set(relation, setTimeout(() => { card.classList.remove('node-accepted'); this.acceptedTimers.delete(relation); }, 1500));
  }
  drawNodeFacts(card, relation, rows) {
    const preview = card.querySelector('.node-fact-preview');
    preview.replaceChildren();
    if (relation === 'screen_cell') {
      this.drawScreenSVG(preview, rows);
      return;
    }
    if (relation === 'screen_text') {
      rows.slice(0, 4).forEach(([id, value], i) => {
        const x = 8 + i % 2 * 80, y = 24 + Math.floor(i / 2) * 28;
        const tile = node('g', { class: 'screen-text-tile' });
        tile.append(node('rect', { x, y, width: 75, height: 24, rx: 3 }));
        tile.append(node('text', { x: x + 4, y: y + 9, class: 'tile-key' }, String(id).slice(0, 12)));
        tile.append(node('text', { x: x + 4, y: y + 19, class: 'tile-value' }, String(value).slice(0, 13)));
        tile.append(node('title', {}, literal(relation, [id, value]))); preview.append(tile);
      });
      if (!rows.length) preview.append(node('text', { x: 8, y: 38, class: 'tuple-empty' }, '∅'));
      return;
    }
    const displayed = rows.length <= 4 ? rows : rows.slice(0, 3);
    if (!rows.length) preview.append(node('text', { x: 8, y: 36, class: 'tuple-empty' }, '∅'));
    displayed.forEach((row, index) => {
      const tuple = `(${row.map(value => JSON.stringify(value)).join(', ')})`;
      const text = node('text', { x: 8, y: 32 + index * 12, class: 'node-tuple' }, tuple.length > 26 ? tuple.slice(0, 25) + '…' : tuple);
      text.append(node('title', {}, literal(relation, row))); preview.append(text);
    });
    if (rows.length > 4) preview.append(node('text', { x: 8, y: 68, class: 'tuple-empty' }, '…'));
  }
  screenAreas() {
    const areas = new Map();
    for (const [area, x, y] of this.result.screen.screen_cell) {
      const size = areas.get(area) ?? { width: 0, height: 0 };
      size.width = Math.max(size.width, x + 1); size.height = Math.max(size.height, y + 1); areas.set(area, size);
    }
    return areas;
  }
  drawScreenSVG(container, rows) {
    let offset = 8;
    for (const [area, { width, height }] of this.screenAreas()) {
      const size = Math.min(6, 120 / height, 84 / width);
      const grid = node('g', { transform: `translate(${offset},29)`, class: 'screen-mini-grid', 'data-area': area });
      grid.append(node('rect', { x: 0, y: 0, width: width * size, height: height * size, class: 'screen-grid-background' }));
      for (const row of rows.filter(row => row[0] === area)) {
        const [, x, y, color, style] = row;
        const cell = node('rect', { x: x * size, y: y * size, width: size - 0.35, height: size - 0.35, class: `cell svg-screen-cell ${color} ${style}` });
        cell.append(node('title', {}, literal('screen_cell', row))); grid.append(cell);
      }
      grid.append(node('text', { x: 0, y: -5, class: 'grid-area-label' }, area)); container.append(grid); offset += width * size + 12;
    }
  }
  drawDetailVisual(relation, rows) {
    const visual = byId('detail-visual'); visual.replaceChildren();
    const spatial = relation === 'screen_cell', textGrid = relation === 'screen_text', controls = relation === 'screen_control';
    visual.hidden = !spatial && !textGrid && !controls;
    if (this.detailRelation !== relation) byId('raw-facts').open = visual.hidden;
    this.detailRelation = relation;
    if (visual.hidden) return;
    if (spatial) {
      for (const [area, { width, height }] of this.screenAreas()) {
        const wrapper = document.createElement('div'); wrapper.className = 'fact-grid-area';
        const label = document.createElement('span'); label.textContent = area; wrapper.append(label);
        const grid = document.createElement('div'); grid.className = 'fact-grid'; grid.dataset.area = area;
        grid.style.gridTemplateColumns = `repeat(${width}, var(--fact-grid-cell))`;
        grid.style.gridTemplateRows = `repeat(${height}, var(--fact-grid-cell))`;
        for (const row of rows.filter(row => row[0] === area)) {
          const [, x, y, color, style] = row;
          const cell = document.createElement('div'); cell.className = `cell ${color} ${style}`;
          cell.style.gridColumn = x + 1; cell.style.gridRow = y + 1; cell.title = literal(relation, row); grid.append(cell);
        }
        wrapper.append(grid); visual.append(wrapper);
      }
    } else {
      visual.classList.add('text-fact-grid');
      for (const row of rows) {
        const tile = document.createElement('div'); tile.className = 'text-fact-tile';
        const name = document.createElement('span'), value = document.createElement('strong');
        name.textContent = String(row[0]); value.textContent = row.slice(1).join(' · '); tile.title = literal(relation, row); tile.append(name, value); visual.append(tile);
      }
    }
    visual.classList.toggle('text-fact-grid', !spatial);
  }
  renderSource() {
    const text = this.evaluation.source?.text ?? '';
    if (this.sourceText === text) return;
    this.sourceText = text;
    const container = byId('rule-source'); container.replaceChildren();
    if (!text) { container.textContent = 'Source locations are unavailable for this recorded round.'; return; }
    renderSourceLines(container, text);
  }

  updateSourceSelection(scroll = false) {
    const firing = this.sourceSpans?.get(this.firingRule), pinned = this.sourceSpans?.get(this.pinnedRule);
    const selected = pinned ?? firing;
    const id = this.pinnedRule ?? this.firingRule;
    const file = this.evaluation.source?.name ?? 'Native lowered rule';
    byId('source-location').textContent = `${file}${selected ? `:${selected.start}${selected.end !== selected.start ? `–${selected.end}` : ''}` : ''}`;
    byId('follow-rule-source').setAttribute('aria-pressed', String(this.pinnedRule === null));
    byId('follow-rule-source').textContent = this.pinnedRule === null ? 'Following firing rule' : 'Follow firing rule';
    byId('source-caption').textContent = id !== null ? `${this.pinnedRule === null ? 'Following' : 'Pinned'} rule #${id} → ${this.rules.get(id)?.head ?? ''}` : 'Hover an edge to preview its rule. Click to pin its source here.';
    for (const line of byId('rule-source').children) {
      const number = Number(line.dataset.line);
      line.classList.toggle('firing-source-line', Boolean(firing && number >= firing.start && number <= firing.end));
      line.classList.toggle('selected-source-line', Boolean(pinned && number >= pinned.start && number <= pinned.end));
    }
    for (const edge of this.edges ?? []) edge.element.classList.toggle('edge-pinned', edge.rule === this.pinnedRule);
    if (selected) {
      const container = byId('rule-source'), line = container.querySelector(`[data-line="${selected.start}"]`);
      if (line && (scroll || this.pinnedRule === null)) {
        const top = line.offsetTop;
        if (scroll || top < container.scrollTop || top + line.offsetHeight > container.scrollTop + container.clientHeight) container.scrollTop = top - 35;
      }
    }
  }
  openRuleSource(rule) {
    this.pause(); this.hideRuleTooltip(); this.pinnedRule = rule.id;
    this.selected = rule.head;
    this.drawState(); this.updateSourceSelection(true);
    byId('rule-source-view').scrollIntoView({ block: 'nearest' });
  }
  showRuleTooltip(rule, event, edge) {
    const source = this.sourceSpans.get(rule.id);
    byId('rule-tooltip-location').textContent = `Rule #${rule.id} · ${this.evaluation.source?.name ?? 'native'}${source ? `:${source.start}` : ''}`;
    highlightSnippet(byId('rule-tooltip-source'), source?.text ?? rule.text);
    byId('rule-tooltip').hidden = false;
    if (event) this.positionRuleTooltip(event.clientX, event.clientY);
    else {
      const point = edge.element.getPointAtLength(edge.element.getTotalLength() / 2);
      const matrix = edge.element.getScreenCTM();
      const client = new DOMPoint(point.x, point.y).matrixTransform(matrix); this.positionRuleTooltip(client.x, client.y);
    }
  }
  positionRuleTooltip(x, y) {
    const tooltip = byId('rule-tooltip');
    if (tooltip.hidden) return;
    const rect = tooltip.getBoundingClientRect();
    tooltip.style.left = `${Math.max(10, Math.min(innerWidth - rect.width - 10, x + 14))}px`;
    tooltip.style.top = `${Math.max(10, Math.min(innerHeight - rect.height - 10, y + 14))}px`;
  }
  hideRuleTooltip() { byId('rule-tooltip').hidden = true; }

}
