// Display-only lexer for .rr source. It never evaluates a rule.
const keywords = new Set(['count', 'sum', 'min', 'max', 'mean', 'join', 'by', 'sep', 'must', 'may', 'widen', 'merge', 'ordered', 'true', 'false']);
const types = new Set(['number', 'symbol', 'node', 'int', 'sym', 'span', 'loc', 'flat', 'unsigned', 'float']);

export function sourceTokens(line, state = { blockComment: false }) {
  const tokens = [];
  let i = 0, declaration = false;
  const push = (text, kind = '') => tokens.push({ text, kind });
  while (i < line.length) {
    const rest = line.slice(i);
    if (state.blockComment || rest.startsWith('/*')) {
      const end = line.indexOf('*/', i + (state.blockComment ? 0 : 2));
      const stop = end < 0 ? line.length : end + 2;
      push(line.slice(i, stop), 'comment'); state.blockComment = end < 0; i = stop; continue;
    }
    if (rest.startsWith('//')) { push(rest, 'comment'); break; }
    if (line[i] === '"' || line[i] === "'" || line.charCodeAt(i) === 96) {
      const quote = line[i], start = i++;
      while (i < line.length) { if (line[i] === '\\') i += 2; else if (line[i++] === quote) break; }
      push(line.slice(start, i), 'string'); continue;
    }
    const directive = rest.match(/^(?:\.[A-Za-z_][\w-]*|#include)\b/);
    if (directive) { push(directive[0], 'directive'); declaration = directive[0] === '.decl'; i += directive[0].length; continue; }
    const number = rest.match(/^-?\d+(?:\.\d+)?/);
    if (number) { push(number[0], 'number'); i += number[0].length; continue; }
    const word = rest.match(/^[$@]?[A-Za-z_][\w]*/);
    if (word) {
      const text = word[0], next = line.slice(i + text.length).trimStart();
      const kind = declaration || next.startsWith('(') ? 'relation' : keywords.has(text) ? 'keyword' : types.has(text) ? 'type' : 'variable';
      push(text, kind); declaration = false; i += text.length; continue;
    }
    const operator = rest.match(/^(?::-|=>|!=|<=|>=|[=!<>+*/%&|~-])/);
    if (operator) { push(operator[0], 'operator'); i += operator[0].length; continue; }
    push(line[i++]);
  }
  return tokens;
}

function appendTokens(container, tokens) {
  for (const { text, kind } of tokens) {
    if (!kind) container.append(document.createTextNode(text));
    else { const span = document.createElement('span'); span.className = 'syntax-' + kind; span.textContent = text; container.append(span); }
  }
}

export function highlightSnippet(container, text) {
  container.replaceChildren();
  const state = { blockComment: false };
  text.split('\n').forEach((line, index) => {
    if (index) container.append(document.createTextNode('\n'));
    appendTokens(container, sourceTokens(line, state));
  });
}

export function renderSourceLines(container, text) {
  container.replaceChildren();
  const state = { blockComment: false };
  const fragment = document.createDocumentFragment();
  text.split('\n').forEach((text, index) => {
    const line = document.createElement('span'); line.className = 'source-line'; line.dataset.line = index + 1;
    const number = document.createElement('span'); number.className = 'source-line-number'; number.textContent = index + 1; number.setAttribute('aria-hidden', 'true');
    const code = document.createElement('span'); code.className = 'source-line-code';
    appendTokens(code, sourceTokens(text, state));
    if (!text) code.append(document.createTextNode(' '));
    line.append(number, code); fragment.append(line);
  });
  container.append(fragment);
}
