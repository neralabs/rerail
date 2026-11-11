"""Port per-line cache detection, its four Boolean regexes, and reviewed exemptions."""
import json
from data_layer_rr_text import module, classes, search


def connection_caches(source, target, header):
    original = module(source, 'connection_caches')
    text = header('connection_caches', target).replace('#include "python/ast.rr"\n', '')
    text = text.replace(', "**/test_*.py"', '').replace('"**/test/**"', '"**/.pytest_cache/**", "**/test/**"')
    text += '.scope dirs option("apiInputDirs")\n'
    text += '.decl cc_pattern(i: number, pattern: symbol, description: symbol)\n'
    text += '.decl cc_regex(i: number, source: symbol, native: symbol)\n'
    text += '.decl cc_allowed(file: symbol, substring: symbol)\n'
    text += '.decl cc_config(key: symbol, i: number, value: symbol)\n'
    text += '.decl cc_skip(part: symbol)\n'
    text += '.decl cc_root(path: symbol)\n'
    quote = json.dumps
    tables = dict(descriptions=[s for _, s in original.ALL_PATTERNS], directories=original.SCAN_DIRECTORIES,
        skip=sorted(original.SKIP_PARTS), allowed_files=[x[0] for x in original.ALLOWED_CACHES],
        allowed_substrings=[x[1] for x in original.ALLOWED_CACHES], allowed_reasons=[x[2] for x in original.ALLOWED_CACHES])
    for key, values in tables.items():
        for i, value in enumerate(values):
            text += f'cc_config({quote(key)}, {i}, {quote(value)}).\n'
    for i, (pattern, description) in enumerate(original.ALL_PATTERNS):
        text += f'cc_regex({i}, {quote(pattern.pattern)}, {quote(search(pattern))}).\n'
    text += 'cc_config("patterns", i, source) :- cc_regex(i, source, _).\n'
    text += 'cc_pattern(i, native, description) :- cc_regex(i, _, native), cc_config("descriptions", i, description).\n'
    text += 'cc_allowed(file, substring) :- cc_config("allowed_files", i, file), cc_config("allowed_substrings", i, substring).\n'
    text += 'cc_skip(part) :- cc_config("skip", _, part).\n'
    text += 'cc_root(path) :- cc_config("directories", _, path).\n'
    _, space, _ = classes()
    text += '.decl cc_trim(raw: symbol, stripped: symbol)\n'
    text += 'cc_trim(raw, @replace_all(raw, ' + quote(r'\A' + space + '+|' + space + r'+\z') + ', "")) :- split_line(_, _, raw).\n'
    return text + r'''
.decl cc_hidden(d: number)
cc_hidden(d) :- path_part(d, part), cc_skip(part).
cc_hidden(d) :- doc(d, _, _), option_item("rootParts", _, part), cc_skip(part).
.decl cc_framework_hidden(d: number)
cc_framework_hidden(d) :- path_part(d, part),
    (part = ".git" ; part = ".venv" ; part = "venv" ; part = "__pycache__" ; part = "tmp" ; part = "test" ; part = "tests").
.decl cc_selected(d: number)
cc_selected(d) :- doc(d, _, _), option("entryPoint", entry), (entry = "scan_file_content" ; entry = "find_violations").
.decl cc_trigger(d: number)
cc_trigger(d) :- split_line(d, _, source),
    (contains("cache", source) ; contains("BranchId", source) ; contains("RevisionId", source) ;
     contains("branch_tip", source) ; contains("tip_revision", source)).
cc_selected(d) :- doc(d, path, _), !option("entryPoint", _), cc_root(root), prefix(cat(root, "/"), path),
    !cc_framework_hidden(d), file_name(d, f), suffix(".py", f), cc_trigger(d).
.decl cc_line(d: number, l: number, snippet: symbol)
cc_line(d, l, snippet) :- cc_selected(d), !cc_hidden(d), split_line(d, l, raw), cc_trim(raw, snippet), !prefix("#", snippet).
.decl cc_line_allowed(d: number, l: number)
cc_line_allowed(d, l) :- cc_line(d, l, snippet), doc(d, path, _), cc_allowed(file, substring),
    suffix(file, path), contains(substring, snippet).
.violation Cache(at: span, ordinal: number, snippet: symbol, description: symbol) code "CC001" "{description}"
Cache(@place(path, l), i, snippet, description) :- cc_line(d, l, snippet), doc(d, path, _),
    cc_pattern(i, pattern, description), match(pattern, snippet), !cc_line_allowed(d, l).

.violation Pattern(at: span) code "CC_PATTERN" "true"
Pattern(@place("cache-policy", 1)) :- option("entryPoint", "pattern_search"), option("patternOrdinal", selector),
    option("patternText", input), cc_pattern(i, regex, _), selector = to_string(i), match(regex, input).
.violation Allowed(at: span) code "CC_ALLOWED" "true"
Allowed(@place("cache-policy", 1)) :- option("entryPoint", "_is_allowed"), option("path", path),
    option("line", line), cc_allowed(file, substring), suffix(file, path), contains(substring, line).
.violation Skip(at: span) code "CC_SKIP" "true"
Skip(@place("cache-policy", 1)) :- option("entryPoint", "skip_path"), option_item("pathParts", _, part), cc_skip(part).
.violation Configuration(at: span, key: symbol, ordinal: number, value: symbol) code "CC_CONFIG" "{value}"
Configuration(@place("cache-policy", 1), key, i, value) :- option("entryPoint", "configuration"), cc_config(key, i, value).
'''
