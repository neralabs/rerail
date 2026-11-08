"""Generate grammar escape-hatch relations from the pinned source constants."""
import json
from data_layer_rr_text import module

GROUPS = ['ALLOWLISTED_CLASSES','DATA_VALUE_PARAMS','OPERATOR_PARAMS','_SAFE_PARAM_TYPES_STATIC',
          'SAFE_PARAM_TYPES','TRANSPARENT_CONTAINER_TYPES','SCHEMA_METADATA_CLASSES',
          'CALLSITE_PROTECTED_CLASSES','FORBIDDEN_ESCAPE_HATCHES']


def sql_grammar_escape_hatches(source,target,header):
    original=module(source,'sql_grammar_escape_hatches')
    text=header('sql_grammar_escape_hatches',target)+'#include "python/print.rr"\n\n.decl sgeh_constant(family: symbol, value: symbol)\n'
    for group in GROUPS:
        for value in sorted(getattr(original,group)):
            text+='sgeh_constant('+json.dumps(group)+','+json.dumps(value)+').\n'
    defaults=original.GrammarClassVisitor('',safe_param_types=frozenset()).grammar_class_names
    for value in sorted(defaults):
        text+='sgeh_constant("DEFAULT_GRAMMAR_CLASS_NAMES",'+json.dumps(value)+').\n'
    text+='.decl sgeh_metadata(key: symbol, value: symbol)\n'
    for key in ['ALLOWLIST_PUBLIC_KEY_HEX','ALLOWLIST_SIGNATURE','MAX_ESCAPE_HATCH_VIOLATIONS']:
        text+='sgeh_metadata('+json.dumps(key)+','+json.dumps(str(getattr(original,key)))+').\n'
    text+='''
.violation SGEConstant(at: span, family: symbol, value: symbol) code "SGE_CONSTANT" "{value}"
SGEConstant(@place("", 0), f, v) :- option("entryPoint", "configuration"), sgeh_constant(f, v).
.violation SGEMetadata(at: span, key: symbol, value: symbol) code "SGE_METADATA" "{value}"
SGEMetadata(@place("", 0), k, v) :- option("entryPoint", "configuration"), sgeh_metadata(k, v).
'''
    return text+argument_helpers()+usage_and_reporting(original)+class_analysis(original)+parse_warnings()+production_reporter()


