"""Translate pinned Python regex indicators and precedence to native value relations."""
import ast
import json
import re
from data_layer_rr_text import module,classes,character_class


def search_exact(pattern):
    text=pattern.pattern
    if not text.isascii() or pattern.flags & ~(re.U|re.I):raise ValueError(('Unsupported regex alphabet or flags',text,pattern.flags))
    # These scanners have one fixed-width negative prefix assertion. Consuming that
    # prefix preserves Boolean search, including the start-of-string alternative.
    text=text.replace('(?<!%)',r'(?:\A|[^%])')
    if '(?<' in text or '(?=' in text or '(?!' in text:raise ValueError(('Untranslated assertion',text))
    return ('(?i)' if pattern.flags&re.I else '')+'(?s:.*)(?:'+text+')(?s:.*)'


def normalization():
    # A finite alphabet quotient for ASCII source regexes. Distinct non-ASCII
    # representatives retain Python 3.13 word, decimal and whitespace membership.
    q=json.dumps
    word=character_class(lambda c:ord(c)>127 and (c=='_' or c.isalnum()) and not c.isdecimal())
    digit=character_class(lambda c:ord(c)>127 and c.isdecimal())
    space=character_class(lambda c:ord(c)>127 and c.isspace() or c in '\x1c\x1d\x1e\x1f')
    other=character_class(lambda c:ord(c)>127 and not c.isalnum() and not c.isspace())
    text='.decl rsp_input(value: symbol)\nrsp_input(p) :- rsp_pattern(p).\nrsp_input(@convert(p, "upperCase", "none")) :- rsp_pattern(p).\n'
    text+='.decl rsp_normal(value: symbol, insensitive: number, normalized: symbol)\n'
    for flag in [0,1]:
        value='p' if flag==0 else '@replace_all(@replace_all(@replace_all(p, "[İı]", "i"), "ſ", "s"), "K", "k")'
        for pattern,replacement in [(digit,'٠'),(word,'漢'),(space,'\u00a0'),(other,'☃')]:value='@replace_all('+value+','+q(pattern)+','+q(replacement)+')'
        text+='rsp_normal(p,'+str(flag)+','+value+') :- rsp_input(p).\n'
    return text


def regex_sql_parsing(source,target,header):
    original=module(source,'regex_sql_parsing');tree=ast.parse((source/'scripts/lint_regex_sql_parsing.py').read_text());text=header('regex_sql_parsing',target)+'\n.decl rsp_constant(family: symbol, ordinal: number, pattern: symbol, flags: number)\n'
    for family,name in [('false','_FALSE_POSITIVE_INDICATORS'),('sql','_SQL_SYNTAX_INDICATORS'),('error','_ERROR_MSG_INDICATORS')]:
        patterns=getattr(original,name)
        text+=f'.decl rsp_{family}(value: symbol)\n'
        for index,p in enumerate(patterns):
            text+='rsp_constant('+json.dumps(family)+','+str(index)+','+json.dumps(p.pattern)+','+str(p.flags)+').\n'
            text+='rsp_'+family+'(p) :- rsp_pattern(p), rsp_normal(p, '+str(int(bool(p.flags&re.I)))+', normalized), match('+json.dumps(search_exact(p))+', normalized).\n'
    function=next(n for n in tree.body if isinstance(n,ast.FunctionDef) and n.name=='_classify_pattern')
    calls=sorted([n for n in ast.walk(function) if isinstance(n,ast.Call) and isinstance(n.func,ast.Attribute) and isinstance(n.func.value,ast.Name) and n.func.value.id=='re' and n.func.attr=='search'],key=lambda n:(n.lineno,n.col_offset))
    for i,n in enumerate(calls):
        flags=re.I if len(n.args)>2 else 0
        compiled=re.compile(ast.literal_eval(n.args[0]),flags);value='@convert(p, "upperCase", "none")' if isinstance(n.args[1],ast.Name) and n.args[1].id=='upper' else 'p'
        text+=f'.decl rsp_test_{i}(value: symbol)\nrsp_test_{i}(p) :- rsp_pattern(p), rsp_normal('+value+', '+str(int(bool(compiled.flags&re.I)))+', normalized), match('+json.dumps(search_exact(compiled))+', normalized).\n'
    text+='\n.decl rsp_pattern(value: symbol)\nrsp_pattern(p) :- option("patternProbe", p).\n'
    return text+normalization()+classifier()+graph_kernel()+native_files()+transport_metadata()


