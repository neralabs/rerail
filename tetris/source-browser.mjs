import { renderSourceLines } from './rr-source.mjs';
import { driver } from './transport.mjs';

export class SourceBrowser {
  constructor() {
    this.dialog = document.getElementById('source-browser');
    this.code = document.getElementById('source-browser-code');
    this.files = [];
    this.matches = [];
    this.matchIndex = -1;
    this.request = 0;
    document.getElementById('close-source-browser').addEventListener('click', () => this.dialog.close());
    document.getElementById('source-file-filter').addEventListener('input', () => this.drawFiles());
    document.getElementById('source-text-search').addEventListener('input', () => this.search());
    document.getElementById('source-search-next').addEventListener('click', () => this.nextMatch(1));
    document.getElementById('source-search-previous').addEventListener('click', () => this.nextMatch(-1));
    document.getElementById('source-text-search').addEventListener('keydown', event => {
      if (event.key === 'Enter') { event.preventDefault(); this.nextMatch(event.shiftKey ? -1 : 1); }
    });
    document.getElementById('source-go-line').addEventListener('keydown', event => {
      if (event.key === 'Enter') { event.preventDefault(); this.jump(Number(event.target.value)); }
    });
    document.getElementById('source-line-go').addEventListener('click', () => this.jump(Number(document.getElementById('source-go-line').value)));
    document.getElementById('source-wrap').addEventListener('change', event => this.code.classList.toggle('wrap-lines', event.target.checked));
    this.dialog.addEventListener('click', event => { if (event.target === this.dialog) this.dialog.close(); });
  }
  async open({ path = this.current?.path ?? 'game.rr', line } = {}) {
    if (!this.dialog.open) this.dialog.showModal();
    if (this.current?.path !== path) document.getElementById('source-file-filter').value = '';
    try {
      if (!this.files.length) {
        document.getElementById('source-browser-status').textContent = 'Loading committed .rr sources…';
        const catalog = await driver.sourceList(); this.files = catalog.files;
        document.getElementById('source-browser-revision').textContent = catalog.revision.slice(0, 9);
      }
      this.drawFiles();
      if (this.current?.path !== path) await this.select(path, line);
      else if (line) this.jump(line);
    } catch (error) { document.getElementById('source-browser-status').textContent = error.message; }
  }
  drawFiles() {
    const filter = document.getElementById('source-file-filter').value.toLowerCase();
    const tree = document.getElementById('source-files'); tree.replaceChildren();
    const filtered = this.files.filter(file => file.path.toLowerCase().includes(filter));
    for (const group of new Set(this.files.map(file => file.group))) {
      const files = filtered.filter(file => file.group === group);
      if (!files.length) continue;
      const section = document.createElement('section'), heading = document.createElement('h3');
      heading.textContent = group + ' · ' + files.length; section.append(heading);
      for (const file of files) {
        const button = document.createElement('button'); button.type = 'button'; button.className = 'source-file';
        button.dataset.path = file.path; button.title = file.path; button.textContent = file.label;
        button.setAttribute('aria-current', String(this.current?.path === file.path));
        button.addEventListener('click', () => this.select(file.path)); section.append(button);
      }
      tree.append(section);
    }
    document.getElementById('source-browser-file-count').textContent = this.files.length + ' .rr files';
  }
  async select(path, line) {
    const request = ++this.request;
    try {
      document.getElementById('source-browser-status').textContent = 'Loading ' + path;
      const file = await driver.readSource(path);
      if (request !== this.request) return;
      this.current = file; this.lines = file.text.split('\n');
      renderSourceLines(this.code, file.text);
      this.code.scrollTop = 0; this.code.scrollLeft = 0;
      document.getElementById('source-browser-path').textContent = path;
      document.getElementById('source-browser-kind').textContent = file.executed ? 'RUNNING GAME PROGRAM' : 'PINNED ENGINE REFERENCE';
      document.getElementById('source-browser-status').textContent = this.lines.length + ' lines · ' +
        (file.executed ? 'Complete program evaluated by this game.' : 'Engine source at the displayed revision. This game does not import this file.');
      document.getElementById('source-go-line').max = this.lines.length;
      this.drawFiles(); this.search(false);
      if (line) this.jump(line);
    } catch (error) {
      if (request === this.request) document.getElementById('source-browser-status').textContent = error.message;
    }
  }
  search(scroll = true) {
    const query = document.getElementById('source-text-search').value.toLowerCase();
    this.matches = query ? this.lines?.flatMap((line, index) => line.toLowerCase().includes(query) ? [index + 1] : []) ?? [] : [];
    this.matchIndex = this.matches.length ? 0 : -1;
    const matches = new Set(this.matches);
    for (const line of this.code.children) line.classList.toggle('source-search-match', matches.has(Number(line.dataset.line)));
    this.updateSearchCount();
    if (scroll && this.matches.length) this.jump(this.matches[0]);
  }
  nextMatch(direction) {
    if (!this.matches.length) return;
    this.matchIndex = (this.matchIndex + direction + this.matches.length) % this.matches.length;
    this.updateSearchCount(); this.jump(this.matches[this.matchIndex]);
  }
  updateSearchCount() {
    document.getElementById('source-search-count').textContent = this.matches.length ? (this.matchIndex + 1) + ' / ' + this.matches.length : '0 matches';
  }
  jump(number) {
    const value = Math.max(1, Math.min(this.lines?.length ?? 1, Math.round(number || 1)));
    for (const line of this.code.children) line.classList.toggle('source-browser-selected', Number(line.dataset.line) === value);
    const line = this.code.querySelector('[data-line="' + value + '"]');
    if (line) this.code.scrollTop = Math.max(0, line.offsetTop - 60);
    document.getElementById('source-go-line').value = value;
  }
}