def argument_helpers():
    return r'''
.decl sgeh_kind(n: node, kind: symbol)
sgeh_kind(n, k) :- option_item("astKinds", n, k).
.decl sgeh_edge(p: node, field: symbol, c: node)
sgeh_edge(to_number(p), field, c) :- option_item("astParents", c, p), p != "-1", option_item("astFields", c, field).
.decl sgeh_name(n: node, name: symbol)
sgeh_name(n, s) :- sgeh_kind(n, "Name"), option_item("astIds", n, s).
sgeh_name(n, s) :- sgeh_kind(n, "Attribute"), option_item("astAttrs", n, s).
sgeh_name(n, s) :- sgeh_kind(n, "Subscript"), sgeh_edge(n, "value", c), sgeh_name(c, s).
.decl sgeh_union(n: node)
sgeh_union(n) :- sgeh_kind(n, "BinOp"), sgeh_edge(n, "op", op), sgeh_kind(op, "BitOr").
.decl sgeh_alias(n: node, name: symbol)
sgeh_alias(n, s) :- sgeh_kind(n, "Name"), option_item("astIds", n, s).
sgeh_alias(n, s) :- sgeh_kind(n, "Attribute"), option_item("astAttrs", n, s).
sgeh_alias(n, s) :- sgeh_union(n), sgeh_edge(n, "left", c), sgeh_alias(c, s).
sgeh_alias(n, s) :- sgeh_union(n), sgeh_edge(n, "right", c), sgeh_alias(c, s).
.decl sgeh_container_base(n: node)
sgeh_container_base(n) :- sgeh_kind(n, "Subscript"), sgeh_edge(n, "value", c), sgeh_name(c, name), sgeh_constant("TRANSPARENT_CONTAINER_TYPES", name).
.decl sgeh_literal_word(n: node, word: symbol)
sgeh_literal_word(n, @slice(value, start, end)) :- sgeh_kind(n, "Constant"), option_item("astValueTypes", n, "str"), option_item("astConstantText", n, value),
    match_all("[A-Za-z_][A-Za-z0-9_]*", value, _, start, end).
.decl sgeh_leaf(n: node, name: symbol)
sgeh_leaf(n, s) :- sgeh_kind(n, "Name"), option_item("astIds", n, s), !sgeh_constant("TRANSPARENT_CONTAINER_TYPES", s).
sgeh_leaf(n, s) :- sgeh_kind(n, "Attribute"), option_item("astAttrs", n, s).
sgeh_leaf(n, s) :- sgeh_literal_word(n, s), !sgeh_constant("TRANSPARENT_CONTAINER_TYPES", s).
sgeh_leaf(n, s) :- sgeh_kind(n, "Constant"), option_item("astValueTypes", n, t), t != "str", option_item("astConstantText", n, s).
sgeh_leaf(n, s) :- sgeh_kind(n, "Subscript"), sgeh_edge(n, "value", c), !sgeh_container_base(n), sgeh_leaf(c, s).
sgeh_leaf(n, s) :- sgeh_kind(n, "Subscript"), sgeh_edge(n, "slice", c), sgeh_leaf(c, s).
sgeh_leaf(n, s) :- sgeh_union(n), sgeh_edge(n, "left", c), sgeh_leaf(c, s).
sgeh_leaf(n, s) :- sgeh_union(n), sgeh_edge(n, "right", c), sgeh_leaf(c, s).
sgeh_leaf(n, s) :- sgeh_kind(n, "Tuple"), sgeh_edge(n, "elts", c), sgeh_leaf(c, s).
// Flatten only Subscript.value and BitOr arms. Other expressions are one atom.
// Occurrence keys retain repeated text without string-creating recursion.
.decl sgeh_format_edge(n: node, c: node)
sgeh_format_edge(n, c) :- sgeh_kind(n, "Subscript"), sgeh_edge(n, "value", c).
sgeh_format_edge(n, c) :- sgeh_union(n), sgeh_edge(n, "left", c).
sgeh_format_edge(n, c) :- sgeh_union(n), sgeh_edge(n, "right", c).
.decl sgeh_format_walk(root: node, n: node)
sgeh_format_walk(n, n) :- sgeh_kind(n, _).
sgeh_format_walk(root, c) :- sgeh_format_walk(root, n), sgeh_format_edge(n, c).
.decl sgeh_format_atom(n: node, text: symbol)
sgeh_format_atom(n, s) :- sgeh_kind(n, "Name"), option_item("astIds", n, s).
sgeh_format_atom(n, s) :- sgeh_kind(n, "Attribute"), option_item("astAttrs", n, s).
sgeh_format_atom(n, s) :- sgeh_kind(n, "Constant"), option_item("astConstantText", n, s).
sgeh_format_atom(n, "unknown") :- sgeh_kind(n, k), k != "Name", k != "Attribute", k != "Constant", k != "Subscript", !sgeh_union(n).
.decl sgeh_format(n: node, text: symbol)
sgeh_format(n, text) :- sgeh_kind(n, _), text = join part by at sep " | " : { sgeh_format_walk(n, at), sgeh_format_atom(at, part) }.
.decl sgeh_quoted(raw: symbol, quoted: symbol)
sgeh_quoted(s, @json_str(s)) :- option_item("allowlist", _, s).
.decl sgeh_message(text: symbol)
sgeh_message(cat("{\"allowlist\": [", contents, "], \"version\": 1}")) :- option("entryPoint", _),
    contents = join quoted by raw sep ", " : { sgeh_quoted(raw, quoted) }.
.violation SGEType(at: span, name: symbol) code "SGE_TYPE" "{name}"
SGEType(@place("", 0), s) :- option("entryPoint", "GrammarClassVisitor._extract_leaf_types"), sgeh_leaf(0, s).
SGEType(@place("", 0), s) :- option("entryPoint", "_extract_alias_targets"), sgeh_alias(0, s).
.violation SGEValue(at: span, kind: symbol, value: symbol) code "SGE_VALUE" "{value}"
SGEValue(@place("", 0), "name", s) :- option("entryPoint", "GrammarClassVisitor._get_name"), sgeh_name(0, s).
SGEValue(@place("", 0), "annotation", s) :- option("entryPoint", "GrammarClassVisitor._annotation_to_string"), sgeh_format(0, s).
SGEValue(@place("", 0), "message", s) :- option("entryPoint", "_compute_signature_message"), sgeh_message(s).
SGEValue(@place("", 0), "public_key", s) :- option("entryPoint", "_get_public_key_bytes"), option("publicKey", s).
.violation SGETrue(at: span) code "SGE_TRUE" "true"
SGETrue(@place("", 0)) :- option("entryPoint", "_verify_allowlist_integrity"), option("publicKey", key), option("signature", signature),
    signature != "NEEDS_SIGNING", sgeh_message(message), @ed25519_ok(@replace_all(key, "[\\t-\\r ]", ""), message, signature) = 1.
'''


