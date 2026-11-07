"""Compile the pinned scanners' regex constants to the engine's DFA dialect, offline."""
import importlib
import json
import re
import sys
import unicodedata
from functools import lru_cache


def character_class(predicate):
    ranges = []
    start = previous = None
    for point in range(0x110000):
        if predicate(chr(point)):
            if start is None:
                start = point
            previous = point
        elif start is not None:
            ranges.append((start, previous))
            start = None
    if start is not None:
        ranges.append((start, previous))
    escape = lambda value: rf'\x{{{value:x}}}'
    return '[' + ''.join(escape(a) if a == b else escape(a) + '-' + escape(b) for a, b in ranges) + ']'


@lru_cache
def classes():
    if sys.version_info[:2] != (3, 13) or unicodedata.unidata_version != '15.1.0':
        raise ValueError('Generate with pinned CPython 3.13 / Unicode 15.1')
    word = character_class(lambda c: c == '_' or c.isalnum())
    space = character_class(str.isspace)
    # CPython IGNORECASE preimages, including dotted/dotless I, long s and Kelvin sign.
    ascii_case = re.compile('[a-z]', re.I)
    letters = [chr(i) for i in range(0x110000) if ascii_case.fullmatch(chr(i))]
    folds = {letter: '[' + ''.join(c for c in letters if re.fullmatch(letter, c, re.I)) + ']'
             for letter in 'abcdefghijklmnopqrstuvwxyz'}
    return word, space, folds


def search(pattern):
    """Boolean search only. Refuse regex constructs these scanners do not use."""
    word, space, folds = classes()
    if pattern.flags & ~(re.U | re.I):
        raise ValueError(f'Unsupported source regex flags: {pattern.flags}')
    text, result, in_class, index = pattern.pattern, [], False, 0
    boundaries = [m.start() for m in re.finditer(r'(?<!\\)\\b', text)]
    if boundaries and not (text.startswith(r'\b') or text.endswith(r'\b')):
        raise ValueError(f'Unsupported internal word boundary: {text}')
    while index < len(text):
        if text.startswith(r'.*\b', index):
            if not text[:index].endswith(r'\s+'):
                raise ValueError(f'Unsupported boundary after a wildcard: {text}')
            # The preceding whitespace already establishes a boundary. Otherwise the
            # wildcard must end in a non-word character. Consuming another delimiter
            # unconditionally would reject `from inspect import getsource`.
            result.append('(?:.*[^' + word[1:] + ')?')
            index += 4
            continue
        char = text[index]
        if char == '\\':
            index += 1
            escaped = text[index]
            if escaped == 's':
                if in_class:
                    raise ValueError('Source scanner changed: space escape inside a class')
                result.append(space)
            elif escaped == 'w':
                if in_class:
                    raise ValueError('Source scanner changed: word escape inside a class')
                result.append(word)
            elif escaped == 'b':
                nonword = '[^' + word[1:]
                result.append(r'(?:\A|' + nonword + ')' if index == 1 or index != len(text) - 1 else r'(?:\z|' + nonword + ')')
            elif escaped.isalpha() and escaped != 'n':
                raise ValueError(f'Unsupported escape: {escaped}')
            else:
                result.append('\\' + escaped)
        else:
            if char == '[':
                in_class = True
            elif char == ']':
                in_class = False
            if char == '$' and not in_class:
                result.append(r'(?:\z|\n\z)')
            elif char.isascii() and char.isalpha() and pattern.flags & re.I:
                if in_class:
                    raise ValueError('Source scanner changed: case-folded letters inside a class')
                result.append(folds[char.lower()])
            else:
                result.append(char)
        index += 1
    return '(?s:.*)(?:' + ''.join(result) + ')(?s:.*)'


def module(source, check):
    # This import is offline, in the pinned source environment. Runtime decisions use only .rr.
    sys.path.insert(0, str(source))
    return importlib.import_module('scripts.lint_' + check)


