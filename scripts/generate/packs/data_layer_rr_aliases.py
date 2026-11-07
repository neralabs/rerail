"""Generate the module-level facade alias policy from pinned source constants."""
import json


def data_layer_compat_aliases(source, target, header):
    from data_layer_rr_author import constants
    messages = constants(source / 'scripts/lint_data_layer_compat_aliases.py')['VIOLATIONS']
    text = header('data_layer_compat_aliases', target)
    text += '''
// Function and class scopes suppress aliases, including their decorators and signatures.
.decl alias_scope(n: node)
alias_scope(n) :- kind(f, "function_definition"), (within(n, f) ; decorator(f, n) ; decorator(f, d), within(n, d)).
alias_scope(n) :- kind(c, "class_definition"), (within(n, c) ; decorator(c, n) ; decorator(c, d), within(n, d)).
.decl alias_selected(d: number)
alias_selected(d) :- doc(d, _, _), option("entryPoint", _).
alias_selected(d) :- doc(d, p, _), !option("entryPoint", _), (prefix("data_layer/", p) ; p = "testing.py"),
    !alias_hidden(d), !path_part(d, "test"), !path_part(d, "tests").
.decl alias_hidden(d: number)
alias_hidden(d) :- path_part(d, ".git").
alias_hidden(d) :- path_part(d, ".venv").
alias_hidden(d) :- path_part(d, "venv").
alias_hidden(d) :- path_part(d, "__pycache__").
alias_hidden(d) :- path_part(d, "tmp").
.decl alias_leaf(e: node, x: symbol)
alias_leaf(e, @nfkc(raw)) :- unparen(e, n), kind(n, "identifier"), text(n, raw).
alias_leaf(e, @nfkc(raw)) :- unparen(e, n), kind(n, "attribute"), field(n, "attribute", a), text(a, raw).
.decl alias_import(a: node, s: node, name: symbol, original: symbol)
alias_import(a, s, @nfkc(name), @nfkc(original)) :- kind(s, "import_from_statement"), field(s, "name", a),
    kind(a, "aliased_import"), field(a, "alias", nm), text(nm, name), field(a, "name", v), text(v, original).
alias_import(a, s, @nfkc(name), @nfkc(original)) :- kind(s, "import_statement"), field(s, "name", a),
    kind(a, "aliased_import"), field(a, "alias", nm), text(nm, name), field(a, "name", v),
    child(v, leaf, i), !child(v, _, i + 1), text(leaf, original).
.decl alias_assignment(t: node, s: node, name: symbol, original: symbol)
alias_assignment(t, s, @nfkc(name), original) :- asgstmt(s, _), assign_target(s, target), unparen(target, t),
    kind(t, "identifier"), text(t, name), assign_value(s, value), alias_leaf(value, original).
'''
    for code, rel in [('DLA001', 'alias_import'), ('DLA002', 'alias_assignment')]:
        clause = 'ImportAlias' if code == 'DLA001' else 'AssignmentAlias'
        text += f'''
.violation {clause}(at: span, witness: node) code "{code}" {json.dumps(messages[code])}
{clause}(@place(p, l), a) :- {rel}(a, s, name, original), !prefix("_", name), name != original,
    !alias_scope(s), docof(s, d), alias_selected(d), doc(d, p, _), line(s, l), !syntax_error(d, _, _).
'''
    text += '''
// Helper inputs are typed DTO fields from the caller, not Python execution.
.violation PublicName(at: span) code "ALIAS_PUBLIC" "public name"
PublicName(@place("input.py", 1)) :- option("entryPoint", "_is_public_name"), option("name", name), !prefix("_", name).
.violation LeafName(at: span, result: symbol) code "ALIAS_LEAF" "{result}"
LeafName(@place("input.py", 1), name) :- option("entryPoint", "_leaf_name"), option("nodeKind", kind),
    (kind = "Name" ; kind = "Attribute"), option("nodeName", name).
'''
    return text