def usage_and_reporting(original):
    from data_layer_rr_text import character_class
    text='\n.decl sgeh_forbidden(name: symbol, ordinal: number)\n'
    for index,name in enumerate(original.FORBIDDEN_ESCAPE_HATCHES):
        text+='sgeh_forbidden('+json.dumps(name)+','+str(index)+').\n'
    whitespace=character_class(str.isspace)
    text+='\n.decl sgeh_usage_line(d: number, l: number, text: symbol)\n'
    text+='sgeh_usage_line(d, l, raw) :- doc(d, _, _), lf_line(d, l, raw), !prefix("#", @replace_all(raw, '+json.dumps('^'+whitespace+'*')+', "")).\n'
    bar='='*60
    hint='Mechanical cleanup:\n  uv run python scripts/fix_grammar_linter_violations.py --write\n  uv run python scripts/rollup_sql_fragments.py --write\n\n'
    text+='\n.decl sgeh_report_hint(text: symbol)\nsgeh_report_hint('+json.dumps(hint)+').\n'
    text+='\n.decl sgeh_bar(text: symbol)\nsgeh_bar('+json.dumps(bar)+').\n'
    return text+r'''
.violation SGEUsage(at: span, class_name: symbol, ordinal: number) code "SGE_USAGE" "Usage of forbidden escape hatch class '{class_name}'"
SGEUsage(@place(path, l), name, i) :- option("entryPoint", "scan_usages"), option("originalPath", path), sgeh_usage_line(_, l, text),
    sgeh_forbidden(name, i), (contains(cat(name, "("), text) ; contains(cat("import ", name), text) ; contains(cat(", ", name), text)).
.decl sgeh_report_row(i: number, path: symbol, line: number, class_name: symbol, reason: symbol)
sgeh_report_row(i, path, to_number(line), name, reason) :- option_item("reportPaths", i, path), option_item("reportLines", i, line), option_item("reportClasses", i, name), option_item("reportReasons", i, reason).
.decl sgeh_report_count(n: number, ceiling: number)
sgeh_report_count(n, to_number(c)) :- option("entryPoint", "_report_violations"), option("ceiling", c), n = count : { sgeh_report_row(_, _, _, _, _) }.
.decl sgeh_report_item(i: number, path: symbol, line: number, text: symbol)
sgeh_report_item(i, p, l, cat(p, ":", to_string(l), "\n  Class: ", c, "\n  Reason: ", r, "\n\n")) :- sgeh_report_row(i, p, l, c, r).
.decl sgeh_report_body(text: symbol)
sgeh_report_body(text) :- sgeh_report_count(_, _), text = join value by (path, line, i) sep "" : { sgeh_report_item(i, path, line, value) }.
.decl sgeh_report_exit(value: number)
sgeh_report_exit(1) :- sgeh_report_count(n, c), n > c, option("checkMode", "true").
sgeh_report_exit(0) :- sgeh_report_count(n, c), n > c, !option("checkMode", "true").
sgeh_report_exit(0) :- sgeh_report_count(n, c), n <= c.
.decl sgeh_report_tail(text: symbol)
sgeh_report_tail(cat(bar, "\nESCAPE HATCH COUNT GREW: ", to_string(n), " > ", to_string(c), " (ceiling)\nYou introduced new escape hatches. Fix them before merging.\n", bar, "\n\n")) :- sgeh_report_count(n, c), n > c, sgeh_bar(bar).
sgeh_report_tail(cat("Escape hatch count (", to_string(n), ") is below ceiling (", to_string(c), "). Consider reducing MAX_ESCAPE_HATCH_VIOLATIONS.\n")) :- sgeh_report_count(n, c), n < c.
sgeh_report_tail(cat("Escape hatch count (", to_string(n), ") is at ceiling (", to_string(c), ").\n")) :- sgeh_report_count(n, c), n = c.
.violation SGEReport(at: span, output: symbol, status: number) code "SGE_REPORT" "report"
SGEReport(@place("", 0), cat("\n", bar, "\nSQL GRAMMAR ESCAPE HATCH VIOLATIONS: ", to_string(n), "\n", bar, "\n\n", body, hint, tail), status) :-
    sgeh_report_count(n, _), sgeh_bar(bar), sgeh_report_body(body), sgeh_report_hint(hint), sgeh_report_tail(tail), sgeh_report_exit(status).
'''


