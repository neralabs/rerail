"""Writes src/kinds.rs: the node kinds of tree-sitter-java (the token grammar the engine reads,
`native/grammars/token-grammars/java`), named and anonymous, as the lean parser's `K` and `KINDS`.

    python3 native/crates/java_lean/gen_kinds.py

With them, `DROPPED`: the tokens a tape leaves out under each kind, from `implied.tsv`, which the
engine's automata write (`cargo test … ecma_implied_tokens --ignored` with
`RERAIL_IMPLIED_DIALECTS=java RERAIL_ECMA_IMPLIED=native/crates/java_lean/implied.tsv`): the parser
stores a token by the grammar, as the engine's tape writer does, not by hand.

A dev tool: the generated file is checked in, and the build does not run this."""
import json, os, re

here = os.path.dirname(os.path.abspath(__file__))
grammar = os.path.join(here, '..', '..', 'grammars', 'token-grammars', 'java', 'grammar.json')

named, anon, fields = [], [], []

def add(lst, v):
    if v not in lst:
        lst.append(v)

def walk(r):
    t = r['type']
    if t in ('TOKEN', 'IMMEDIATE_TOKEN'):
        # A token's own pieces are not tokens (but a string inside one is its text: `tok{prec{"<"}}`).
        c = r['content']
        while c['type'].startswith('PREC'):
            c = c['content']
        if c['type'] == 'STRING':
            add(anon, c['value'])
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

g = json.load(open(grammar))
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
words = {'!': 'Bang', '!=': 'Ne', '"': 'DQuote', '#': 'Hash', '$': 'Dollar', '%': 'Percent', '%=': 'PercentEq', '&': 'Amp',
         '&&': 'AmpAmp', '&=': 'AmpEq', "'": 'Quote', '(': 'LParen', ')': 'RParen', '*': 'Star', '*=': 'StarEq', '*/': 'StarSlash',
         '+': 'Plus', '+=': 'PlusEq', ',': 'Comma', '-': 'Minus', '-=': 'MinusEq', '->': 'Arrow', '.': 'Dot', '..': 'DotDot',
         '...': 'DotDotDot', '..=': 'DotDotEq', '/': 'Slash', '/*': 'SlashStar', '//': 'SlashSlash', '/=': 'SlashEq', ':': 'Colon',
         '::': 'ColonColon', ';': 'Semi', '<': 'Lt', '<<': 'LtLt', '<<=': 'LtLtEq', '<=': 'Le', '=': 'Eq', '==': 'EqEq', '=>': 'FatArrow',
         '>': 'Gt', '>=': 'Ge', '>>': 'GtGt', '>>=': 'GtGtEq', '?': 'Question', '@': 'At', '[': 'LBracket', ']': 'RBracket', '^': 'Caret',
         '^=': 'CaretEq', '_': 'Underscore', '{': 'LBrace', '|': 'Pipe', '|=': 'PipeEq', '||': 'PipePipe', '}': 'RBrace',
         '>>>': 'GtGtGt', '>>>=': 'GtGtGtEq', '++': 'PlusPlus', '--': 'MinusMinus', '~': 'Tilde', '@interface': 'AtInterface',
         '\\{': 'BackslashLBrace', '"""': 'TripleQuote', 'non-sealed': 'NonSealed'}

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

out = ['// Written by gen_kinds.py from the token grammar; do not edit.', '',
       '/// Node kinds, as the tape names them: tree-sitter-java\'s, named and anonymous.',
       '#[derive(Clone, Copy, Debug, PartialEq, Eq)]', '#[repr(u16)]', 'pub enum K {']
out += [f'    {v},' for v, _, _ in rows]
out += ['}', '', '/// Each kind\'s name and whether it is named, in `K`\'s order.', 'pub const KINDS: &[(&str, bool)] = &[']
out += [f'    ({json.dumps(n)}, {str(b).lower()}),' for _, n, b in rows]
out += ['];', '', '/// The last kind.', f'pub const LAST: K = K::{rows[-1][0]};', '']
fields.sort()
out += ['/// Fields, as the tape names them; 0 is no field.', 'pub const FIELDS: &[&str] = &[', '    "",']
out += [f'    "{f}",' for f in fields]
out += ['];', '', '/// Field ids, in `FIELDS`\' order.', 'pub mod f {', '    pub const NONE: u8 = 0;']
out += [f'    pub const {f.upper()}: u8 = {i + 1};' for i, f in enumerate(fields)]
out += ['}', '']
index = {(n, b): i for i, (_, n, b) in enumerate(rows)}
dropped = set()
for line in open(os.path.join(here, 'implied.tsv')):
    d, kind, token, field = line.rstrip('\n').split('\t')
    parent = index[(kind, True)]
    tok = index.get((token, False))
    assert tok is not None, (kind, token)
    fid = 0 if field == '' else fields.index(field) + 1
    dropped.add(parent | tok << 9 | fid << 18)
keys = sorted(dropped)
out += ['/// The tokens a tape leaves out: `kind | token << 9 | field << 18`, sorted (`implied.tsv`, the',
        '/// engine\'s automata).', 'pub const DROPPED: &[u32] = &[']
for i in range(0, len(keys), 12):
    out.append('    ' + ' '.join(f'{k},' for k in keys[i:i + 12]))
out += ['];', '']
open(os.path.join(here, 'src', 'kinds.rs'), 'w').write('\n'.join(out))
print(len(rows), 'kinds,', len(fields), 'fields,', len(keys), 'dropped')
