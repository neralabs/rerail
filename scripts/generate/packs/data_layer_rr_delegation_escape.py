"""Escape-hatch events in AST visit order, with whole-file line deduplication."""
import ast
import json


def escape_relations(source):
    module=ast.parse((source/'scripts/lint_async_sync_delegation.py').read_text())
    values={n.targets[0].id:ast.literal_eval(n.value) for n in module.body
            if isinstance(n,ast.Assign) and len(n.targets)==1 and isinstance(n.targets[0],ast.Name)
            and n.targets[0].id=='ESCAPE_HATCH_PATTERNS'}
    for n in module.body:
        if isinstance(n,ast.AnnAssign) and isinstance(n.target,ast.Name) and n.target.id=='ESCAPE_HATCH_FUNCTION_ALLOWLIST':
            values[n.target.id]=ast.literal_eval(n.value)
    text='\n.decl asde_pattern(name: symbol)\n'
    for name in values['ESCAPE_HATCH_PATTERNS']:text+='asde_pattern('+json.dumps(name)+').\n'
    text+='\n.decl asde_allow_function(path: symbol, name: symbol)\n'
    for path,name in sorted(values['ESCAPE_HATCH_FUNCTION_ALLOWLIST']):text+='asde_allow_function('+json.dumps(path)+','+json.dumps(name)+').\n'
    return text+r'''
.decl asde_name(call: node, name: symbol)
asde_name(call, @nfkc(name)) :- kind(call, "call"), field(call, "function", raw), unparen(raw, fn), kind(fn, "identifier"), text(fn, name).
asde_name(call, cat(@nfkc(receiver), ".", @nfkc(name))) :- kind(call, "call"), field(call, "function", raw), unparen(raw, fn), kind(fn, "attribute"), field(fn, "object", rawobj), unparen(rawobj, obj), kind(obj, "identifier"), text(obj, receiver), field(fn, "attribute", nm), text(nm, name).
asde_name(call, @nfkc(name)) :- kind(call, "call"), field(call, "function", raw), unparen(raw, fn), kind(fn, "attribute"), field(fn, "object", rawobj), unparen(rawobj, obj), !kind(obj, "identifier"), field(fn, "attribute", nm), text(nm, name).
.decl asde_call(call: node, name: symbol)
asde_call(call, name) :- asde_name(call, name), asde_pattern(pattern), (name = pattern ; suffix(cat(".", pattern), name)).
.decl asde_owner(n: node, name: symbol)
asde_owner(n, name) :- kind(n, k), (k = "call" ; k = "await"), def_frame(n, f), asdp_function(f, _, name).
asde_owner(n, "<module>") :- kind(n, k), (k = "call" ; k = "await"), !def_frame(n, _).
.decl asde_allowed(n: node)
asde_allowed(n) :- def_frame(n, f), asdp_function(f, path, name), asde_allow_function(tail, name), suffix(tail, path).
.decl asde_event(n: node, d: number, path: symbol, line: number, function_name: symbol, description: symbol, ordinal: number)
asde_event(n, d, path, line, owner, cat("Escape hatch: ", name, "()"), i) :- asde_call(n, name), docof(n, d), asdp_path(d, path), !syntax_error(d, _, _), !asde_allowed(n), asde_owner(n, owner), line(n, line), visit_index(n, i).
asde_event(n, d, path, line, owner, cat("Escape hatch: await ", name, "()"), i) :- kind(n, "await"), child(n, raw, 0), unparen(raw, call), asde_call(call, name), docof(n, d), asdp_path(d, path), !syntax_error(d, _, _), !asde_allowed(n), asde_owner(n, owner), line(n, line), visit_index(n, i).
ASDAnalysis(@place(path, line), owner, "C", description, "", "Replace with native async driver call", 0, 1, 5, i) :- asde_event(_, d, path, line, owner, description, i), i = min j : { asde_event(_, d, _, line, _, _, j) }.
'''
