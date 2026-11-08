"""Translate raw render-string classification, native previews, and report formatting."""
import ast
import json
import re
from data_layer_rr_render_fstrings import render_context,render_positions,formats as fstring_formats


def render_raw_strings(source,target,header):
    tree=ast.parse((source/'scripts/lint_render_raw_strings.py').read_text())
    method=next(n for n in ast.walk(tree) if isinstance(n,ast.FunctionDef) and n.name=='_is_allowed_string_arg')
    allowed=next(ast.literal_eval(n.value) for n in ast.walk(method) if isinstance(n,ast.Assign) and any(isinstance(t,ast.Name) and t.id=='allowed_funcs' for t in n.targets))
    text=header('render_raw_strings',target).replace('.mode block\n','.mode log\n')
    text=re.sub(r'^\.scope exclude .*$', '.scope exclude "**/.git/**", "**/.venv/**", "**/venv/**", "**/__pycache__/**", "**/tmp/**"',text,flags=re.M)
    text+=render_context().replace('rf_','rs_')+render_positions().replace('rf_','rs_')
    text+='\n.decl rs_allowed_name(name: symbol)\n'+''.join('rs_allowed_name('+json.dumps(v)+').\n' for v in sorted(allowed))
    return text+projection()+decisions()+outputs()+formats(tree)


def projection():
    return r'''
.decl rs_piece(n: node, i: number, value: symbol)
rs_piece(n, i, value) :- !rs_graph(), str_part(n, i, value, _, _).
rs_piece(n, 0, value) :- option_item("astKinds", n, "Constant"), option_item("astValueTypes", n, "str"), option_item("astConstantText", n, value).
.decl rs_plain(n: node)
rs_plain(n) :- !rs_graph(), str_const(n, _).
rs_plain(n) :- option_item("astKinds", n, "Constant"), option_item("astValueTypes", n, "str").
.decl rs_identity(n: node, i: number, identity: symbol)
rs_identity(n, i, cat(to_string(n), ":", to_string(i))) :- !rs_graph(), rs_piece(n, i, _).
rs_identity(n, 0, identity) :- rs_piece(n, 0, _), option_item("astIdentity", n, identity).
.decl rs_anchor_candidate(n: node, i: number, anchor: node, depth: number)
rs_anchor_candidate(n, i, a, depth) :- !rs_graph(), str_part_offset(n, i, off), node(d, n, _, _, _), within(a, n), node(d, a, _, start, end), start <= off, off < end, ast_depth(a, depth).
.decl rs_piece_position(n: node, i: number, d: number, line: number, ordinal: number)
rs_piece_position(n, i, d, l, order) :- !rs_graph(), rs_piece(n, i, _), docof(n, d), str_part_start(n, i, l, _),
 depth = max k : { rs_anchor_candidate(n, i, _, k) },
 anchor = max a : { rs_anchor_candidate(n, i, a, depth) }, visit_index(anchor, order).
rs_piece_position(n, 0, 0, to_number(l), n) :- rs_piece(n, 0, _), option_item("astLines", n, l).
.decl rs_name(n: node, name: symbol)
rs_name(n, @nfkc(name)) :- !rs_graph(), unparen(n, p), ast_name(p, name).
rs_name(n, name) :- option_item("astKinds", n, "Name"), option_item("astIds", n, name).
.decl rs_attr(n: node, base: node, name: symbol)
rs_attr(n, base, @nfkc(name)) :- !rs_graph(), unparen(n, p), attr_of(p, base, name).
rs_attr(n, base, name) :- option_item("astKinds", n, "Attribute"), rs_edge(n, "value", base), option_item("astAttrs", n, name).
.decl rs_call(c: node, fn: node)
rs_call(c, fn) :- !rs_graph(), kind(c, "call"), field(c, "function", fn).
rs_call(c, fn) :- option_item("astKinds", c, "Call"), rs_edge(c, "func", fn).
.decl rs_func_name(n: node, name: symbol)
rs_func_name(n, name) :- rs_name(n, name).
rs_func_name(n, name) :- rs_attr(n, _, name).
.decl rs_posarg(c: node, i: number, a: node)
rs_posarg(c, i, a) :- !rs_graph(), rs_call(c, _), field(c, "arguments", al), child(al, raw, k), !kind(raw, "keyword_argument"), !kind(raw, "dictionary_splat"), !kind(raw, "comment"), unparen(raw, a),
 i = count : { child(al, other, j), j < k, !kind(other, "keyword_argument"), !kind(other, "dictionary_splat"), !kind(other, "comment") }.
rs_posarg(c, to_number(i), a) :- rs_call(c, _), rs_edge(c, "args", a), option_item("astOrdinals", a, i).
.decl rs_selected(d: number)
rs_selected(d) :- rs_path(d, _), !rs_graph(), !rs_collect_skip(d), !syntax_error(d, _, _), option("entryPoint", _), file_name(d, file), suffix(".py", file).
rs_selected(0) :- rs_graph().
rs_selected(d) :- rs_path(d, path), !option("entryPoint", _), prefix("impl/common/l2100_sql_grammar/", path), suffix(".py", path), !rs_hidden(d), !syntax_error(d, _, _).
.decl rs_stats_selected(d: number)
rs_stats_selected(d) :- rs_selected(d), !rs_graph(), source(d, sp), text(sp, source), contains("render", source).
rs_stats_selected(0) :- rs_graph().
.decl rs_active(n: node, f: node, d: number, method: symbol, cls: symbol, has: number)
rs_active(n, f, d, method, cls, has) :- rs_ancestor(n, f), rs_render(f, method), rs_context(f, cls, has), rs_position(n, d, _, _), rs_selected(d).
.decl rs_repr_value(value: symbol, printed: symbol)
rs_repr_value(value, @python_repr(value)) :- rs_piece(_, _, value).
'''