def class_analysis(original):
    triggers=original.SQL_GRAMMAR_ESCAPE_HATCH_EXTRACTOR.source_triggers
    text='\n.decl sgeh_trigger(d: number)\n'
    text+='sgeh_trigger(d) :- source(d, sp), text(sp, s), ('+' ; '.join('contains('+json.dumps(x)+', s)' for x in triggers)+').\n'
    return text+r'''
.decl sgeh_source_doc(d: number, path: symbol)
sgeh_source_doc(d, p) :- doc(d, logical, _), option_item("logicalPaths", i, logical), option_item("originalPaths", i, p), sgeh_trigger(d), !syntax_error(d, _, _), !sgeh_file_bad(d).
.decl sgeh_src_edge(n: node, c: node)
sgeh_src_edge(n, c) :- kind(n, "type"), child(n, c, _).
sgeh_src_edge(n, c) :- kind(n, "parenthesized_expression"), child(n, c, _), !kind(c, "comment").
.decl sgeh_src_name(n: node, name: symbol)
sgeh_src_name(n, @nfkc(s)) :- kind(n, "identifier"), text(n, s).
sgeh_src_name(n, @nfkc(s)) :- attr_of(n, _, s).
sgeh_src_name(n, @nfkc(s)) :- kind(n, "dotted_name"), child(n, c, i), text(c, s), i = max j : { child(n, _, j) }.
sgeh_src_name(n, s) :- sgeh_src_edge(n, c), sgeh_src_name(c, s).
sgeh_src_name(n, s) :- kind(n, "generic_type"), child(n, c, 0), sgeh_src_name(c, s).
sgeh_src_name(n, s) :- kind(n, "subscript"), field(n, "value", c), sgeh_src_name(c, s).
.decl sgeh_src_union(n: node)
sgeh_src_union(n) :- kind(n, "union_type").
sgeh_src_union(n) :- kind(n, "binary_operator"), op(n, "|").
.decl sgeh_src_leaf_edge(n: node, c: node)
sgeh_src_leaf_edge(n, c) :- sgeh_src_edge(n, c).
sgeh_src_leaf_edge(n, c) :- (kind(n, "type_parameter") ; kind(n, "tuple") ; kind(n, "union_type")), child(n, c, _).
sgeh_src_leaf_edge(n, c) :- kind(n, "binary_operator"), op(n, "|"), (field(n, "left", c) ; field(n, "right", c)).
sgeh_src_leaf_edge(n, c) :- kind(n, "generic_type"), child(n, args, 1), kind(args, "type_parameter"), child(args, c, _).
sgeh_src_leaf_edge(n, c) :- kind(n, "subscript"), field(n, "subscript", c).
sgeh_src_leaf_edge(n, c) :- kind(n, "generic_type"), child(n, c, 0), !sgeh_src_container(n).
sgeh_src_leaf_edge(n, c) :- kind(n, "subscript"), field(n, "value", c), !sgeh_src_container(n).
.decl sgeh_src_container(n: node)
sgeh_src_container(n) :- (kind(n, "generic_type") ; kind(n, "subscript")), sgeh_src_name(n, s), sgeh_constant("TRANSPARENT_CONTAINER_TYPES", s).
.decl sgeh_src_leaf(n: node, name: symbol)
sgeh_src_leaf(n, @nfkc(s)) :- kind(n, "identifier"), text(n, s), !sgeh_constant("TRANSPARENT_CONTAINER_TYPES", @nfkc(s)).
sgeh_src_leaf(n, @nfkc(s)) :- attr_of(n, _, s).
sgeh_src_leaf(n, s) :- kind(n, "dotted_name"), sgeh_src_name(n, s).
sgeh_src_leaf(n, @slice(value, start, end)) :- str_const(n, value), match_all("[A-Za-z_][A-Za-z0-9_]*", value, _, start, end), !sgeh_constant("TRANSPARENT_CONTAINER_TYPES", @slice(value, start, end)).
sgeh_src_leaf(n, s) :- annotation_constant_repr(n, s).
sgeh_src_leaf(n, s) :- sgeh_src_leaf_edge(n, c), sgeh_src_leaf(c, s).
.decl sgeh_class(c: node, d: number, name: symbol, line: number, ordinal: number)
sgeh_class(c, d, @nfkc(s), l, i) :- kind(c, "class_definition"), docof(c, d), sgeh_source_doc(d, _), def_name(c, s), line(c, l), visit_index(c, i).
.decl sgeh_base(c: node, name: symbol)
sgeh_base(c, s) :- sgeh_class(c, _, _, _, _), field(c, "superclasses", args), child(args, b, _), sgeh_src_name(b, s).
.decl sgeh_src_alias(n: node, name: symbol)
sgeh_src_alias(n, @nfkc(s)) :- kind(n, "identifier"), text(n, s).
sgeh_src_alias(n, @nfkc(s)) :- attr_of(n, _, s).
sgeh_src_alias(n, s) :- kind(n, "dotted_name"), sgeh_src_name(n, s).
sgeh_src_alias(n, s) :- sgeh_src_edge(n, c), sgeh_src_alias(c, s).
sgeh_src_alias(n, s) :- sgeh_src_union(n), child(n, c, _), sgeh_src_alias(c, s).
.decl sgeh_alias_target(name: symbol, target: symbol)
sgeh_alias_target(@nfkc(s), t) :- assign_target(a, target), kind(target, "identifier"), text(target, s), !assign_ann(a, _), assign_value(a, value), docof(a, d), sgeh_source_doc(d, _), sgeh_src_alias(value, t).
.decl sgeh_render_base(name: symbol)
sgeh_render_base("SqlRenderable"). sgeh_render_base("SqlClause"). sgeh_render_base("SqlFragment"). sgeh_render_base("SqlParam").
sgeh_render_base(name) :- sgeh_class(c, _, name, _, _), sgeh_base(c, base), sgeh_render_base(base).
.decl sgeh_render(name: symbol)
sgeh_render(name) :- sgeh_render_base(name).
sgeh_render(name) :- sgeh_alias_target(name, target), sgeh_render(target).
.decl sgeh_safe(name: symbol)
sgeh_safe(name) :- sgeh_constant("_SAFE_PARAM_TYPES_STATIC", name).
sgeh_safe(name) :- sgeh_render(name).
.decl sgeh_custom_types()
sgeh_custom_types() :- option("customGrammarProvided", "true"), option("customSafeProvided", "true").
.decl sgeh_effective_render(name: symbol)
sgeh_effective_render(name) :- sgeh_render(name), !sgeh_custom_types().
sgeh_effective_render(name) :- sgeh_custom_types(), option_item("grammarClassNames", _, name).
.decl sgeh_effective_safe(name: symbol)
sgeh_effective_safe(name) :- sgeh_safe(name), !sgeh_custom_types().
sgeh_effective_safe(name) :- sgeh_custom_types(), option_item("safeParamTypes", _, name).
.decl sgeh_parameter(c: node, p: node, name: symbol, annotation: node, line: number, ordinal: number)
sgeh_parameter(c, nm, @nfkc(name), ann, l, i) :- sgeh_class(c, _, _, _, _), field(c, "body", body), body_stmt(body, stmt, _), defstmt(f, stmt),
    kind(f, "function_definition"), !op(f, "async"), def_name(f, raw), @nfkc(raw) = "__init__", sig_param(f, _, nm, _, "pos", ann, _), !unset(ann),
    text(nm, name), @nfkc(name) != "self", line(nm, l), visit_index(nm, i).
.decl sgeh_unsafe(p: node, name: symbol)
sgeh_unsafe(p, name) :- sgeh_parameter(_, p, _, ann, _, _), sgeh_src_leaf(ann, name), !sgeh_effective_safe(name).
.decl sgeh_name_exempt(name: symbol)
sgeh_name_exempt(name) :- (sgeh_constant("ALLOWLISTED_CLASSES", name) ; sgeh_constant("SCHEMA_METADATA_CLASSES", name) ; sgeh_constant("CALLSITE_PROTECTED_CLASSES", name)).
.decl sgeh_param_exempt(p: node)
sgeh_param_exempt(p) :- sgeh_parameter(_, p, name, _, _, _), sgeh_unsafe(p, "str"), count : { sgeh_unsafe(p, _) } = 1,
    (sgeh_constant("DATA_VALUE_PARAMS", name) ; sgeh_constant("OPERATOR_PARAMS", name)).
.decl sgeh_skip_classes(d: number)
sgeh_skip_classes(d) :- sgeh_source_doc(d, p), option_item("directoryPaths", _, p), contains("test", p).
.decl sgeh_candidate(d: number, line: number, class_name: symbol, phase: number, ordinal: number, reason: symbol)
sgeh_candidate(d, l, name, 0, i, cat("FORBIDDEN: '", name, "' is a known escape hatch class that must be deleted")) :- sgeh_class(c, d, name, l, i), !sgeh_skip_classes(d), sgeh_forbidden(name, _).
sgeh_candidate(d, l, name, 0, i, cat("Parameter '", param, "' uses unsafe type(s): ", types, ". Use branded types, enums, or grammar classes instead.")) :-
    sgeh_class(c, d, name, _, _), !sgeh_skip_classes(d), !sgeh_forbidden(name, _), !sgeh_name_exempt(name), sgeh_base(c, base), sgeh_effective_render(base),
    sgeh_parameter(c, p, param, _, l, i), sgeh_unsafe(p, _), !sgeh_param_exempt(p), types = join type by type sep ", " : { sgeh_unsafe(p, type) }.
sgeh_candidate(d, l, name, 1, i, cat("Usage of forbidden escape hatch class '", name, "'")) :- sgeh_source_doc(d, _), sgeh_usage_line(d, l, text), sgeh_forbidden(name, i),
    (contains(cat(name, "("), text) ; contains(cat("import ", name), text) ; contains(cat(", ", name), text)).
.decl sgeh_first(d: number, line: number, name: symbol, phase: number, ordinal: number)
sgeh_first(d, l, n, phase, ordinal) :- sgeh_candidate(d, l, n, phase, ordinal, _),
    count : { sgeh_candidate(d, l, n, p, i, _), (p < phase ; p = phase, i < ordinal) } = 0.
.violation SGEFile(at: span, class_name: symbol, phase: number, ordinal: number, reason: symbol) code "SGE_FILE" "{reason}"
SGEFile(@place(path, l), name, phase, i, reason) :- (option("entryPoint", "scan_file") ; option("entryPoint", "_collect_violations")),
    sgeh_candidate(d, l, name, phase, i, reason), sgeh_first(d, l, name, phase, i), sgeh_source_doc(d, path).
'''


