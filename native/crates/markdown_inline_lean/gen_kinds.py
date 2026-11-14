"""Writes src/kinds.rs: the node kinds of tree-sitter-md's inline grammar (
`native/grammars/tree-sitter-markdown/markdown-inline`), named and anonymous, as the lean parser's `K` and `KINDS`.

    python3 native/crates/markdown_inline_lean/gen_kinds.py

With them, the engine's token automata (`automata.tsv`), so the writer stores a node's tokens as
the engine's tape writer does, not by hand; and the entity names.

A dev tool: the generated file is checked in, and the build does not run this."""
import json, os, re

here = os.path.dirname(os.path.abspath(__file__))
grammar = os.path.join(here, '..', '..', 'grammars', 'tree-sitter-markdown', 'markdown-inline', 'src', 'grammar.json')

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
         '\\{': 'BackslashLBrace', '"""': 'TripleQuote', '??': 'QQ', '??=': 'QQEq', '#if': 'HashIf', '#else': 'HashElse', '#elif': 'HashElif', '#endif': 'HashEndif', '#region': 'HashRegion', '#endregion': 'HashEndregion', '#line': 'HashLine', '#pragma': 'HashPragma', '#nullable': 'HashNullable', '#error': 'HashError', '#warning': 'HashWarning', '#define': 'HashDefine', '#undef': 'HashUndef', '__makeref': 'Makeref', '__reftype': 'Reftype', '__refvalue': 'Refvalue', '#!': 'Shebang', '@"': 'AtQuote', 'Cdecl': 'CdeclCc', 'Stdcall': 'StdcallCc', 'Thiscall': 'ThiscallCc', 'Fastcall': 'FastcallCc', '?->': 'QArrow', '<=>': 'Spaceship', '===': 'EqEqEq', '!==': 'NeEq', '<>': 'LtGt', '**': 'StarStar', '**=': 'StarStarEq', '.=': 'DotEq', '|>': 'PipeGt', '#[': 'HashLBracket', '<<<': 'LtLtLt', '\\': 'Backslash', '`': 'Backtick', '?>': 'QGt', '\\u': 'BackslashU', 'yield from': 'YieldFrom', '<!': 'LtBang', '</': 'LtSlash', '/>': 'SlashGt', '-->': 'CommentEnd', ']]>': 'CdataEnd', '<!--': 'CommentStart', '<?': 'LtQuestion', '<![CDATA[': 'CdataStart', '?>': 'QGt'}

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
       '/// Node kinds, as the tape names them: tree-sitter-md\'s, named and anonymous.',
       '#[derive(Clone, Copy, Debug, PartialEq, Eq)]', '#[repr(u16)]', 'pub enum K {']
out += [f'    {v},' for v, _, _ in rows]
out += ['}', '', '/// Each kind\'s name and whether it is named, in `K`\'s order.', 'pub const KINDS: &[(&str, bool)] = &[']
out += [f'    ({json.dumps(n)}, {str(b).lower()}),' for _, n, b in rows]
out += ['];', '', '/// The last kind.', '#[allow(dead_code)] // the tests\' check of the table', f'pub const LAST: K = K::{rows[-1][0]};', '']
fields.sort()
out += ['/// Fields, as the tape names them; 0 is no field.', 'pub const FIELDS: &[&str] = &[', '    "",']
out += [f'    "{f}",' for f in fields]
out += ['];', '', '/// Field ids, in `FIELDS`\' order.', 'pub mod f {', '    #[allow(dead_code)] // no field', '    pub const NONE: u8 = 0;']
out += [f'    pub const {f.upper()}: u8 = {i + 1};' for i, f in enumerate(fields)]
out += ['}', '']
index = {(n, b): i for i, (_, n, b) in enumerate(rows)}
# The names `entity_reference` takes (`&name;`), sorted for a binary search.
pattern = g['rules']['entity_reference']['value']
assert pattern.startswith('&(') and pattern.endswith(');'), pattern[:40]
names = sorted(set(pattern[2:-2].split('|')))
assert all(re.fullmatch(r'[A-Za-z0-9]+', n) for n in names)
out += ['/// The names an `entity_reference` takes (`&name;`, the grammar\'s pattern), sorted.', 'pub static ENTITIES: &[&str] = &[']
for i in range(0, len(names), 12):
    out.append('    ' + ' '.join(f'"{n}",' for n in names[i:i + 12]))
out += ['];', '']
# The token automata of the kinds whose tapes leave tokens out (`automata.tsv`, the engine's
# `TokenGrammar::automata_text`: `cargo test … token_automata_text --ignored` with
# `RERAIL_AUTOMATA_OUT=native/crates/markdown_inline_lean/automata.tsv`), so the writer decides each gap as the
# engine's `stored_children` does. A label is `kind | field << 10`. Only the implied words that hold a
# token the tape can leave out are kept: a gap matching any other leaves its tokens stored.
def lab(t):
    kind, name, field = t.split('\x1e')
    k = index.get((name, kind == 'n'))
    assert k is not None, (kind, name)
    return k | (0 if field == '' else fields.index(field) + 1) << 10