def no_source_inspection(source, target, header):
    original = module(source, 'no_source_inspection')
    _, space, _ = classes()
    quote = json.dumps
    text = header('no_source_inspection', target).replace('#include "python/ast.rr"\n', '')
    text += '.decl inspection_skip(x: symbol)\n'
    text += ''.join(f'inspection_skip({quote(x)}).\n' for x in original.EXCLUDED_PATHS)
    text += '.decl inspection_line(d: number, l: number, t: symbol)\n'
    text += f'inspection_line(d, l, t) :- split_line(d, l, t), !match({quote("(?s)" + space + "*#.*")}, t), !inspection_excluded(d), !option("entryPoint", "pattern_search").\n'
    text += '.decl inspection_excluded(d: number)\ninspection_excluded(d) :- path_part(d, x), inspection_skip(x).\n'
    trim = r'\A' + space + '+|' + space + r'+\z'
    for index, (pattern, label) in enumerate(original.FORBIDDEN_PATTERNS):
        text += f'\n.violation Inspection{index}(at: span, ordinal: number, label: symbol, snippet: symbol) code "NSI001"\n    "{{label}} is forbidden in tests"\n'
        text += f'Inspection{index}(@place(p, l), {index}, {quote(label)}, @replace_all(t, {quote(trim)}, "")) :- inspection_line(d, l, t), doc(d, p, _), match({quote(search(pattern))}, t).\n'
        text += f'\n.violation PatternProbe{index}(at: span, ordinal: number) code "NSI_PATTERN"\n    {quote(label)}\n'
        text += f'PatternProbe{index}(@place("input.py", 1), {index}) :- option("entryPoint", "pattern_search"), option("patternOrdinal", "{index}"), option("patternText", t), match({quote(search(pattern))}, t).\n'
    return text


def serverplan_name_inference(source, target, header):
    original = module(source, 'serverplan_name_inference')
    _, space, _ = classes()
    quote = json.dumps
    text = header('serverplan_name_inference', target).replace('#include "python/ast.rr"\n', '')
    text += '.decl name_scan(d: number, l: number, t: symbol)\n'
    text += f'name_scan(d, l, t) :- split_line(d, l, t), !match({quote("(?s)" + space + "*#.*")}, t).\n'
    patterns = [('_COLUMN_SHAPE_RE', 'column shape selects semantic behavior'),
                ('_REFERENCE_PARSE_RE', 'reference text selects semantic behavior'),
                ('_REFERENCE_SYNTHESIS_RE', 'semantic reference is synthesized from text'),
                ('_COLUMN_TYPE_RE', 'SQL type is inferred from column spelling')]
    for index, (name, message) in enumerate(patterns, 1):
        body = f'name_scan(d, l, t), doc(d, p, _), match({quote(search(getattr(original, name)))}, t)'
        if index == 1:
            body += f', match({quote(search(original._CONTROL_LITERAL_RE))}, t)'
        if index == 4:
            body += ', file_name(d, "sql_column_type_hints.py")'
        text += f'\n.violation NameInference{index}(at: span) code "SPNI00{index}"\n    {quote(message)}\nNameInference{index}(@place(p, l)) :- {body}.\n'
    return text