def parse_warnings():
    from data_layer_rr_text import character_class
    ws=character_class(str.isspace)
    sep=r'\n\r\x0b\x0c\x1c-\x1e\x85\x{2028}\x{2029}'
    line='[^'+sep+']'; no_close='[^'+sep+')]'
    pattern='(?s)'+ws+'*def (?:'+line+r'*\(:'+line+'*|'+no_close+r'*\('+no_close+'*)(?:['+sep+'].*)?'
    return '.decl sgeh_malformed(d: number)\nsgeh_malformed(d) :- source(d, sp), text(sp, raw), match('+json.dumps(pattern)+', raw).\n'+r'''
.decl sgeh_file_bad(d: number)
sgeh_file_bad(d) :- option("entryPoint", "scan_file"), sgeh_malformed(d).
.violation SGEWarning(at: span, path: symbol) code "SGE_WARNING" "Warning: Could not parse {path}"
SGEWarning(@place(path, 1), path) :- option("entryPoint", "scan_file"), doc(d, logical, _), option_item("logicalPaths", i, logical), option_item("originalPaths", i, path), sgeh_malformed(d).
.violation SGESyntaxWarning(at: span, path: symbol, message: symbol, filename: symbol, line: number) code "SGE_WARNING"
    "Warning: Could not parse {path}: SyntaxError: {message} (source_0_{filename}, line {line})"
SGESyntaxWarning(@place(path, l), path, message, filename, l) :- option("entryPoint", "scan_file"), doc(d, logical, _),
    option_item("logicalPaths", i, logical), option_item("originalPaths", i, path), option_item("originalNames", i, filename),
    sgeh_trigger(d), syntax_error(d, l, message), !sgeh_malformed(d).
'''