def classifier():
    return r'''
.decl rsp_func(value: symbol)
rsp_func(f) :- option("funcProbe", f).
.decl rsp_choice(pattern: symbol, func: symbol, rank: number, category: symbol)
rsp_choice(p, f, 0, "ERROR_MSG_PARSE") :- rsp_pattern(p), rsp_func(f), rsp_error(p).
rsp_choice(p, f, 1, "SQL_COMMENT_STRIP") :- rsp_pattern(p), rsp_func(f), rsp_sql(p), rsp_test_0(p).
rsp_choice(p, f, 2, "SQL_PARAM_NORMALIZE") :- rsp_pattern(p), rsp_func(f), rsp_sql(p), rsp_test_1(p), !rsp_test_2(p).
rsp_choice(p, f, 3, "SQL_PARAM_NORMALIZE") :- rsp_pattern(p), rsp_func(f), rsp_sql(p), rsp_test_3(p).
rsp_choice(p, f, 4, "SQL_PARAM_NORMALIZE") :- rsp_pattern(p), rsp_func(f), rsp_sql(p), rsp_test_4(p).
rsp_choice(p, f, 5, "CONSTRAINT_PARSE") :- rsp_pattern(p), rsp_func(f), rsp_sql(p), rsp_test_5(p).
rsp_choice(p, f, 6, "CONSTRAINT_PARSE") :- rsp_pattern(p), rsp_func(f), rsp_sql(p), rsp_test_6(p).
rsp_choice(p, f, 7, "CONSTRAINT_PARSE") :- rsp_pattern(p), rsp_func(f), rsp_sql(p), rsp_test_7(p), !rsp_test_8(p).
rsp_choice(p, f, 8, "TRIGGER_PARSE") :- rsp_pattern(p), rsp_func(f), rsp_sql(p), rsp_test_9(p).
rsp_choice(p, f, 9, "SQL_STRUCTURE_PARSE") :- rsp_pattern(p), rsp_func(f), rsp_sql(p), rsp_test_10(p).
rsp_choice(p, "sub", 10, "SQL_REWRITE") :- rsp_pattern(p), rsp_func("sub"), rsp_sql(p).
rsp_choice(p, f, 11, "SQL_STRUCTURE_PARSE") :- rsp_pattern(p), rsp_func(f), rsp_sql(p).
.decl rsp_category(pattern: symbol, func: symbol, category: symbol)
rsp_category(p, f, c) :- rsp_choice(p, f, k, c), !rsp_false(p), k = min rank : { rsp_choice(p, f, rank, _) }.
.violation RSPCategory(at: span, value: symbol) code "RSP_CATEGORY" "{value}"
RSPCategory(@place("", 0), c) :- option("entryPoint", "_classify_pattern"), option("patternProbe", p), option("funcProbe", f), rsp_category(p, f, c).
.violation RSPPredicate(at: span) code "RSP_TRUE" "true"
RSPPredicate(@place("", 0)) :- option("entryPoint", "_is_false_positive"), option("patternProbe", p), rsp_false(p).
RSPPredicate(@place("", 0)) :- option("entryPoint", "_is_sql_syntax_pattern"), option("patternProbe", p), rsp_sql(p).
RSPPredicate(@place("", 0)) :- option("entryPoint", "_is_error_msg_pattern"), option("patternProbe", p), rsp_error(p).
'''


