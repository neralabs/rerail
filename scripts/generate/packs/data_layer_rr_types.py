"""Translate the pinned type-safety scanner, including its textual signature windows."""
import json
import re
from data_layer_rr_text import module, classes


def regex_search(pattern):
    """A Boolean Python search, with pinned Unicode classes and true word boundaries."""
    word, space, _ = classes()
    if pattern.flags != re.U:
        raise ValueError('Review new source regex flags')
    # These patterns contain no class escapes except whitespace and word boundaries.
    text = pattern.pattern
    if re.search(r'(?<!\\)\\(?![bs])[a-zA-Z]', text):
        raise ValueError('Review new source regex escape')
    # Every boundary in these source expressions borders an ASCII word and a branch
    # edge. Boolean search can consume that delimiter; captures would need offsets.
    pieces, offset = [], 0
    for match in re.finditer(r'(?<!\\)\\b', text):
        pieces.append(text[offset:match.start()])
        if match.start() and re.fullmatch('[A-Za-z0-9_]', text[match.start() - 1]):
            if match.end() < len(text) and text[match.end()] not in '|)':
                raise ValueError('Review boundary inside a source expression')
            pieces.append(r'(?:\z|[^' + word[1:] + ')')
        elif match.start() == 0 or text[match.start() - 1] == '|':
            pieces.append(r'(?:\A|[^' + word[1:] + ')')
        else:
            raise ValueError('Review non-word source boundary')
        offset = match.end()
    pieces.append(text[offset:])
    text = ''.join(pieces).replace(r'\s', space)
    return '(?s:.*?)(?:' + text + ')(?s:.*)'


