"""Writes a lean parser's src/kinds.rs: the node kinds of its tree-sitter grammar (the token grammar the
engine reads, `native/grammars/token-grammars/<lang>`), named and anonymous, as the parser's `K` and
`KINDS`; its fields; and the engine's token automata as tables, so the writer decides each gap's
tokens as the engine's `stored_children` does.

    python3 native/crates/kotlin_lean/gen_kinds.py kotlin
    python3 native/crates/kotlin_lean/gen_kinds.py swift
    python3 native/crates/kotlin_lean/gen_kinds.py scala

`<crate>/automata.tsv` is the engine's `TokenGrammar::automata_text` (`cargo test … token_automata_text
--ignored` with `RERAIL_AUTOMATA_DIALECT=<lang> RERAIL_AUTOMATA_OUT=native/crates/<lang>_lean/automata.tsv`).

A dev tool: the generated file is checked in, and the build does not run this."""
import json, os, re, sys

lang = sys.argv[1] if len(sys.argv) > 1 else 'kotlin'
here = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', f'{lang}_lean')
grammar = os.path.join(here, '..', '..', 'grammars', 'token-grammars', lang, 'grammar.json')

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
# The automata's own token names (externals named by a rule, aliases) are in the table too.
for line in open(os.path.join(here, 'automata.tsv')):
    r = line.rstrip('\n').split('\t')
    labels = []
    if r[0] == 'M':
        labels = [r[2]]
    elif r[0] in ('I', 'E'):
        labels = [x for x in r[-1].split('\x1f') if x]
    elif r[0] == 'P':
        labels = [r[1]]
    for l in labels:
        kind, name, field = l.split('\x1e')
        add(named if kind == 'n' else anon, name)
        if field:
            add(fields, field)
    if r[0] == 'K':
        add(named, r[1])

words = {'!': 'Bang', '!=': 'Ne', '"': 'DQuote', '#': 'Hash', '$': 'Dollar', '%': 'Percent', '%=': 'PercentEq', '&': 'Amp',
         '&&': 'AmpAmp', '&=': 'AmpEq', "'": 'Quote', '(': 'LParen', ')': 'RParen', '*': 'Star', '*=': 'StarEq', '*/': 'StarSlash',
         '+': 'Plus', '+=': 'PlusEq', ',': 'Comma', '-': 'Minus', '-=': 'MinusEq', '->': 'Arrow', '.': 'Dot', '..': 'DotDot',
         '...': 'DotDotDot', '..=': 'DotDotEq', '/': 'Slash', '/*': 'SlashStar', '//': 'SlashSlash', '/=': 'SlashEq', ':': 'Colon',
         '::': 'ColonColon', ';': 'Semi', '<': 'Lt', '<<': 'LtLt', '<<=': 'LtLtEq', '<=': 'Le', '=': 'Eq', '==': 'EqEq', '=>': 'FatArrow',
         '>': 'Gt', '>=': 'Ge', '>>': 'GtGt', '>>=': 'GtGtEq', '?': 'Question', '@': 'At', '[': 'LBracket', ']': 'RBracket', '^': 'Caret',
         '^=': 'CaretEq', '_': 'Underscore', '{': 'LBrace', '|': 'Pipe', '|=': 'PipeEq', '||': 'PipePipe', '}': 'RBrace',
         '>>>': 'GtGtGt', '>>>=': 'GtGtGtEq', '++': 'PlusPlus', '--': 'MinusMinus', '~': 'Tilde', '@interface': 'AtInterface',
         '\\{': 'BackslashLBrace', '"""': 'TripleQuote', '#!': 'Shebang', '?.': 'QuestionDot', '?:': 'Elvis',
         '!!': 'BangBang', '..<': 'DotDotLt', '===': 'EqEqEq', '!==': 'NeEq', 'as?': 'AsQuestion', 'as!': 'AsBang', '${': 'DollarLBrace',
         'return@': 'ReturnAt', 'continue@': 'ContinueAt', 'break@': 'BreakAt', 'this@': 'ThisAt', 'super@': 'SuperAt',
         '\\u': 'BackslashU', '`': 'Backtick', '\\': 'Backslash', '??': 'QQ', '...': 'DotDotDot', '<-': 'LArrow', '=>>': 'FatArrowGt',
         '?=>': 'QFatArrow', '<:': 'Subtype', '>:': 'Supertype', '<%': 'ViewBound', '#': 'Hash', '#if': 'HashIf', '#else': 'HashElse',
         '#elseif': 'HashElseif', '#endif': 'HashEndif', '#selector': 'HashSelector', '#keyPath': 'HashKeyPath', '#available': 'HashAvailable',
         '#unavailable': 'HashUnavailable', '#colorLiteral': 'HashColorLiteral', '#fileLiteral': 'HashFileLiteral',
         '#imageLiteral': 'HashImageLiteral', '#sourceLocation': 'HashSourceLocation', '#file': 'HashFile', '#fileID': 'HashFileID',
         '#filePath': 'HashFilePath', '#line': 'HashLine', '#column': 'HashColumn', '#function': 'HashFunction', '#dsohandle': 'HashDsohandle',
         '#warning': 'HashWarning', '#error': 'HashError', '\\(': 'BackslashLParen', '"': 'DQuote', '?:': 'Elvis', '...': 'DotDotDot',
         '..<': 'DotDotLt', '&&=': 'AmpAmpEq', '||=': 'PipePipeEq', '<<<': 'LtLtLt', '>>>': 'GtGtGt', '**': 'StarStar',
         '&+': 'AmpPlus', '&-': 'AmpMinus', '&*': 'AmpStar', '&<<': 'AmpLtLt', '&>>': 'AmpGtGt', '&<<=': 'AmpLtLtEq', '&>>=': 'AmpGtGtEq',
         '%=': 'PercentEq', '^=': 'CaretEq', '~=': 'TildeEq', '-->': 'LongArrow', '#"': 'HashQuote', '"#': 'QuoteHash', '?!': 'QBang',
         '>>=': 'GtGtEq', '<<=': 'LtLtEq', '<<': 'LtLt', '>>': 'GtGt', '\'': 'Quote', '\\\\': 'BackslashBackslash', '$"': 'DollarQuote',
         '"""': 'TripleQuote', '@@': 'AtAt', '?->': 'QArrow', '!=': 'Ne', '|': 'Pipe', '~>': 'TildeGt', '#>': 'HashGt', '<#': 'LtHash'}