def hasattr_async(source, target, header):
    original = module(source, 'hasattr_async')
    _, space, _ = classes()
    quote = json.dumps
    text = header('hasattr_async', target).replace('#include "python/ast.rr"\n', '')
    text += '.decl async_allowed(d: number, l: number)\n'
    for pattern in original.ALLOWED_PATTERNS:
        text += f'async_allowed(d, l) :- split_line(d, l, t), match({quote(search(re.compile(pattern)))}, t).\n'
    text += '.decl async_allowed_probe(t: symbol)\n'
    for pattern in original.ALLOWED_PATTERNS:
        text += f'async_allowed_probe(t) :- option("line", t), match({quote(search(re.compile(pattern)))}, t).\n'
    text += '\n.violation AllowedProbe(at: span) code "ASYNC_ALLOWED" "allowed capability check"\n'
    text += 'AllowedProbe(@place("input.py", 1)) :- option("entryPoint", "is_allowed_line"), async_allowed_probe(_).\n'
    text += '\n.violation PathProbe(at: span) code "ASYNC_EXCLUDED" "excluded path"\n'
    for pattern in original.EXCLUDE_PATTERNS:
        text += f'PathProbe(@place("input.py", 1)) :- option("entryPoint", "should_exclude"), option("path", t), match({quote(search(re.compile(pattern)))}, t).\n'
    text += '.decl async_line(d: number, l: number, t: symbol)\n'
    text += f'async_line(d, l, t) :- split_line(d, l, t), !match({quote("(?s)" + space + "*#.*")}, t), !async_allowed(d, l).\n'
    for kind, pattern in [('hasattr', original.HASATTR_ASYNC_PATTERN), ('getattr', original.GETATTR_ASYNC_PATTERN)]:
        # Both pinned expressions have one unnamed capture: the method spelling.
        if pattern.groups != 1 or pattern.groupindex or pattern.pattern.count(r'(\w+_async)') != 1:
            raise ValueError('hasattr scanner capture contract changed')
        named = re.compile(pattern.pattern.replace(r'(\w+_async)', r'(?P<method>\w+_async)'), pattern.flags)
        regex = search(named).replace('(?s:.*)', '(?s:.*?)', 1)
        text += f'\n.violation Async{kind}(at: span, method: symbol) code {quote(kind)}\n'
        text += f'    "{kind}(..., \'{{method}}\'...) checks for async method existence instead of calling the interface method directly"\n'
        text += f'Async{kind}(@place(p, l), method) :- async_line(d, l, t), doc(d, p, _), match({quote(regex)}, t).\n'
        text += f'\n.violation Probe{kind}(at: span, method: symbol) code "ASYNC_PATTERN" "{{method}}"\n'
        text += f'Probe{kind}(@place("input.py", 1), method) :- option("entryPoint", "pattern_search"), option("patternOrdinal", {quote(kind)}), option("patternText", t), match({quote(regex)}, t).\n'
    return text


def no_blanket_grammar_bypass(source, target, header):
    original = module(source, 'no_blanket_grammar_bypass')
    _, space, _ = classes()
    quote = json.dumps
    text = header('no_blanket_grammar_bypass', target)
    text += '.decl bypass_allowed(p: symbol)\n'
    text += ''.join(f'bypass_allowed({quote(p)}).\n' for p in sorted(original.ALLOWED_FILES))
    text += '.decl bypass_public_name(n: symbol)\n'
    text += ''.join(f'bypass_public_name({quote(p)}).\n' for p in sorted(original.FORBIDDEN_PUBLIC_METHODS))
    text += '.decl bypass_sql_target(p: symbol)\n'
    text += 'bypass_sql_target(p) :- option("sqlConnectionFile", p).\n'
    text += f'bypass_sql_target({quote(original.SQL_CONNECTION_FILE)}) :- !option("sqlConnectionFile", _).\n'
    text += '.decl bypass_scan(d: number, p: symbol, l: number, t: symbol)\n'
    text += f'bypass_scan(d, p, l, t) :- doc(d, p, _), split_line(d, l, t), !bypass_allowed(p), !match({quote("(?s)" + space + "*#.*")}, t).\n'
    text += '''
.violation BypassConstant(at: span, ordinal: number) code "NGB001" "_ALLOW_DIRECT_EXECUTION reference"
BypassConstant(@place(p, l), 0) :- bypass_scan(_, p, l, t), contains("_ALLOW_DIRECT_EXECUTION", t),
    !option("entryPoint", "check_no_public_grammar_methods").
.violation BypassArgument(at: span, ordinal: number) code "NGB001" "allow_direct_execution reference"
BypassArgument(@place(p, l), 1) :- bypass_scan(_, p, l, t),
    contains("allow_direct_execution", @replace_all(t, "#.*\\\\z", "")),
    !option("entryPoint", "check_no_public_grammar_methods").
.decl bypass_method_trigger(d: number)
bypass_method_trigger(d) :- split_line(d, _, t),
    (contains("execute_grammar", t) ; contains("_ALLOW_DIRECT_EXECUTION", t) ; contains("allow_direct_execution", t)).
.violation BypassPublic(at: node, name: symbol, ordinal: number) code "NGB001" "public method {name}() still exists"
'''
    text += 'BypassPublic(f, n, 2) :- doc(d, p, _), bypass_sql_target(p), bypass_method_trigger(d),\n'
    text += '    docof(f, d), kind(f, "function_definition"), def_name(f, n), bypass_public_name(n), !syntax_error(d, _, _),\n'
    text += '    !option("entryPoint", "check_no_bypass_references").\n'
    text += '\n.violation BypassMissing(at: span) code "NGB001" "file not found"\n'
    text += 'BypassMissing(@place(p, 0)) :- option("entryPoint", "check_no_public_grammar_methods"), bypass_sql_target(p), !file(p).\n'
    return text