def type_safety_patterns(source, target, header):
    original = module(source, 'type_safety_patterns')
    q = json.dumps
    _, space, _ = classes()
    trim = r'\A' + space + '+|' + space + r'+\z'
    text = header('type_safety_patterns', target)
    text += '.decl tsp_category(c: symbol, label: symbol)\n'
    text += ''.join(f'tsp_category({q(c.name)}, {q(c.value)}).\n' for c in original.IssueCategory)
    text += '.decl tsp_allowed(c: symbol, p: symbol)\n'
    text += ''.join(f'tsp_allowed({q(c.name)}, {q(p)}).\n' for c, paths in original.ALLOWLIST.items() for p in sorted(paths))
    text += r'''
.decl tsp_source(d: number, t: symbol)
tsp_source(d, t) :- source(d, sp), text(sp, t).
.decl tsp_pattern_trigger(d: number)
'''
    for trigger in original.PATTERN_SOURCE_TRIGGERS:
        text += f'tsp_pattern_trigger(d) :- tsp_source(d, t), contains({q(trigger)}, t).\n'
    text += '.decl tsp_signature_trigger(d: number)\n'
    for trigger in original.SIGNATURE_SOURCE_TRIGGERS:
        text += f'tsp_signature_trigger(d) :- tsp_source(d, t), contains({q(trigger)}, t).\n'
    text += r'''
.decl tsp_text_mode(d: number)
tsp_text_mode(d) :- doc(d, _, _), !option("entryPoint", _).
tsp_text_mode(d) :- doc(d, _, _), option("entryPoint", e),
    (e = "check_file_for_patterns" ; e = "check_file_content_for_patterns").
tsp_text_mode(d) :- tsp_fact_file(d), option("entryPoint", "_issues_for_files").
.decl tsp_fact_file(d: number)
tsp_fact_file(d) :- file_name(d, n), suffix(".py", n), !path_part(d, "tests"), !path_part(d, "test").
.decl tsp_signature_mode(d: number)
tsp_signature_mode(d) :- doc(d, _, _), !option("entryPoint", _).
tsp_signature_mode(d) :- tsp_fact_file(d), option("entryPoint", e),
    (e = "check_function_signatures" ; e = "check_function_signature_content" ; e = "_issues_for_files").
// The content API uses its argument only as a trigger, then the framework reads the file.
.decl tsp_signature_gate(d: number)
tsp_signature_gate(d) :- tsp_signature_trigger(d), !option("callSource", _).
'''
    for trigger in original.SIGNATURE_SOURCE_TRIGGERS:
        text += f'tsp_signature_gate(d) :- doc(d, _, _), option("callSource", t), contains({q(trigger)}, t).\n'
    text += r'''
.decl tsp_issue(d: number, l: number, c: symbol, column: number, ordinal: number, suggestion: symbol, snippet: symbol)
'''
    patterns = [
        ('STR_EXPR_PATTERN', 'STR_UNION_EXPRESSION', "Remove 'str' from union; use Expression or Field only"),
        ('STR_FIELD_PATTERN', 'STR_UNION_FIELD', "Remove 'str' from union; use Expression or Field only"),
        ('EXPR_ANY_ANY_PATTERN', 'EXPRESSION_ANY_ANY', 'Use Expression[Any, T] or Expression[V, T] with specific types'),
        ('DEPRECATED_OP_PATTERN', 'DEPRECATED_COMPARISON_OP', 'Use Expression syntax: field(Model.col) > value'),
        ('DEPRECATED_FILTER_METHOD_PATTERN', 'DEPRECATED_FILTER_METHOD', 'Use .filter(field(Model.col) > value) or .filter(IN(field(Model.col), values))')]
    for ordinal, (name, category, suggestion) in enumerate(patterns):
        rx = q(regex_search(getattr(original, name)))
        text += f'tsp_issue(d, l, {q(category)}, 0, {ordinal}, {q(suggestion)}, @replace_all(t, {q(trim)}, "")) :- tsp_text_mode(d), tsp_pattern_trigger(d), split_line(d, l, t), doc(d, p, _), !tsp_allowed({q(category)}, p), match({rx}, t).\n'
        text += f'.violation TspPattern{ordinal}(at: span) code "TSP_PATTERN" "pattern"\nTspPattern{ordinal}(@place("input.py", 1)) :- option("entryPoint", "pattern_search"), option("patternOrdinal", "{ordinal}"), option("patternText", t), match({rx}, t).\n'
    text += r'''
// The scanner reads splitlines(), rather than the function node's source span.
// Preserve its first-eleven-lines limit and its early textual terminator for order_by.
.decl tsp_function(f: node, d: number, l: number, name: symbol, last: number)
tsp_function(f, d, l, @nfkc(n), last) :- kind(f, "function_definition"), def_name(f, n),
    docof(f, d), line(f, l), tsp_signature_mode(d), tsp_signature_gate(d), !syntax_error(d, _, _),
    last = max e : { within(t, f), !kind(t, "comment"), !child(t, _, _), end_line(t, e) }.
.decl tsp_terminator(f: node, k: number)
tsp_terminator(f, k) :- tsp_function(f, d, l, "order_by", _), split_line(d, k, t), k >= l, k <= l + 10,
    (contains("):", t) ; contains(") ->", t)).
.decl tsp_window(f: node, t: symbol)
tsp_window(f, t) :- tsp_function(f, d, l, name, last), name != "order_by",
    t = join s by k sep "\n" : { split_line(d, k, s), k >= l, k <= last }.
tsp_window(f, t) :- tsp_function(f, d, l, "order_by", _),
    t = join s by k sep "\n" : { split_line(d, k, s), k >= l, k <= l + 10,
        !tsp_terminator(f, k2), k2 < k }.
'''
    # Avoid a negation with an unbound comparison variable: an explicit earlier terminator relation.
    text = text.replace('!tsp_terminator(f, k2), k2 < k', '!tsp_after_terminator(f, k)')
    text += '.decl tsp_after_terminator(f: node, k: number)\ntsp_after_terminator(f, k) :- tsp_function(f, d, _, "order_by", _), split_line(d, k, _), tsp_terminator(f, k2), k2 < k.\n'
    signature_patterns = [
        ('order_by', r'column\s*:\s*str\b|column\s*:\s*[^,)]+\|\s*str\b|str\s*\|\s*[^,)]*column', 'order_by should accept Expression or Field, not str'),
        ('where', r'expression\s*:\s*str\b|:\s*str\s*\|', 'where should accept Expression or Field, not str'),
        ('select', r'\*columns\s*:\s*str\b|columns\s*:\s*str\b|:\s*str\s*\|', 'select should accept Expression or Field, not str'),
        ('group_by', r'\*columns\s*:\s*str\b|columns\s*:\s*str\b|:\s*str\s*\|', 'group_by should accept Expression or Field, not str')]
    text += '.decl tsp_signature_match(f: node, c: symbol, suggestion: symbol)\n'
    for name, pattern, suggestion in signature_patterns:
        category = 'STR_IN_' + name.upper()
        text += f'tsp_signature_match(f, {q(category)}, {q(suggestion)}) :- tsp_function(f, d, _, {q(name)}, _), doc(d, p, _), !tsp_allowed({q(category)}, p), tsp_window(f, t), match({q(regex_search(re.compile(pattern)))}, t).\n'
    text += f'tsp_issue(d, l, c, column, 5 + i, suggestion, @replace_all(t, {q(trim)}, "")) :- tsp_signature_match(f, c, suggestion), tsp_function(f, d, l, _, _), visit_index(f, i), col(f, column), split_line(d, l, t), !tsp_earlier_signature(f, c).\n'
    text += r'''
.decl tsp_earlier_signature(f: node, c: symbol)
tsp_earlier_signature(f, c) :- tsp_signature_match(f, c, _), tsp_signature_match(g, c, _),
    docof(f, d), docof(g, d), line(f, l), line(g, l), earlier(g, f).
.violation TypeSafety(at: span, category: symbol, column: number, ordinal: number, suggestion: symbol, snippet: symbol, label: symbol) code "TSP001"
    "{label}: {suggestion}"
TypeSafety(@place(p, l), c, column, ordinal, suggestion, snippet, label) :- tsp_issue(d, l, c, column, ordinal, suggestion, snippet), doc(d, p, _), tsp_category(c, label).
.violation TspAllowed(at: span) code "TSP_ALLOWED" "allowlisted"
TspAllowed(@place("input.py", 1)) :- option("entryPoint", "is_allowlisted"), option("category", c), tsp_normalized(p), tsp_allowed(c, p).
.decl tsp_normalized(p: symbol)
tsp_normalized(".") :- option("filePath", p), option("basePath", p).
tsp_normalized(@slice(p, strlen(prefix), strlen(p))) :- option("filePath", p), option("basePath", b),
    prefix = cat(@replace_all(b, "/+$", ""), "/"), prefix(prefix, p).
tsp_normalized(p) :- option("filePath", p), option("basePath", b), p != b,
    prefix = cat(@replace_all(b, "/+$", ""), "/"), !prefix(prefix, p).
.violation TspNormalized(at: span, result: symbol) code "TSP_NORMALIZED" "relative path"
TspNormalized(@place("input.py", 1), p) :- option("entryPoint", "normalize_path"), tsp_normalized(p).
.violation TspDiscovered(at: span) code "TSP_DISCOVERED" "source file"
TspDiscovered(@place(p, 1)) :- option("entryPoint", "find_python_files"), doc(d, p, _), file_name(d, n),
    suffix(".py", n), !prefix("test_", n), !suffix("_test.py", n), !tsp_excluded(d).
.decl tsp_excluded(d: number)
tsp_excluded(d) :- path_part(d, p), (p = "tests" ; p = "test" ; p = "vendor" ; p = "__pycache__" ; p = ".git" ; p = "tmp"), !file_name(d, p).
.violation TspDedupe(at: span, ordinal: number) code "TSP_DEDUPE" "first issue"
TspDedupe(@place("input.py", 1), i) :- option("entryPoint", "_dedupe_issues"), option_item("issuePaths", i, _), !tsp_duplicate(i).
.decl tsp_duplicate(i: number)
tsp_duplicate(i) :- option_item("issuePaths", i, p), option_item("issueLines", i, l), option_item("issueCategories", i, c),
    option_item("issuePaths", j, p), option_item("issueLines", j, l), option_item("issueCategories", j, c), j < i.
'''
    return text