def camel(s):
    return ''.join(w[:1].upper() + w[1:] for w in re.split(r'[_ ]', s) if w)

def ident(s):
    """A token's variant: its name in `words`, else its letters camel-cased, else its bytes."""
    if s in words:
        return 'T' + words[s]
    c = camel(s)
    if re.fullmatch(r'[A-Za-z0-9]+', c):
        return 'T' + c
    return 'T' + ''.join(f'X{ord(ch):02x}' if not ch.isalnum() else ch for ch in s)

rows = [(camel(n), n, True) for n in named]
seen = {r[0] for r in rows}
for a in anon:
    v = ident(a)
    while v in seen:
        v += '_'
    assert re.fullmatch(r'T[A-Za-z0-9_]+', v), a
    seen.add(v)
    rows.append((v, a, False))
assert len(rows) < 1024, len(rows)

out = ['// Generated by native/crates/kotlin_lean/gen_kinds.py; do not edit. Written from the token grammar.', '#![allow(dead_code)]', '',
       f'/// Node kinds, as the tape names them: tree-sitter-{lang}\'s, named and anonymous.',
       '#[derive(Clone, Copy, Debug, PartialEq, Eq)]', '#[repr(u16)]', '#[allow(non_camel_case_types)]', 'pub enum K {']
out += [f'    {v},' for v, _, _ in rows]
out += ['}', '', '/// Each kind\'s name and whether it is named, in `K`\'s order.', 'pub const KINDS: &[(&str, bool)] = &[']
out += [f'    ({json.dumps(n, ensure_ascii=False)}, {str(b).lower()}),' for _, n, b in rows]
out += ['];', '', '/// The last kind.', f'pub const LAST: K = K::{rows[-1][0]};', '']
fields.sort()
assert len(fields) < 64
out += ['/// Fields, as the tape names them; 0 is no field.', 'pub const FIELDS: &[&str] = &[', '    "",']
out += [f'    "{f}",' for f in fields]
out += ['];', '', '/// Field ids, in `FIELDS`\' order.', 'pub mod f {', '    pub const NONE: u8 = 0;']
out += [f'    pub const {f.upper()}: u8 = {i + 1};' for i, f in enumerate(fields)]
out += ['}', '']
index = {(n, b): i for i, (_, n, b) in enumerate(rows)}

