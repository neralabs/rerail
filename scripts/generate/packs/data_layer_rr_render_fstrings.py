"""Translate render-function contexts, decoded f-string previews, and reports."""
import re


def render_fstrings(source, target, header):
    text=header('render_fstrings',target)
    text=re.sub(r'^\.scope exclude .*$', '.scope exclude "**/.git/**", "**/.venv/**", "**/venv/**", "**/__pycache__/**", "**/tmp/**"',text,flags=re.M)
    return text+r'''
.decl rf_graph()
rf_graph() :- option_item("astKinds", 0, _).
.decl rf_edge(p: node, field: symbol, c: node)
rf_edge(p, "", c) :- !rf_graph(), ast_parent(c, p).
rf_edge(to_number(p), field, c) :- option_item("astParents", c, p), p != "-1", option_item("astFields", c, field).
.decl rf_ancestor(n: node, a: node)
rf_ancestor(n, a) :- rf_edge(a, _, n).
rf_ancestor(n, a) :- rf_edge(p, _, n), rf_ancestor(p, a).
.decl rf_function(f: node, name: symbol)
rf_function(f, @nfkc(name)) :- !rf_graph(), kind(f, "function_definition"), def_name(f, name).
rf_function(f, name) :- option_item("astKinds", f, k), (k = "FunctionDef" ; k = "AsyncFunctionDef"), option_item("astNames", f, name).
.decl rf_class(c: node, name: symbol)
rf_class(c, @nfkc(name)) :- !rf_graph(), kind(c, "class_definition"), def_name(c, name).
rf_class(c, name) :- option_item("astKinds", c, "ClassDef"), option_item("astNames", c, name).
.decl rf_render_name(name: symbol)
rf_render_name(name) :- rf_function(_, name), (name = "render" ; prefix("render_", name)).
rf_render_name(name) :- option("nameProbe", name), (name = "render" ; prefix("render_", name)).
.decl rf_shadowed(f: node)
rf_shadowed(f) :- rf_function(f, _), rf_ancestor(f, a), rf_function(a, name), rf_render_name(name).
.decl rf_render(f: node, name: symbol)
rf_render(f, name) :- rf_function(f, name), rf_render_name(name), !rf_shadowed(f).
.decl rf_current_class(f: node, c: node)
rf_current_class(f, c) :- rf_render(f, _), rf_ancestor(f, _), c = max a : { rf_ancestor(f, a), rf_class(a, _) }.
.decl rf_context(f: node, cls: symbol, has_class: number)
rf_context(f, cls, 1) :- rf_current_class(f, c), rf_class(c, cls).
rf_context(f, "", 0) :- rf_render(f, _), !rf_current_class(f, _).
.decl rf_concat_piece(n: node)
rf_concat_piece(n) :- child(p, n, _), kind(p, "concatenated_string").
.decl rf_joined(n: node)
rf_joined(n) :- !rf_graph(), fstring(n), !rf_concat_piece(n).
rf_joined(n) :- !rf_graph(), kind(n, "format_specifier").
rf_joined(n) :- option_item("astKinds", n, "JoinedStr").
.decl rf_piece_owner(lit: node, i: number, owner: node)
rf_piece_owner(lit, i, lit) :- !rf_graph(), str_part(lit, i, _, _, 0), rf_joined(lit).
rf_piece_owner(lit, i, owner) :- !rf_graph(), str_part(lit, i, _, _, 1), str_part_offset(lit, i, off),
 node(d, lit, _, _, _), within(owner, lit), kind(owner, "format_specifier"), node(d, owner, _, start, end), start <= off, off < end,
 start = max begin : { within(s, lit), kind(s, "format_specifier"), node(d, s, _, begin, stop), begin <= off, off < stop }.
.decl rf_interpolation(owner: node, expr: node)
rf_interpolation(n, e) :- !rf_graph(), rf_joined(n), kind(n, "string"), child(n, e, _), kind(e, "interpolation").
rf_interpolation(n, e) :- !rf_graph(), rf_joined(n), kind(n, "concatenated_string"), child(n, s, _), kind(s, "string"), child(s, e, _), kind(e, "interpolation").
rf_interpolation(n, e) :- !rf_graph(), kind(n, "format_specifier"), child(n, e, _), kind(e, "format_expression").
.decl rf_item(n: node, position: number, rank: number, value: symbol)
rf_item(n, off, 1, value) :- rf_piece_owner(lit, i, n), str_part(lit, i, value, _, _), str_part_offset(lit, i, off).
rf_item(n, end, 0, "{...}") :- rf_interpolation(n, e), node(_, e, _, _, end).
rf_item(n, to_number(i), 1, value) :- rf_joined(n), rf_edge(n, "values", c), option_item("astKinds", c, "Constant"), option_item("astOrdinals", c, i), option_item("astConstantText", c, value).
rf_item(n, to_number(i), 1, "{...}") :- rf_joined(n), rf_edge(n, "values", c), option_item("astKinds", c, "FormattedValue"), option_item("astOrdinals", c, i).
.decl rf_preview(n: node, value: symbol)
rf_preview(n, cat("f\"", value, "\"")) :- rf_joined(n), raw = join piece by (at, rank) sep "" : { rf_item(n, at, rank, piece) }, @slice(raw, 50, 51) = "", value = raw.
rf_preview(n, cat("f\"", @slice(raw, 0, 47), "...\"")) :- rf_joined(n), raw = join piece by (at, rank) sep "" : { rf_item(n, at, rank, piece) }, @slice(raw, 50, 51) != "".
.decl rf_position(n: node, d: number, l: number, i: number)
rf_position(n, d, l, i) :- !rf_graph(), docof(n, d), line(n, l), visit_index(n, i).
rf_position(n, 0, to_number(l), n) :- option_item("astKinds", n, _), option_item("astLines", n, l).
.decl rf_path(d: number, path: symbol)
rf_path(d, original) :- !rf_graph(), doc(d, path, _), doc_role(d, "primary"), option_item("documentPaths", i, path), option_item("originalPaths", i, original).
rf_path(d, path) :- !rf_graph(), doc(d, path, _), doc_role(d, "primary"), !option_item("documentPaths", _, path).
rf_path(0, path) :- rf_graph(), option("originalPath", path).
.decl rf_hidden(d: number)
rf_hidden(d) :- path_part(d, part), (part = ".git" ; part = ".venv" ; part = "venv" ; part = "__pycache__" ; part = "tmp").
.decl rf_collect_skip(d: number)
rf_collect_skip(d) :- option("entryPoint", "collect_reports"), rf_path(d, path), contains("__pycache__", path).
.decl rf_selected(d: number)
rf_selected(d) :- rf_path(d, _), !rf_graph(), !rf_collect_skip(d), !syntax_error(d, _, _), option("entryPoint", _), file_name(d, file), suffix(".py", file), source(d, sp), text(sp, source), (contains("render", source) ; contains("f\"", source) ; contains("f'", source)).
rf_selected(0) :- rf_graph().
rf_selected(d) :- rf_path(d, path), !option("entryPoint", _), prefix("impl/common/l2100_sql_grammar/", path), suffix(".py", path), !rf_hidden(d), !syntax_error(d, _, _), source(d, sp), text(sp, source), (contains("render", source) ; contains("f\"", source) ; contains("f'", source)).
.decl rf_stats_selected(d: number)
rf_stats_selected(d) :- rf_selected(d), !rf_graph(), source(d, sp), text(sp, source), contains("render", source).
rf_stats_selected(0) :- rf_graph().
.decl rf_hit(n: node, d: number, l: number, i: number, method: symbol, cls: symbol, has_class: number, preview: symbol)
rf_hit(n, d, l, i, method, cls, has, preview) :- rf_joined(n), rf_ancestor(n, f), rf_render(f, method), rf_context(f, cls, has), rf_preview(n, preview), rf_position(n, d, l, i), rf_selected(d).
.violation RFFact(at: span, method_name: symbol, class_name: symbol, has_class: number, ordinal: number, preview: symbol) code "RF_FACT" "{preview}"
RFFact(@place(path, l), method, cls, has, i, preview) :- option("entryPoint", _), rf_hit(_, d, l, i, method, cls, has, preview), rf_path(d, path).
.decl rf_report_stats(d: number, methods: number, violations: number)
rf_report_stats(d, methods, violations) :- option("entryPoint", _), rf_path(d, _),
 methods = count : { rf_render(f, _), rf_position(f, d, _, _), rf_stats_selected(d) },
 violations = count : { rf_hit(_, d, _, _, _, _, _, _) }.
.decl rf_report_include(d: number, selected: number)
rf_report_include(d, 1) :- rf_report_stats(d, methods, violations), (methods > 0 ; violations > 0).
rf_report_include(d, 0) :- rf_report_stats(d, 0, 0).
.violation RFReport(at: span, render_methods_scanned: number, violation_count: number, include_report: number) code "RF_REPORT" "report"
RFReport(@place(path, 0), methods, violations, selected) :- rf_report_stats(d, methods, violations), rf_report_include(d, selected), rf_path(d, path).
.violation RFPreview(at: span, ordinal: number, preview: symbol) code "RF_PREVIEW" "{preview}"
RFPreview(@place("", 0), n, preview) :- option("entryPoint", "preview"), rf_preview(n, preview).
.violation RFRead(at: span) code "RF_READ" "read"
RFRead(@place("", 0)) :- option("entryPoint", "source_gate"), option("providedSource", source), contains("render", source).
.violation RFName(at: span) code "RF_NAME" "render"
RFName(@place("", 0)) :- option("entryPoint", "_is_render_method"), option("nameProbe", name), rf_render_name(name).
.violation RenderFString(at: span, ordinal: number, preview: symbol, method: symbol) code "render_fstring" "{method}() uses {preview}"
RenderFString(@place(path, l), i, preview, method) :- !option("entryPoint", _), rf_hit(_, d, l, i, method, "", 0, preview), rf_path(d, path).
RenderFString(@place(path, l), i, preview, method) :- !option("entryPoint", _), rf_hit(_, d, l, i, name, cls, 1, preview), rf_path(d, path), method = cat(cls, ".", name).
'''+formats()


