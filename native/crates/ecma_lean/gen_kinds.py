"""Writes src/kinds.rs: the node kinds of tree-sitter-javascript, -typescript and -tsx (Rerail's
vendored, patched grammars), named and anonymous, as the lean parser's `K` and `KINDS`.

    python3 native/crates/ecma_lean/gen_kinds.py

A dev tool: the generated file is checked in, and the build does not run this."""
import json, os, re

here = os.path.dirname(os.path.abspath(__file__))
grammars = os.path.join(here, '..', '..', 'grammars')
paths = ['tree-sitter-javascript/src/grammar.json', 'tree-sitter-typescript/typescript/src/grammar.json', 'tree-sitter-typescript/tsx/src/grammar.json']

named, anon, fields = [], [], []

def add(lst, v):
    if v not in lst:
        lst.append(v)

def walk(r):
    t = r['type']
    if t in ('TOKEN', 'IMMEDIATE_TOKEN'):
        # A token's own pieces are not tokens; an aliased token is (below).
        return
    if t == 'FIELD':
        add(fields, r['name'])
    if t == 'STRING':
        add(anon, r['value'])
    elif t == 'ALIAS':
        add(named if r['named'] else anon, r['value'])
        if r['content']['type'] not in ('STRING', 'TOKEN', 'IMMEDIATE_TOKEN', 'PATTERN'):
            walk(r['content'])
        return
    for m in r.get('members', []):
        walk(m)
    if 'content' in r:
        walk(r['content'])

for p in paths:
    g = json.load(open(os.path.join(grammars, p)))
    sup = set(g.get('supertypes', []))
    for k, v in g['rules'].items():
        if not k.startswith('_') and k not in sup:
            add(named, k)
        walk(v)
    for e in g['externals']:
        if e['type'] == 'SYMBOL' and not e['name'].startswith('_'):
            add(named, e['name'])
        elif e['type'] == 'STRING':
            add(anon, e['value'])
# A named kind that is also a keyword's text (`this`, `true`): the named one is the kind.
words = {'!': 'Bang', '!=': 'Ne', '!==': 'NeEq', '"': 'DQuote', '#': 'Hash', '${': 'DollarBrace', '%': 'Percent', '%=': 'PercentEq',
         '&': 'Amp', '&&': 'AmpAmp', '&&=': 'AmpAmpEq', '&=': 'AmpEq', "'": 'SQuote', '(': 'LParen', ')': 'RParen', '*': 'Star',
         '**': 'StarStar', '**=': 'StarStarEq', '*=': 'StarEq', '+': 'Plus', '++': 'PlusPlus', '+=': 'PlusEq', '+?:': 'PlusQColon',
         ',': 'Comma', '-': 'Minus', '--': 'MinusMinus', '-=': 'MinusEq', '-?:': 'MinusQColon', '.': 'Dot', '...': 'Ellipsis',
         '/': 'Slash', '/=': 'SlashEq', '/>': 'SlashGt', ':': 'Colon', ';': 'Semi', '<': 'Lt', '</': 'LtSlash', '<<': 'LtLt',
         '<<=': 'LtLtEq', '<=': 'Le', '=': 'Eq', '==': 'EqEq', '===': 'EqEqEq', '=>': 'Arrow', '>': 'Gt', '>=': 'Ge', '>>': 'GtGt',
         '>>=': 'GtGtEq', '>>>': 'GtGtGt', '>>>=': 'GtGtGtEq', '?': 'Question', '?.': 'QDot', '?:': 'QColon', '??': 'QQ', '??=': 'QQEq',
         '@': 'At', '[': 'LBracket', '\\': 'Backslash', ']': 'RBracket', '^': 'Caret', '^=': 'CaretEq', '`': 'Backtick', '{': 'LBrace',
         '{|': 'LBracePipe', '|': 'Pipe', '|=': 'PipeEq', '||': 'PipePipe', '||=': 'PipePipeEq', '|}': 'PipeRBrace', '}': 'RBrace', '~': 'Tilde',
         'unique symbol': 'UniqueSymbol'}

def camel(s):
    return ''.join(w[:1].upper() + w[1:] for w in re.split(r'[_ ]', s) if w)

rows = [(camel(n), n, True) for n in named]
seen = {r[0] for r in rows}
for a in anon:
    v = 'T' + (words.get(a) or camel(a))
    assert re.fullmatch(r'T[A-Za-z0-9]+', v), a
    assert v not in seen, a
    seen.add(v)
    rows.append((v, a, False))

out = ['// Written by gen_kinds.py from the vendored grammars; do not edit.', '',
       '/// Node kinds, as the tape names them: tree-sitter-javascript\'s, -typescript\'s and -tsx\'s, named', '/// and anonymous (a dialect has a subset).',
       '#[derive(Clone, Copy, Debug, PartialEq, Eq)]', '#[repr(u16)]', 'pub enum K {']
out += [f'    {v},' for v, _, _ in rows]
out += ['}', '', '/// Each kind\'s name and whether it is named, in `K`\'s order.', 'pub const KINDS: &[(&str, bool)] = &[']
out += [f'    ({json.dumps(n)}, {str(b).lower()}),' for _, n, b in rows]
out += ['];', '', f'/// The last kind.', f'pub const LAST: K = K::{rows[-1][0]};', '']
fields.sort()
out += ['/// Fields, as the tape names them; 0 is no field.', 'pub const FIELDS: &[&str] = &[', '    "",']
out += [f'    "{f}",' for f in fields]
out += ['];', '', '/// Field ids, in `FIELDS`\' order.', 'pub mod f {', '    pub const NONE: u8 = 0;']
out += [f'    pub const {f.upper()}: u8 = {i + 1};' for i, f in enumerate(fields)]
out += ['}', '']
open(os.path.join(here, 'src', 'kinds.rs'), 'w').write('\n'.join(out))
print(len(rows), 'kinds')
