"""Ordered category inference from the pinned transaction-pattern linter."""
import json
from data_layer_rr_text import character_class


def inference():
    q=json.dumps
    # Source re.match accepts a prefix; the .rr match constraint matches the whole symbol.
    layer=r'\Al'+character_class(str.isdecimal)+r'{4}_[^\n](?s:.*)'
    text=r'''
.decl tp_infer_request(name: symbol, path: symbol)
tp_infer_request(name, path) :- tp_function(f, name), tp_position(f, d, _, _, _), tp_doc(d, path).
tp_infer_request(name, path) :- option("entryPoint", "_infer_category"), option("functionName", name), option("originalPath", path).
.decl tp_component(path: symbol, i: number, component: symbol)
tp_component(path, i, component) :- tp_infer_request(_, path), match_all("(?<component>[^/]+)", path, i, _, _).
.decl tp_root_candidate(path: symbol, i: number, root: symbol)
tp_root_candidate(path, i, root) :- tp_component(path, i, root), (root = "api" ; root = "impl" ; root = "data_layer" ; root = "_internal").
'''
    text+=f'tp_root_candidate(path, i, root) :- tp_component(path, i, root), match({q(layer)}, root).\n'
    text+=r'''
.decl tp_root(path: symbol, root: symbol)
tp_root(path, root) :- tp_root_candidate(path, i, root), i = min j : { tp_root_candidate(path, j, _) }.
.decl tp_guess(name: symbol, path: symbol, priority: number, category: symbol)
'''
    def emit(priority,cat,condition):
        nonlocal text
        text+=f'tp_guess(name, path, {priority}, {q(cat)}) :- tp_infer_request(name, path), {condition}.\n'
    def contains(values,subject='name',insensitive=False):
        fun='icontains' if insensitive else 'contains'
        return '('+' ; '.join(f'{fun}({q(x)}, {subject})' for x in values)+')'
    ts='(contains("l9900_ts_models", path) ; contains("l0500_ts_models", path))'
    for i,(cat,needles) in enumerate([
        ('BOOTSTRAP',['initialize','ensure','bootstrap','create_log_table']),
        ('BACKGROUND',['ripple','reaper','background','process_']),
        ('CAPABILITY',['table_exists','get_table','list_tables','introspect']),
        ('BULK_OPS',['export_','import_','validate_import']),
        ('BOOTSTRAP',['rbac','permission']),('LEGITIMATE',['schema_transaction','begin_'])]):
        emit(i,cat,ts+', '+contains(needles))
    emit(6,'BACKEND_OPS',ts)
    emit(10,'ADMIN_TOOLS','(contains("/tools/", path) ; contains("_handler", name) ; tp_root(path, "l8000_admin"))')
    emit(20,'VALIDATION','(icontains("validat", name) ; contains("validation", path))')
    emit(30,'BOOTSTRAP','(contains("ensure", name) ; contains("initialize", name) ; icontains("bootstrap", name))')
    emit(40,'BACKGROUND',contains(['stale','reaper','recover','ripple','process_','enqueue_ripple']))
    emit(50,'CAPABILITY','('+contains(['_detect_','get_capabilities','has_native','get_backend_type','run_after_rollback','attach_renewable_capability_runtime'])+' ; tp_root(path, "l6510_credentials"))')
    emit(60,'SCHEMA_OPS',contains(['register_queue','register_table','sync_column','register_trigger']))
    emit(70,'BOOTSTRAP','(tp_root(path, "l3100_schema_introspector") ; tp_root(path, "l3200_metadata") ; tp_root(path, "l3250_migrations"))')
    emit(80,'SUBSCRIPTION','contains("subscribe", name)')
    emit(90,'BULK_OPS','(contains("bulk", path) ; contains("export_database", name) ; contains("import_database", name))')
    emit(100,'BACKEND_SPECIFIC','tp_root(path, "impl"), '+contains(['sqlite','postgres','mariadb','mongodb','neo4j','doltdb'],'path'))
    # Every recognized non-special layer falls through to BACKEND_OPS in the source.
    emit(110,'BACKEND_OPS','tp_root(path, _)')
    text+='tp_guess(name, path, 120, "NEEDS_REVIEW") :- tp_infer_request(name, path).\n'
    return text+r'''
.decl tp_inferred(name: symbol, path: symbol, category: symbol)
tp_inferred(name, path, category) :- tp_guess(name, path, priority, category),
    priority = min p : { tp_guess(name, path, p, _) }.
.violation TPInferred(at: span, category: symbol) code "TP_INFER" "{category}"
TPInferred(@place(path, 0), category) :- option("entryPoint", "_infer_category"), option("functionName", name),
    option("originalPath", path), tp_inferred(name, path, category).
'''
