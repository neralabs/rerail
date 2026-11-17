"""Writes src/kinds.rs: the node kinds of tree-sitter-cpp (the token grammar the engine reads,
`native/grammars/token-grammars/cpp`; tree-sitter-c's are among them), named and anonymous, as
the lean parser's `K` and `KINDS`, and the fields of both grammars.

    python3 native/crates/c_lean/gen_kinds.py

With them, `DROPPED_C` and `DROPPED_CPP`: the tokens a tape leaves out under each kind, from
`implied-c.tsv` and `implied-cpp.tsv`, which the engine's automata write (`cargo test …
ecma_implied_tokens --ignored` with `RERAIL_IMPLIED_DIALECTS=c` or `cpp` and
`RERAIL_ECMA_IMPLIED=…`): the parser stores a token by the grammar, as the engine's tape writer
does, not by hand.

A dev tool: the generated file is checked in, and the build does not run this."""
import json, os, re

here = os.path.dirname(os.path.abspath(__file__))
grammar = os.path.join(here, '..', '..', 'grammars', 'token-grammars', 'cpp', 'grammar.json')
c_grammar = os.path.join(here, '..', '..', 'grammars', 'token-grammars', 'c', 'grammar.json')

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
cg = json.load(open(c_grammar))
csup = set(cg.get('supertypes', []))
for k, v in cg['rules'].items():
    if not k.startswith('_') and k not in csup:
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
         '\\{': 'BackslashLBrace', '"""': 'TripleQuote', 'non-sealed': 'NonSealed', ':=': 'ColonEq', '<-': 'LtMinus', '&^': 'AmpCaret', '&^=': 'AmpCaretEq', '\n': 'Newline', '\u0000': 'Nul', '`': 'Backtick', '[[': 'LBracketLBracket', ']]': 'RBracketRBracket', '.*': 'DotStar', '->*': 'ArrowStar', '<=>': 'Spaceship', '()': 'Parens', '[]': 'Brackets', '""': 'EmptyString', 'L\'': 'LQuote', 'u\'': 'USmallQuote', 'U\'': 'UQuote', 'u8\'': 'U8Quote', 'L"': 'LDQuote', 'u"': 'USmallDQuote', 'U"': 'UDQuote', 'u8"': 'U8DQuote', 'R"': 'RDQuote', 'LR"': 'LRDQuote', 'uR"': 'USmallRDQuote', 'UR"': 'URDQuote', 'u8R"': 'U8RDQuote', '#include': 'HashInclude', '#define': 'HashDefine', '#if': 'HashIf', '#ifdef': 'HashIfdef', '#ifndef': 'HashIfndef', '#else': 'HashElse', '#elif': 'HashElif', '#elifdef': 'HashElifdef', '#elifndef': 'HashElifndef', '#endif': 'HashEndif'}

def camel(s):
    return ''.join(w[:1].upper() + w[1:] for w in re.split(r'[_ ]', s) if w)

rows = [(camel(n), n, True) for n in named]
seen = {r[0] for r in rows}
for a in anon:
    # A keyword keeps its spelling (`T__attribute__`, `Tstruct`): camel case would merge
    # `__attribute` and `__attribute__`.
    v = 'T' + (words.get(a) or (a if re.fullmatch(r'[A-Za-z_]\w*', a) else camel(a)))
    assert re.fullmatch(r'T\w+', v), a
    assert v not in seen, a
    seen.add(v)
    rows.append((v, a, False))

out = ['// Written by gen_kinds.py from the token grammar; do not edit.', '',
       '/// Node kinds, as the tape names them: tree-sitter-cpp\'s (with tree-sitter-c\'s), named and anonymous.',
       '#[derive(Clone, Copy, Debug, PartialEq, Eq)]', '#[allow(non_camel_case_types)]', '#[repr(u16)]', 'pub enum K {']
out += [f'    {v},' for v, _, _ in rows]
out += ['}', '', '/// Each kind\'s name and whether it is named, in `K`\'s order.', 'pub const KINDS: &[(&str, bool)] = &[']
out += [f'    ({json.dumps(n).replace(chr(92) + "u0000", chr(92) + "0")}, {str(b).lower()}),' for _, n, b in rows]
out += ['];', '', '/// The last kind.', f'pub const LAST: K = K::{rows[-1][0]};', '']
fields.sort()
out += ['/// Fields, as the tape names them; 0 is no field.', 'pub const FIELDS: &[&str] = &[', '    "",']
out += [f'    "{f}",' for f in fields]
out += ['];', '', '/// Field ids, in `FIELDS`\' order.', 'pub mod f {', '    pub const NONE: u8 = 0;']
out += [f'    pub const {f.upper()}: u8 = {i + 1};' for i, f in enumerate(fields)]
out += ['}', '']
index = {(n, b): i for i, (_, n, b) in enumerate(rows)}
counts = []
for dialect in ('c', 'cpp'):
    dropped = set()
    # A `\n` token is written as itself: its row spans two lines.
    lines = open(os.path.join(here, f'implied-{dialect}.tsv')).read().split('\n')
    rows_ = []
    while lines:
        line = lines.pop(0)
        if not line:
            continue
        if line.count('\t') == 2:
            line += '\n' + lines.pop(0)
        rows_.append(line)
    for line in rows_:
        d, kind, token, field = line.split('\t')
        parent = index[(kind, True)]
        tok = index.get((token, False))
        assert tok is not None, (kind, token)
        fid = 0 if field == '' else fields.index(field) + 1
        dropped.add(parent | tok << 9 | fid << 18)
    keys = sorted(dropped)
    counts.append(len(keys))
    out += [f'/// The tokens a {dialect.upper()} tape leaves out: `kind | token << 9 | field << 18`, sorted',
            f'/// (`implied-{dialect}.tsv`, the engine\'s automata).', f'pub const DROPPED_{dialect.upper()}: &[u32] = &[']
    for i in range(0, len(keys), 10):
        out.append('    ' + ' '.join(f'{k},' for k in keys[i:i + 10]))
    out += ['];', '']
open(os.path.join(here, 'src', 'kinds.rs'), 'w').write('\n'.join(out))
print(len(rows), 'kinds,', len(fields), 'fields,', counts, 'dropped')