def graph_kernel():
    return r'''
.decl rsp_kind(n: node, kind: symbol)
rsp_kind(n, k) :- option_item("astKinds", n, k).
.decl rsp_edge(p: node, field: symbol, c: node)
rsp_edge(to_number(p), field, c) :- option_item("astParents", c, p), p != "-1", option_item("astFields", c, field).
.decl rsp_depth(n: node, d: number)
rsp_depth(n, to_number(d)) :- option_item("astDepths", n, d).
.decl rsp_e_item(n: node, i: number, value: symbol)
rsp_e_item(n, to_number(i), v) :- rsp_kind(n, "JoinedStr"), rsp_edge(n, "values", c), rsp_kind(c, "Constant"), option_item("astOrdinals", c, i), option_item("astConstantText", c, v).
rsp_e_item(n, to_number(i), "{...}") :- rsp_kind(n, "JoinedStr"), rsp_edge(n, "values", c), rsp_kind(c, k), k != "Constant", option_item("astOrdinals", c, i).
.decl rsp_extract(n: node, value: symbol)
rsp_extract(n, v) :- rsp_kind(n, "Constant"), option_item("astValueTypes", n, "str"), option_item("astConstantText", n, v).
rsp_extract(n, v) :- rsp_kind(n, "JoinedStr"), v = join text by i sep "" : { rsp_e_item(n, i, text) }.
rsp_pattern(v) :- rsp_extract(_, v).
rsp_func(method) :- rsp_call(_, method, _).
.decl rsp_call(c: node, method: symbol, pattern: symbol)
rsp_call(c, method, pattern) :- rsp_kind(c, "Call"), rsp_edge(c, "func", f), rsp_kind(f, "Attribute"), option_item("astAttrs", f, method), rsp_edge(f, "value", receiver), rsp_kind(receiver, "Name"), option_item("astIds", receiver, "re"),
 (method = "search" ; method = "sub" ; method = "match" ; method = "findall" ; method = "fullmatch" ; method = "split" ; method = "compile"),
 rsp_edge(c, "args", first), option_item("astOrdinals", first, "0"), rsp_extract(first, pattern).
.decl rsp_selected_call(c: node)
rsp_selected_call(c) :- option("entryPoint", "check_file"), rsp_call(c, _, _).
rsp_selected_call(c) :- option("entryPoint", "collector"), rsp_call(c, _, _).
rsp_selected_call(0) :- option("entryPoint", "_check_re_call"), rsp_call(0, m, _), m != "compile".
rsp_selected_call(0) :- option("entryPoint", "_check_re_compile"), rsp_call(0, "compile", _).
.decl rsp_short(value: symbol, preview: symbol)
rsp_short(v, v) :- rsp_pattern(v), @slice(v, 60, 61) = "".
rsp_short(v, cat(@slice(v, 0, 57), "...")) :- rsp_pattern(v), @slice(v, 60, 61) != "".
.violation RSPExtract(at: span, value: symbol) code "RSP_EXTRACT" "{value}"
RSPExtract(@place("", 0), v) :- option("entryPoint", "_extract_regex_string"), rsp_extract(0, v).
.decl rsp_handler(method: symbol, handler: symbol)
rsp_handler("compile", "_check_re_compile").
rsp_handler(m, "_check_re_call") :- rsp_func(m), m != "compile".
.violation RSPFact(at: span, depth: number, ordinal: number, category: symbol, method: symbol, preview: symbol, handler: symbol) code "RSP_FACT" "re.{method}() with SQL pattern: {preview}"
RSPFact(@place(path, to_number(l)), depth, c, category, method, preview, handler) :- option("originalPath", path), rsp_selected_call(c), rsp_call(c, method, pattern), rsp_category(pattern, method, category), rsp_depth(c, depth), option_item("astLines", c, l), rsp_short(pattern, text), preview = @python_repr(text), rsp_handler(method, handler).
.decl rsp_meta(name: symbol, value: symbol)
rsp_meta("name", "regex_sql_parsing").
rsp_meta("description", "Regex-based SQL parsing anti-patterns").
rsp_meta("directories", "public").
rsp_meta("include_tests", "false").
.violation RSPMetadata(at: span, name: symbol, value: symbol) code "RSP_METADATA" "{value}"
RSPMetadata(@place("", 0), name, value) :- option("entryPoint", "metadata"), rsp_meta(name, value).
'''