def formats():
    import json
    q=json.dumps
    bar='='*80
    title=bar+'\nRENDER() F-STRING LINT REPORT\n'+bar+'\n\nSUMMARY\n'+'-'*40+'\n'
    detail=bar+'\nRENDER() F-STRING LINT REPORT - DETAILED\n'+bar+'\n\n'
    return r'''
.decl rf_fmt_report(r: number, path: symbol, methods: number)
rf_fmt_report(r, path, to_number(methods)) :- option_item("formatReportPaths", r, path), option_item("formatReportMethods", r, methods).
.decl rf_fmt_method_name(i: number, method: symbol)
rf_fmt_method_name(i, name) :- option_item("formatViolationMethods", i, name), option_item("formatViolationClasses", i, "").
rf_fmt_method_name(i, cat(cls, ".", name)) :- option_item("formatViolationMethods", i, name), option_item("formatViolationClasses", i, cls), cls != "".
.decl rf_fmt_v(r: number, i: number, line: symbol, method: symbol, preview: symbol)
rf_fmt_v(to_number(owner), i, line, method, preview) :- option_item("formatViolationOwners", i, owner), option_item("formatViolationLines", i, line), rf_fmt_method_name(i, method), option_item("formatViolationPreviews", i, preview).
.decl rf_fmt_count(r: number, n: number)
rf_fmt_count(r, n) :- rf_fmt_report(r, _, _), n = count : { rf_fmt_v(r, _, _, _, _) }.
.decl rf_fmt_totals(files: number, methods: number, violations: number, positive: number)
rf_fmt_totals(files, methods, violations, positive) :- option("entryPoint", "format"),
 files = count : { rf_fmt_report(_, _, _) }, methods = sum m : { rf_fmt_report(_, _, m) },
 violations = count : { rf_fmt_v(_, _, _, _, _) }, positive = count : { rf_fmt_count(_, n), n > 0 }.
.decl rf_fmt_summary_heading(value: symbol)
rf_fmt_summary_heading(cat(TITLE, "Total files scanned: ", to_string(files), "\nTotal render() methods scanned: ", to_string(methods), "\nFiles with f-string violations: ", to_string(positive), "\nTotal f-string violations: ", to_string(violations), "\n\n")) :- rf_fmt_totals(files, methods, violations, positive).
.decl rf_fmt_method(r: number, method: symbol, value: symbol)
rf_fmt_method(r, method, cat("  ", method, "():\n", lines, "\n")) :- rf_fmt_v(r, _, _, method, _),
 lines = join value by i sep "\n" : { rf_fmt_v(r, i, line, method, preview), value = cat("    Line ", line, ": ", preview) }.
.decl rf_fmt_file(r: number, path: symbol, value: symbol)
rf_fmt_file(r, path, cat(FILEBAR, "\nFILE: ", path, "\n  Violations: ", to_string(count), "\n\n", methods)) :-
 rf_fmt_report(r, path, _), rf_fmt_count(r, count), count > 0,
 methods = join value by name sep "\n" : { rf_fmt_method(r, name, value) }.
.decl rf_fmt_bad(r: number, error: symbol)
rf_fmt_bad(r, error) :- rf_fmt_count(r, count), count > 0, option_item("formatReportErrors", r, error), error != "".
.decl rf_fmt_error(mode: symbol, error: symbol)
rf_fmt_error("summary", error) :- rf_fmt_bad(r, error), rf_fmt_count(r, count),
 count = max n : { rf_fmt_bad(s, _), rf_fmt_count(s, n) },
 r = min s : { rf_fmt_bad(s, _), rf_fmt_count(s, count) }.
rf_fmt_error("detailed", error) :- rf_fmt_bad(r, error), option_item("formatReportOriginalPaths", r, path),
 path = min p : { rf_fmt_bad(s, _), option_item("formatReportOriginalPaths", s, p) },
 r = min s : { rf_fmt_bad(s, _), option_item("formatReportOriginalPaths", s, path) }.
.violation RFFormatError(at: span, mode: symbol, value: symbol) code "RF_FORMAT_ERROR" "{value}"
RFFormatError(@place("", 0), mode, error) :- rf_fmt_error(mode, error).
.violation RFFormatted(at: span, mode: symbol, value: symbol) code "RF_FORMAT" "{value}"
RFFormatted(@place("", 0), "summary", cat(head, "No f-strings found in render() methods.")) :- !rf_fmt_error("summary", _), rf_fmt_summary_heading(head), rf_fmt_totals(_, _, 0, _).
RFFormatted(@place("", 0), "summary", cat(head, "BY FILE\n", SHORTBAR, "\n", files, "\n")) :- !rf_fmt_error("summary", _), rf_fmt_summary_heading(head), rf_fmt_totals(_, _, count, _), count > 0,
 files = join value by (negative, r) sep "\n" : { rf_fmt_report(r, path, _), rf_fmt_count(r, n), n > 0, negative = 0 - n, value = cat(path, ": ", to_string(n), " violations") }.
RFFormatted(@place("", 0), "detailed", cat(DETAIL, "No f-strings found in render() methods.")) :- !rf_fmt_error("detailed", _), rf_fmt_totals(_, _, 0, _).
RFFormatted(@place("", 0), "detailed", cat(DETAIL, "Total violations: ", to_string(count), "\n\n", files)) :- !rf_fmt_error("detailed", _), rf_fmt_totals(_, _, count, _), count > 0,
 files = join value by (path, r) sep "\n" : { rf_fmt_file(r, path, value) }.
'''.replace('TITLE',q(title)).replace('FILEBAR',q('-'*60)).replace('SHORTBAR',q('-'*40)).replace('DETAIL',q(detail))