def admin_async_purity(source, target, header):
    original = module(source, 'admin_async_purity')
    _, space, _ = classes()
    quote = json.dumps
    text = header('admin_async_purity', target).replace('#include "python/ast.rr"\n', '')
    text += '.decl admin_skip_path(p: symbol)\n'
    for index, pattern in enumerate(original.SKIP_PATTERNS):
        text += f'admin_skip_path(p) :- doc(_, p, _), match({quote(search(pattern))}, p).\n'
        text += f'\n.violation AdminSkipPattern{index}(at: span) code "ADMIN_SKIP_PATTERN" "matched"\n'
        text += f'AdminSkipPattern{index}(@place("input.py", 1)) :- option("entryPoint", "skip_pattern_search"), option("patternOrdinal", "{index}"), option("patternText", t), match({quote(search(pattern))}, t).\n'
    text += '.decl admin_allowlisted(p: symbol)\nadmin_allowlisted(p) :- option_item("allowlistedFiles", _, p).\n'
    text += '.decl admin_candidate(d: number, l: number, t: symbol, ordinal: number, label: symbol)\n'
    for index, (pattern, label) in enumerate(original.FORBIDDEN_PATTERNS):
        text += f'admin_candidate(d, l, t, {index}, {quote(label)}) :- split_line(d, l, t), doc(d, p, _), !admin_skip_path(p), !admin_allowlisted(p), !match({quote("(?s)" + space + "*#.*")}, t), match({quote(search(pattern))}, t).\n'
        text += f'\n.violation AdminPattern{index}(at: span) code "ADMIN_PATTERN" "matched"\n'
        text += f'AdminPattern{index}(@place("input.py", 1)) :- option("entryPoint", "pattern_search"), option("patternOrdinal", "{index}"), option("patternText", t), match({quote(search(pattern))}, t).\n'
    triples = quote('"""|\x27\x27\x27')
    text += '.decl admin_quotes(d: number, l: number, n: number)\n'
    text += f'admin_quotes(d, l, n) :- split_line(d, l, t), n = (strlen(t) - strlen(@replace_all(t, {triples}, ""))) / 3, n > 0.\n'
    text += '.decl admin_toggle(d: number, l: number)\nadmin_toggle(d, l) :- admin_quotes(d, l, n), n % 2 = 1.\n'
    # Only candidate lines read preceding toggles. This preserves the source's textual
    # state machine, including quote markers in comments and mixed quote delimiters.
    text += '.decl admin_visible(d: number, l: number, t: symbol, ordinal: number, label: symbol)\n'
    text += 'admin_visible(d, l, t, ordinal, label) :- admin_candidate(d, l, t, ordinal, label), !admin_quotes(d, l, _), parity = count : { admin_toggle(d, before), before < l }, parity % 2 = 0.\n'
    trim = space + r'+\z'
    text += '\n.violation AdminPurity(at: span, ordinal: number, pattern: symbol, text: symbol) code "AAP001"\n    "{pattern}: {text}"\n'
    text += f'AdminPurity(@place(p, l), ordinal, label, @replace_all(t, {quote(trim)}, "")) :- admin_visible(d, l, t, ordinal, label), doc(d, p, _).\n'
    for entry, code, relation in [('_should_skip', 'ADMIN_SKIP', 'admin_skip_probe'), ('_is_allowlisted', 'ADMIN_ALLOWLIST', 'admin_allowlisted')]:
        text += f'\n.violation {code}(at: span) code "{code}" "matched"\n{code}(@place("input.py", 1)) :- option("entryPoint", {quote(entry)}), option("path", p), {relation}(p).\n'
    text += '.decl admin_skip_probe(p: symbol)\n'
    for pattern in original.SKIP_PATTERNS:
        text += f'admin_skip_probe(p) :- option("path", p), match({quote(search(pattern))}, p).\n'
    return text
