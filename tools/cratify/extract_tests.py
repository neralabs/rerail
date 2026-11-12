"""Moves inline test modules into files of their own, text unchanged: `#[cfg(test)] mod NAME { BODY }` in a/b.rs
becomes `mod NAME;` and a/b/NAME.rs holds BODY as written (a/mod.rs: a/NAME.rs; an enclosing inline module's name
joins the path). The crate split mounts a file module where its crate map says (a test the engine runs, say); an inline
one can only stay with its file.

    python3 tools/cratify/extract_tests.py <source root> FILE:NAME...    (FILE under the root)

A relative `include_str!`/`include_bytes!` path gains a `../`. Prints each file made, or why a module was left."""
import os, re, sys


def block_end(text, open_at):
    """The index of the `}` closing the `{` at `open_at`: strings, raw strings, chars and comments skipped."""
    depth, i, n = 0, open_at, len(text)
    while i < n:
        c = text[i]
        if text.startswith('//', i):
            i = text.find('\n', i)
            i = n if i < 0 else i
        elif text.startswith('/*', i):
            level, i = 1, i + 2
            while i < n and level:
                if text.startswith('/*', i): level, i = level + 1, i + 2
                elif text.startswith('*/', i): level, i = level - 1, i + 2
                else: i += 1
            continue
        elif m := re.match(r'b?r(#*)"', text[i:i + 300]) if c in 'rb' and (i == 0 or not (text[i - 1].isalnum() or text[i - 1] == '_')) else None:
            close = '"' + m.group(1)
            i = text.index(close, i + m.end()) + len(close)
            continue
        elif c == '"':
            i += 1
            while text[i] != '"':
                i += 2 if text[i] == '\\' else 1
        elif c == "'":
            m = re.match(r"'(\\(x..|u\{[0-9a-fA-F]+\}|.)|[^\\'])'", text[i:i + 12], re.S)
            if m:
                i += m.end()
                continue
        elif c == '{':
            depth += 1
        elif c == '}':
            depth -= 1
            if depth == 0:
                return i
        i += 1
    raise ValueError('unbalanced')


root = sys.argv[1].rstrip('/') + '/'
for arg in sys.argv[2:]:
    rel, name = arg.rsplit(':', 1)
    text = open(root + rel).read()
    heads = [m for m in re.finditer(r'(?m)^([ \t]*)((?:pub(?:\([^)]*\))?\s+)?mod ' + name + r')\s*\{', text)
             if 'test' in text[max(0, text.rfind('\n', 0, max(0, m.start() - 1)) - 200):m.start()]]
    if not heads and 'test' in os.path.basename(rel):
        # a module of a test-only file needs no cfg of its own
        heads = list(re.finditer(r'(?m)^([ \t]*)((?:pub(?:\([^)]*\))?\s+)?mod ' + name + r')\s*\{', text))
    if len(heads) != 1:
        print('left', arg, len(heads), 'candidates'); continue
    head = heads[0]
    end = block_end(text, head.end() - 1)
    # the inline modules enclosing it
    outer = []
    for m in re.finditer(r'(?m)^[ \t]*(?:pub(?:\([^)]*\))?\s+)?mod (\w+)\s*\{', text[:head.start()]):
        if block_end(text, m.end() - 1) > end:
            outer.append(m.group(1))
    base = rel[:-3]
    base = os.path.dirname(base) if base.endswith('/mod') or base.endswith('/lib') else base
    target = os.path.join(base, *outer, name + '.rs')
    if os.path.exists(root + target) or os.path.exists(root + target[:-3] + '/mod.rs'):
        print('left', arg, target, 'exists'); continue
    body = text[head.end():end].strip('\n')
    # a relative include is read from the file's directory, now one deeper
    body = re.sub(r'(include_(?:str|bytes)!\(\s*")(?![/])', r'\1../', body)
    os.makedirs(os.path.dirname(root + target), exist_ok=True)
    open(root + target, 'w').write(body + '\n')
    open(root + rel, 'w').write(text[:head.start()] + head.group(1) + head.group(2) + ';' + text[end + 1:])
    print('made', target)