def native_files():
    from data_layer_rr_render_fstrings import fstring_parts
    from data_layer_rr_text import character_class
    shared=fstring_parts().replace('rf_','rsp_native_')
    shared+='.decl rsp_native_graph()\nrsp_native_graph() :- rsp_kind(0, _).\n.decl rsp_native_edge(p: node, field: symbol, c: node)\nrsp_native_edge(p, f, c) :- rsp_edge(p, f, c).\n'
    layer='(?s)l'+character_class(str.isdecimal)+r'{4}_[^/]+/.*'
    shared+='\n.decl rsp_doc_selected(d: number)\nrsp_doc_selected(d) :- doc(d, path, _), doc_role(d, "primary"), !rsp_hidden(d), !syntax_error(d, _, _), (prefix("impl/", path) ; prefix("data_layer/", path) ; prefix("_internal/", path) ; match('+json.dumps(layer)+', path)), source(d, span), text(span, source), (contains("re.", source) ; contains("SELECT", source) ; contains("INSERT", source) ; contains("UPDATE", source) ; contains("DELETE", source) ; contains("CHECK", source) ; contains("ON DELETE", source)).\n'
    shared+=r'''
.decl rsp_hidden(d: number)
rsp_hidden(d) :- path_part(d, p), (p = ".git" ; p = ".venv" ; p = "venv" ; p = "__pycache__" ; p = "tmp" ; p = "test" ; p = "tests").
rsp_extract(n, value) :- !rsp_native_graph(), str_const(n, value).
rsp_extract(n, value) :- !rsp_native_graph(), rsp_native_joined(n), value = join part by (at, rank) sep "" : { rsp_native_item(n, at, rank, part) }.
rsp_call(c, @nfkc(method), value) :- !rsp_native_graph(), kind(c, "call"), field(c, "function", rawfn), unparen(rawfn, fn), attr_of(fn, rawbase, method), unparen(rawbase, base), ast_name(base, re), @nfkc(re) = "re",
 (@nfkc(method) = "search" ; @nfkc(method) = "sub" ; @nfkc(method) = "match" ; @nfkc(method) = "findall" ; @nfkc(method) = "fullmatch" ; @nfkc(method) = "split" ; @nfkc(method) = "compile"),
 field(c, "arguments", args), child(args, first, i), !kind(first, "keyword_argument"), !kind(first, "dictionary_splat"), !kind(first, "comment"),
 0 = count : { child(args, other, j), j < i, !kind(other, "keyword_argument"), !kind(other, "dictionary_splat"), !kind(other, "comment") }, unparen(first, pattern), rsp_extract(pattern, value).

'''
    for category in ['SQL_STRUCTURE_PARSE','SQL_REWRITE','SQL_COMMENT_STRIP','SQL_PARAM_NORMALIZE','ERROR_MSG_PARSE','TRIGGER_PARSE','CONSTRAINT_PARSE']:
        name='RegexSql'+category.replace('_','')
        shared+=f'''
.violation {name}(at: span, ordinal: number, method: symbol, preview: symbol) code "{category}" "re.{{method}}() with SQL pattern: {{preview}}"
{name}(@place(path, l), i, method, preview) :- !option("entryPoint", _), rsp_call(c, method, pattern), docof(c, d), rsp_doc_selected(d), doc(d, path, _), line(c, l), visit_index(c, i), rsp_category(pattern, method, "{category}"), rsp_short(pattern, short), preview = @python_repr(short).
.severity {name} warning
'''
    return shared


def transport_metadata():
    return r'''
.violation RSPConstant(at: span, family: symbol, ordinal: number, pattern: symbol, flags: number) code "RSP_CONSTANT" "constant"
RSPConstant(@place("", 0), family, i, pattern, flags) :- option("entryPoint", "constants"), rsp_constant(family, i, pattern, flags).
.decl rsp_result(i: number, category: symbol, severity: symbol)
rsp_result(i, category, severity) :- option_item("resultCategories", i, category), option_item("resultSeverities", i, severity).
.decl rsp_result_category(i: number, category: symbol)
rsp_result_category(i, category) :- rsp_result(i, category, _), category != "".
rsp_result_category(i, "(uncategorized)") :- rsp_result(i, "", _).
.violation RSPCount(at: span, key: symbol, value: number) code "RSP_COUNT" "{key}"
RSPCount(@place("", 0), "total", n) :- option("entryPoint", "result_counts"), n = count : { rsp_result(_, _, _) }.
RSPCount(@place("", 0), "error", n) :- option("entryPoint", "result_counts"), n = count : { rsp_result(_, _, "error") }.
RSPCount(@place("", 0), "warning", n) :- option("entryPoint", "result_counts"), n = count : { rsp_result(_, _, "warning") }.
.violation RSPCategoryCount(at: span, key: symbol, value: number, first: number) code "RSP_CATEGORY_COUNT" "{key}"
RSPCategoryCount(@place("", 0), category, n, first) :- option("entryPoint", "result_counts"), rsp_result_category(_, category), n = count : { rsp_result_category(_, category) }, first = min i : { rsp_result_category(i, category) }.
'''