autos = []  # per automaton: kind, states [(accepting, [(label, to)])], implied {(state, label): word}, end {state: word}, kept
for line in open(os.path.join(here, 'automata.tsv')):
    r = line.rstrip('\n').split('\t')
    if r[0] == 'K':
        autos.append((index[(r[1], True)], [], {}, {}, set()))
    elif r[0] == 'S':
        assert int(r[1]) == len(autos[-1][1])
        autos[-1][1].append((r[2] == '1', []))
    elif r[0] == 'M':
        autos[-1][1][int(r[1])][1].append((lab(r[2]), int(r[3])))
    elif r[0] == 'I':
        autos[-1][2][(int(r[1]), lab(r[2]))] = [lab(x) for x in r[3].split('\x1f')] if r[3] else []
    elif r[0] == 'E':
        autos[-1][3][int(r[1])] = [lab(x) for x in r[2].split('\x1f')] if r[2] else []
    elif r[0] == 'P':
        autos[-1][4].add(lab(r[1]))
auto_of = [0] * len(rows)
starts, moves_at, moves, accept, implied_at, implied_rows, end_of, word_tbl = [], [0], [], [], [0], [], [], []
# An implied word, packed: where it starts in `WORD` << 12 | its length << 8 | which of its tokens
# the tape leaves out (bit i: token i; a token the automaton keeps wherever it stands is stored).
# Never 0: a word kept here leaves a token out. (A wiki link's gap holds four: `[ [` and `] ]`.)
def packed(w, kept):
    assert 1 <= len(w) <= 8, w
    at = len(word_tbl)
    word_tbl.extend(w)
    return at << 12 | len(w) << 8 | sum(1 << i for i, l in enumerate(w) if l not in kept)
for a, (kind, states, imp, end, kept) in enumerate(autos):
    auto_of[kind] = a + 1
    base = len(accept)
    starts.append(base)
    drops = lambda w: any(l not in kept for l in w)
    for st, (acc, ms) in enumerate(states):
        accept.append(acc)
        for l, to in sorted(ms):
            moves.append((l, base + to))
        moves_at.append(len(moves))
        for l in sorted(l for (s2, l), w in imp.items() if s2 == st and drops(imp[(s2, l)])):
            implied_rows.append((l, packed(imp[(st, l)], kept)))
        implied_at.append(len(implied_rows))
        w = end.get(st)
        end_of.append(packed(w, kept) if w is not None and drops(w) else 0)
def table(name, ty, vals, per=16, doc=''):
    o = [doc, f'pub static {name}: &[{ty}] = &[']
    for i in range(0, len(vals), per):
        o.append('    ' + ' '.join(f'{v},' for v in vals[i:i + per]))
    return o + ['];', '']
out += table('AUTOMATON', 'u16', auto_of, 32, '/// Per kind: its automaton, plus one (0: none; its tokens are all stored).')
out += table('START', 'u32', starts, 16, '/// Per automaton: its first state.')
out += table('MOVES_AT', 'u32', moves_at, 16, '/// Per state: where its moves start in `MOVES`, and (the next one\'s) where they end.')
out += table('MOVES', '(u32, u32)', [f'({l}, {t})' for l, t in moves], 8, '/// Moves, by state, sorted by label: (label, the state moved to).')
out += table('ACCEPTS', 'u64', [sum(1 << b for b in range(64) if i + b < len(accept) and accept[i + b]) for i in range(0, len(accept), 64)], 8, '/// Per state: whether it accepts, a bit each.')
out += table('IMPLIED_AT', 'u32', implied_at, 16, '/// Per state: where its implied words start in `IMPLIED`, and (the next one\'s) where they end.')
out += table('IMPLIED', '(u32, u32)', [f'({l}, {w})' for l, w in implied_rows], 8, '/// The implied words from a gap\'s first state to a named child, by state, sorted by the child\'s label: (label, the word packed: `WORD` start << 12 | length << 8 | the tokens left out).')
out += table('IMPLIED_END', 'u32', end_of, 16, '/// Per state: the implied word from it to the end, packed as in `IMPLIED` (0: none that leaves a token out).')
out += table('WORD', 'u32', word_tbl, 16, '/// The implied words\' labels.')
open(os.path.join(here, 'src', 'kinds.rs'), 'w').write('\n'.join(out))
print(len(rows), 'kinds,', len(fields), 'fields,', len(autos), 'automata,', len(accept), 'states')