def decisions():
    return r'''
.decl rs_special(c: node, arg: node, kind: symbol)
rs_special(c, arg, "template_call") :- rs_call(c, fn), rs_name(fn, "Template"), rs_posarg(c, 0, arg), rs_plain(arg).
rs_special(c, arg, "static_call") :- rs_call(c, fn), rs_attr(fn, base, "static"), rs_name(base, "SqlFragment"), rs_posarg(c, 0, arg), rs_plain(arg).
.decl rs_suppressed(n: node, i: number)
rs_suppressed(n, i) :- rs_piece(n, i, _), rs_identity(n, i, identity), rs_special(call, arg, _), rs_identity(arg, 0, identity),
 rs_active(call, _, _, _, _, _), rs_position(call, _, _, call_order), rs_piece_position(n, i, _, _, order), call_order < order.
.decl rs_raise_or_assert(n: node)
rs_raise_or_assert(n) :- !rs_graph(), rs_ancestor(n, a), (kind(a, "raise_statement") ; kind(a, "assert_statement")).
rs_raise_or_assert(n) :- rs_ancestor(n, a), option_item("astKinds", a, k), (k = "Raise" ; k = "Assert").
.decl rs_docstring(n: node, f: node)
rs_docstring(n, f) :- !rs_graph(), rs_render(f, _), docstring(f, n).
rs_docstring(n, f) :- rs_render(f, _), rs_edge(f, "body", st), option_item("astOrdinals", st, "0"), option_item("astKinds", st, "Expr"), rs_edge(st, "value", value), option_item("astKinds", value, "Constant"), option_item("astIdentity", value, identity), option_item("astIdentity", n, identity).
.decl rs_allowed(n: node)
rs_allowed(n) :- rs_plain(n), rs_posarg(call, _, n), rs_call(call, fn), rs_func_name(fn, name), rs_allowed_name(name).
.decl rs_special_preview(c: node, value: symbol)
rs_special_preview(c, cat("Template(\"", v, "\")")) :- rs_special(c, arg, "template_call"), rs_piece(arg, 0, value), @slice(value, 40, 41) = "", v = value.
rs_special_preview(c, cat("Template(\"", @slice(value, 0, 37), "...\")")) :- rs_special(c, arg, "template_call"), rs_piece(arg, 0, value), @slice(value, 40, 41) != "".
rs_special_preview(c, cat("SqlFragment.static(\"", v, "\")")) :- rs_special(c, arg, "static_call"), rs_piece(arg, 0, value), @slice(value, 40, 41) = "", v = value.
rs_special_preview(c, cat("SqlFragment.static(\"", @slice(value, 0, 37), "...\")")) :- rs_special(c, arg, "static_call"), rs_piece(arg, 0, value), @slice(value, 40, 41) != "".
.decl rs_hit(n: node, piece: number, d: number, line: number, ordinal: number, method: symbol, cls: symbol, has: number, kind: symbol, preview: symbol)
rs_hit(n, i, d, l, order, method, cls, has, "string_literal", @slice(printed, 0, 50)) :- rs_piece(n, i, value), @slice(value, 1, 2) != "", rs_repr_value(value, printed),
 rs_active(n, f, d, method, cls, has), rs_piece_position(n, i, d, l, order), !rs_suppressed(n, i), !rs_raise_or_assert(n), !rs_docstring(n, f), !rs_allowed(n).
rs_hit(c, -1, d, l, order, method, cls, has, kind, preview) :- rs_special(c, _, kind), rs_active(c, _, d, method, cls, has), rs_position(c, d, l, order), rs_special_preview(c, preview).
'''