def production_reporter():
    return r'''
#include "project.rr"
.decl sgeh_default_hidden(d: number)
sgeh_default_hidden(d) :- path_part(d, part), (part = ".git" ; part = ".venv" ; part = "venv" ; part = "__pycache__" ; part = "tmp" ; part = "tests" ; part = "test").
sgeh_source_doc(d, path) :- !option("entryPoint", _), doc(d, path, _), doc_role(d, "primary"), file_name(d, file), suffix(".py", file),
    (prefix("api/", path) ; prefix("impl/", path)), !sgeh_default_hidden(d), sgeh_trigger(d), !syntax_error(d, _, _).
.decl sgeh_unique(d: number, line: number, name: symbol, phase: number, ordinal: number, reason: symbol)
sgeh_unique(d, l, name, phase, i, reason) :- sgeh_candidate(d, l, name, phase, i, reason), sgeh_first(d, l, name, phase, i).
.decl sgeh_default_count(n: number)
sgeh_default_count(n) :- !option("entryPoint", _), n = count : { sgeh_unique(_, _, _, _, _, _) }.
.decl sgeh_default_quoted(name: symbol, quoted: symbol)
sgeh_default_quoted(name, @json_str(name)) :- sgeh_constant("ALLOWLISTED_CLASSES", name).
.decl sgeh_default_message(text: symbol)
sgeh_default_message(cat("{\"allowlist\": [", contents, "], \"version\": 1}")) :-
    contents = join quoted by name sep ", " : { sgeh_default_quoted(name, quoted) }.
.decl sgeh_default_verified()
sgeh_default_verified() :- sgeh_metadata("ALLOWLIST_PUBLIC_KEY_HEX", key), sgeh_metadata("ALLOWLIST_SIGNATURE", signature),
    signature != "NEEDS_SIGNING", sgeh_default_message(message), @ed25519_ok(key, message, signature) = 1.
.decl sgeh_script_path(path: symbol)
sgeh_script_path(path) :- !option("entryPoint", _), option("sourceModulePath", path).
sgeh_script_path(cat(@replace_all(root, "/+$", ""), "/scripts/lint_sql_grammar_escape_hatches.py")) :- !option("entryPoint", _), workspace_root(root), !option("sourceModulePath", _).
.violation EscapeHatch(at: span, class_name: symbol, phase: number, ordinal: number, count: number, ceiling: number, reason: symbol) code "escape_hatch"
    "{class_name}: {reason} (count {count} > ceiling {ceiling})"
EscapeHatch(@place(path, l), name, phase, i, n, to_number(ceiling), reason) :- !option("entryPoint", _), sgeh_default_verified(),
    sgeh_default_count(n), sgeh_metadata("MAX_ESCAPE_HATCH_VIOLATIONS", ceiling), n > to_number(ceiling),
    sgeh_unique(d, l, name, phase, i, reason), sgeh_source_doc(d, path).
.violation InvalidAllowlist(at: span) code "invalid_allowlist_signature"
    "SQL grammar escape hatch allowlist signature verification failed"
InvalidAllowlist(@place(path, 1)) :- !option("entryPoint", _), !sgeh_default_verified(), sgeh_script_path(path).
.severity EscapeHatch error
.severity InvalidAllowlist error
'''