# The token automata of the kinds whose tapes leave tokens out, so the writer decides each gap as the
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
# An implied word, packed: where it starts in `WORD` << 12 | its length << 8 | which of its tokens the
# tape leaves out (bit i: token i; a token the automaton keeps wherever it stands is stored). Never 0:
# a word kept here leaves a token out.
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
# Tokens tree-sitter names at run time though the automata take them as tokens (Scala's `(`, whose
# symbol the alias `arguments` shares). The engine's walk refuses such a child, and so stores every
# child of its parent: an automaton that takes no other token before one is not used (a parent that
# takes tokens before it, Scala's `for_expression`, is the parser's to write).
RUNTIME_NAMED = {'scala': ['(']}.get(lang, [])
for a, (kind, states, imp, end, kept) in enumerate(autos):
    rn = {index[(n, False)] for n in RUNTIME_NAMED}
    seen, todo = {0}, [0]
    while todo:
        st = todo.pop()
        for l, to in states[st][1]:
            if (l & 1023) not in rn and to not in seen:
                seen.add(to)
                todo.append(to)
    # Before such a token it takes none: with one, all its children are stored; without, it has
    # no tokens to leave out.
    takes_token = any(not rows[l & 1023][2] and (l & 1023) not in rn for st in seen for l, _ in states[st][1])
    if any((l & 1023) in rn for _, ms in states for l, _ in ms) and not takes_token:
        auto_of[kind] = 0
def table(name, ty, vals, per=16, doc=''):
    o = [doc, f'pub static {name}: &[{ty}] = &[']
    for i in range(0, len(vals), per):
        o.append('    ' + ' '.join(f'{v},' for v in vals[i:i + per]))
    return o + ['];', '']
# Each token's fate under each automaton, where the kind alone decides it: over every gap a token can
# stand in (every path of tokens from a gap's first state to a named child or the end), left out in all
# of them, or stored in all. A token whose gaps differ, or that stands on a cycle of tokens, or that the
# automaton never takes, is decided by the walk. `FATE[a]`: the dropped bits by kind, then the walk bits.
W = (len(rows) + 63) // 64
KINDS_NAMED = [r[2] for r in rows]
fates, dynamic = [], []
for a, (kind, states, imp, end, kept) in enumerate(autos):
    named = lambda l: KINDS_NAMED[l & 1023]
    fate, walkset = {}, set()
    gap_starts = {0} | {to for _, ms in states for l, to in ms if named(l)}
    for s0 in sorted(gap_starts):
        stack = [(s0, [], frozenset([s0]))]
        while stack:
            q, path, seen_ = stack.pop()
            acc, ms = states[q]
            ends = [None] if acc else []
            for l, to in ms:
                if named(l):
                    ends.append(l)
                elif to in seen_ or len(path) >= 8:
                    walkset.update(t & 1023 for t in path + [l])
                else:
                    stack.append((to, path + [l], seen_ | {to}))
            for nx in ends:
                if not path:
                    continue
                w = end.get(s0) if nx is None else imp.get((s0, nx))
                implied = w is not None and w == path
                for t in path:
                    fate.setdefault(t & 1023, set()).add(implied and t not in kept)
    # Dynamic: some token the automaton takes is decided by the walk, so a node of it holds all its
    # tokens until it closes (the walk reads every child).
    dynamic.append(any(len(v) > 1 or k in walkset for k, v in fate.items()) or bool(walkset))
    bits = [0] * (2 * W)
    for k in range(len(rows)):
        if KINDS_NAMED[k]:
            continue
        f = fate.get(k)
        if f is None or len(f) > 1 or k in walkset:
            bits[W + k // 64] |= 1 << (k % 64)
        elif True in f:
            bits[k // 64] |= 1 << (k % 64)
    fates.append(bits)
out += [f'/// Per automaton: each token kind\'s fate where the kind decides it, a bit each: `[..{W}]` left',
        f'/// out, `[{W}..]` decided by the walk (neither: stored).', f'pub static FATE: &[[u64; {2 * W}]] = &[']
for bits in fates:
    out.append('    [' + ', '.join(f'{x:#x}' for x in bits) + '],')
out += ['];', '', f'/// The words of a kind bitset in `FATE`.', f'pub const FATE_WORDS: usize = {W};', '']
out += table('DYNAMIC', 'bool', ['true' if d else 'false' for d in dynamic], 16, '/// Per automaton: whether the walk decides some token it takes (a node of it holds its tokens to its close).')
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
print(len(autos), 'automata,', len(accept), 'states,', len(implied_rows), 'implied,', sum(1 for e in end_of if e), 'end,', len(rows), 'kinds,', len(fields), 'fields,', sum(dynamic), 'dynamic')