def outputs():
    text=r'''
.violation RSFact(at: span, method_name: symbol, class_name: symbol, has_class: number, ordinal: number, violation_type: symbol, preview: symbol) code "RS_FACT" "{preview}"
RSFact(@place(path, l), method, cls, has, order, kind, preview) :- option("entryPoint", _), rs_hit(_, _, d, l, order, method, cls, has, kind, preview), rs_path(d, path).
.decl rs_report_stats(d: number, methods: number, violations: number)
rs_report_stats(d, methods, violations) :- option("entryPoint", _), rs_path(d, _),
 methods = count : { rs_render(f, _), rs_position(f, d, _, _), rs_stats_selected(d) },
 violations = count : { rs_hit(_, _, d, _, _, _, _, _, _, _) }.
.decl rs_report_include(d: number, selected: number)
rs_report_include(d, 1) :- rs_report_stats(d, methods, violations), (methods > 0 ; violations > 0).
rs_report_include(d, 0) :- rs_report_stats(d, 0, 0).
.violation RSReport(at: span, render_methods_scanned: number, violation_count: number, include_report: number) code "RS_REPORT" "report"
RSReport(@place(path, 0), methods, violations, selected) :- rs_report_stats(d, methods, violations), rs_report_include(d, selected), rs_path(d, path).
.violation RSRead(at: span) code "RS_READ" "read"
RSRead(@place("", 0)) :- option("entryPoint", "source_gate"), option("providedSource", source), contains("render", source).
.violation RSName(at: span) code "RS_NAME" "render"
RSName(@place("", 0)) :- option("entryPoint", "_is_render_method"), option("nameProbe", name), rs_render_name(name).
'''
    for label,kind in [('RenderRawLiteral','string_literal'),('RenderTemplate','template_call'),('RenderStatic','static_call')]:
        text+=f'''
.violation {label}(at: span, ordinal: number, method: symbol, preview: symbol) code "{kind}" "{{method}}: {{preview}}"
{label}(@place(path, l), order, name, preview) :- !option("entryPoint", _), rs_hit(_, _, d, l, order, name, "", 0, "{kind}", preview), rs_path(d, path).
{label}(@place(path, l), order, method, preview) :- !option("entryPoint", _), rs_hit(_, _, d, l, order, name, cls, 1, "{kind}", preview), rs_path(d, path), method = cat(cls, ".", name).
.severity {label} info
'''
    return text


