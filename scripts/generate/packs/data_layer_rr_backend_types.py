"""Port the raw backend-comparison scanner with exact Python regex word semantics."""
import json
import re
from data_layer_rr_text import module, classes, character_class

MESSAGES = {
    'BT000': 'Possible backend type string comparison - consider using BackendType enum',
    'BT001': 'Use BackendType enum with .is_mysql_family or .is_sql_backend instead of dialect.name in (...)',
    'BT002': "Use BackendType enum instead of dialect.name == 'string'",
    'BT003': 'Use BackendType enum instead of string comparison',
    'BT004': 'Use BackendType.from_connection_class() instead of substring matching',
    'BT005': 'Use BackendType.from_connection_class() instead of class name inspection',
}


def backend_type_strings(source, target, header):
    original = module(source, 'backend_type_strings')
    word, space, _ = classes()
    digits = character_class(str.isdecimal)
    q = json.dumps
    trim = r'\A' + space + '+|' + space + r'+\z'
    text = header('backend_type_strings', target).replace('#include "python/ast.rr"\n', '')
    text = re.sub(r'^\.scope include .*$', '.scope include "impl/**/*.py", "data_layer/**/*.py", "_internal/**/*.py", "l*/**/*.py"', text, flags=re.M)
    text = re.sub(r'^\.scope exclude .*$', '.scope exclude "**/tests/**", "**/test/**", "**/.git/**", "**/.venv/**", "**/venv/**", "**/__pycache__/**", "**/tmp/**"', text, flags=re.M)
    text += '.decl bts_config(key: symbol, ordinal: number, value: symbol)\n'
    for key, values in [('BACKEND_STRINGS', sorted(original.BACKEND_STRINGS)),
                        ('SKIP_PATTERNS', original.SKIP_PATTERNS),
                        ('COMPARISON_PATTERNS', [p.pattern for p in original.COMPARISON_PATTERNS])]:
        for i, value in enumerate(values):
            text += f'bts_config({q(key)}, {i}, {q(value)}).\n'
    text += '.decl bts_skip(path: symbol)\nbts_skip(path) :- bts_path(path), bts_config("SKIP_PATTERNS", _, part), contains(part, path).\n'
    text += '.decl bts_path(path: symbol)\nbts_path(path) :- option("path", path).\nbts_path(path) :- option("sourcePath", path).\nbts_path(path) :- doc(_, path, _).\n'
    text += '.decl bts_trigger(value: symbol)\n'
    for trigger in original.BACKEND_TYPE_STRINGS_EXTRACTOR.source_triggers:
        text += f'bts_trigger({q(trigger)}).\n'
    text += r'''
.decl bts_api_skip()
bts_api_skip() :- option("skipOverride", "true").
bts_api_skip() :- !option("skipOverride", _), option("sourcePath", path), bts_skip(path).
.decl bts_framework_skip(d: number)
bts_framework_skip(d) :- path_part(d, p),
    (p = "tests" ; p = "test" ; p = ".git" ; p = ".venv" ; p = "venv" ; p = "__pycache__" ; p = "tmp").
.decl bts_doc(d: number)
bts_doc(d) :- doc(d, _, _), option("entryPoint", entry),
    (entry = "check_file" ; entry = "check_file_content"), !bts_api_skip().
bts_doc(d) :- doc(d, path, _), !option("entryPoint", _), file_name(d, file), suffix(".py", file),
    !bts_framework_skip(d), !bts_skip(path),
    (prefix("impl/", path) ; prefix("data_layer/", path) ; prefix("_internal/", path) ; match(__LAYER__, path)),
    source(d, sp), text(sp, raw), bts_trigger(trigger), contains(trigger, raw).
.decl bts_line(d: number, line: number, raw: symbol, normalized: symbol, snippet: symbol)
'''.replace('__LAYER__', q('l' + digits + r'{4}_[^/\n][^/]*/[\s\S]*'))
    # The markers preserve scalar length and every character predicate in these eight source patterns:
    # ASCII literals, word boundaries/classes, whitespace and [^)]. Reconstruct captures from the original line.
    text += f'bts_line(d, l, raw, @replace_all(@replace_all(raw, {q("[" + word + r"&&\W]")}, "Ж"), {q(r"[\w--" + word + "]")}, "�"), @slice(@replace_all(raw, {q(trim)}, ""), 0, 80)) :- bts_doc(d), split_line(d, l, raw), !match({q("(?s)" + space + "*#.*")}, raw).\n'
    text += '.decl bts_occurrence(d: number, line: number, pattern: number, occurrence: number, start: number, matched: symbol, snippet: symbol)\n'
    if len(original.COMPARISON_PATTERNS) != 8:
        raise ValueError('Review the changed source comparison family')
    for i, pattern in enumerate(original.COMPARISON_PATTERNS):
        if pattern.flags != re.U or not pattern.pattern.isascii() or any(
                m.group(1) not in "swb.()'\"" for m in re.finditer(r'\\(.)', pattern.pattern)):
            raise ValueError('Review new character predicates before applying word normalization')
        regex = '(?<whole>' + pattern.pattern.replace(r'\s', space) + ')'
        text += f'bts_occurrence(d, l, {i}, i, a, @slice(raw, a, b), snippet) :- bts_line(d, l, raw, normalized, snippet), match_all({q(regex)}, normalized, i, a, b).\n'
    text += r'''
.decl bts_matched(m: symbol)
bts_matched(m) :- bts_occurrence(_, _, _, _, _, m, _).
.decl bts_dialect(m: symbol)
bts_dialect(m) :- bts_matched(m), contains("dialect.name", m).
.decl bts_backend(m: symbol)
bts_backend(m) :- bts_matched(m),
    (contains("backend_type", m) ; contains("backend ==", m) ; contains("backend in", m)).
.decl bts_substring(m: symbol)
bts_substring(m) :- bts_matched(m), contains(" in ", m), (contains("class_name", m) ; contains("type(", m)).
.decl bts_class(m: symbol)
bts_class(m) :- bts_matched(m), contains("__class__.__name__", m).
.decl bts_code(m: symbol, code: symbol)
bts_code(m, "BT001") :- bts_dialect(m), contains(" in ", m).
bts_code(m, "BT002") :- bts_dialect(m), !contains(" in ", m).
bts_code(m, "BT003") :- bts_backend(m), !bts_dialect(m).
bts_code(m, "BT004") :- bts_substring(m), !bts_dialect(m), !bts_backend(m).
bts_code(m, "BT005") :- bts_class(m), !bts_dialect(m), !bts_backend(m), !bts_substring(m).
bts_code(m, "BT000") :- bts_matched(m), !bts_dialect(m), !bts_backend(m), !bts_substring(m), !bts_class(m).
'''
    for code, message in MESSAGES.items():
        text += f'.violation Backend{code}(at: span, pattern: number, occurrence: number, column: number, snippet: symbol) code {q(code)} {q(message)}\n'
        text += f'Backend{code}(@place(path, l), pattern, occurrence, a + 1, snippet) :- bts_occurrence(d, l, pattern, occurrence, a, matched, snippet), bts_code(matched, {q(code)}), doc(d, path, _).\n'
    return text + r'''
.violation Skip(at: span) code "BTS_SKIP" "true"
Skip(@place("backend-policy", 1)) :- option("entryPoint", "should_skip_file"), option("path", path), bts_skip(path).
.violation Skipped(at: span) code "BTS_SKIPPED" "Path skipped"
Skipped(@place("backend-policy", 1)) :- option("entryPoint", entry),
    (entry = "check_file" ; entry = "check_file_content"), bts_api_skip().
.violation File(at: span, result: symbol) code "BTS_FILE" "{result}"
File(@place("backend-policy", 1), path) :- option("entryPoint", "find_python_files"), option_item("candidates", _, path), suffix(".py", path).
.violation Configuration(at: span, key: symbol, ordinal: number, value: symbol) code "BTS_CONFIG" "{value}"
Configuration(@place("backend-policy", 1), key, i, value) :- option("entryPoint", "configuration"), bts_config(key, i, value).
'''