def formats(tree):
    q=json.dumps
    hint_fn=next(n for n in tree.body if isinstance(n,ast.FunctionDef) and n.name=='_autofix_hint_lines')
    hints=ast.literal_eval(next(n.value for n in hint_fn.body if isinstance(n,ast.Return)))
    common=fstring_formats().split('.violation RFFormatted',1)[0].replace('rf_','rs_').replace('RF','RS')
    common=common.replace('F-STRING','RAW STRING').replace('f-string','raw string').replace('f-strings','raw strings')
    common=common.replace('method: symbol, preview: symbol)','method: symbol, kind: symbol, preview: symbol)')
    common=common.replace('rs_fmt_v(to_number(owner), i, line, method, preview)', 'rs_fmt_v(to_number(owner), i, line, method, kind, preview)')
    common=common.replace('rs_fmt_method_name(i, method), option_item("formatViolationPreviews"','rs_fmt_method_name(i, method), option_item("formatViolationTypes", i, kind), option_item("formatViolationPreviews"')
    common=common.replace('rs_fmt_v(r, _, _, _, _)','rs_fmt_v(r, _, _, _, _, _)').replace('rs_fmt_v(_, _, _, _, _)','rs_fmt_v(_, _, _, _, _, _)').replace('rs_fmt_v(r, _, _, method, _)','rs_fmt_v(r, _, _, method, _, _)').replace('rs_fmt_v(r, i, line, method, preview)','rs_fmt_v(r, i, line, method, kind, preview)').replace('cat("    Line ", line, ": ", preview)','cat("    Line ", line, " [", kind, "]: ", preview)')
    common+='\n.decl rs_hint(i: number, value: symbol)\n'+''.join('rs_hint('+str(i)+','+q(v)+').\n' for i,v in enumerate(hints))
    return common+r'''
.decl rs_fmt_kind(kind: symbol, count: number, first: number)
rs_fmt_kind(kind, count, first) :- rs_fmt_v(_, _, _, _, kind, _), count = count : { rs_fmt_v(_, _, _, _, kind, _) }, first = min i : { rs_fmt_v(_, i, _, _, kind, _) }.
.violation RSFormatted(at: span, mode: symbol, value: symbol) code "RS_FORMAT" "{value}"
RSFormatted(@place("", 0), "summary", cat(head, "No raw strings found in render() methods.")) :- !rs_fmt_error("summary", _), rs_fmt_summary_heading(head), rs_fmt_totals(_, _, 0, _).
RSFormatted(@place("", 0), "summary", cat(head, "BY VIOLATION TYPE\n", SHORTBAR, "\n", types, "\n\nBY FILE\n", SHORTBAR, "\n", files, "\n\n", HINTS, "\n")) :- !rs_fmt_error("summary", _), rs_fmt_summary_heading(head), rs_fmt_totals(_, _, count, _), count > 0,
 types = join value by (negative, first) sep "\n" : { rs_fmt_kind(kind, n, first), negative = 0 - n, value = cat("  ", kind, ": ", to_string(n)) },
 files = join value by (negative, r) sep "\n" : { rs_fmt_report(r, path, _), rs_fmt_count(r, n), n > 0, negative = 0 - n, value = cat(path, ": ", to_string(n), " violations") }.
RSFormatted(@place("", 0), "detailed", cat(DETAIL, "No raw strings found in render() methods.")) :- !rs_fmt_error("detailed", _), rs_fmt_totals(_, _, 0, _).
RSFormatted(@place("", 0), "detailed", cat(DETAIL, "Total violations: ", to_string(count), "\n\n", files, "\n", HINTS)) :- !rs_fmt_error("detailed", _), rs_fmt_totals(_, _, count, _), count > 0,
 files = join value by (path, r) sep "\n" : { rs_fmt_file(r, path, value) }.
.violation RSHint(at: span, ordinal: number, value: symbol) code "RS_HINT" "{value}"
RSHint(@place("", 0), i, value) :- option("entryPoint", "_autofix_hint_lines"), rs_hint(i, value).
'''.replace('SHORTBAR',q('-'*40)).replace('HINTS',q('\n'.join(hints))).replace('DETAIL',q('='*80+'\nRENDER() RAW STRING LINT REPORT - DETAILED\n'+'='*80+'\n\n'))
