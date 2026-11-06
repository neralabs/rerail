# SPDX-License-Identifier: Apache-2.0
"""Compile unit contracts into SQL; these programs do not ship as linter rules."""
import copy
import json
import re
from pathlib import Path

from data_layer_ten.common import S, make_clause, make_document, q, row, select_entry
from data_layer_ten.introspection import data_path_predicate

PARAMETERS = S('helper_parameters', {'$option': 'parameters', 'escape': 'sql', 'template': 'SELECT {} data'})
FILES = {'files': {'evidence': {'as': 'f', 'query': {'regex': {'source': 'file.content', 'pattern': r'(?s)\A.*\z'}}, 'project': {'path': 'file.path'}}}}
OWNER = "(SELECT CAST(json_extract(data,'$.owner') AS INTEGER) FROM helper_parameters)"
SYMBOL = "(SELECT json_extract(data,'$.symbol') FROM helper_parameters)"


def prune(plan):
    catalog = {stage['name']: stage for stage in plan['stages']}
    needed = set()
    pending = [json.dumps(plan['query'])]
    while pending:
        for name in re.findall(r'(?i)\b(?:FROM|JOIN)\s+([a-z_]\w*)', pending.pop()):
            if name in catalog and name not in needed:
                needed.add(name)
                pending.append(json.dumps(catalog[name]))
    plan['stages'] = [stage for stage in plan['stages'] if stage['name'] in needed]


def ast_helpers():
    def result(source='a n', where='1', value='n.id'):
        return row('HELPER', q('Native AST helper result'), source, where, extra=',n.id node_id,' + value + ' helper_value')
    mention = "EXISTS(SELECT 1 FROM a owner JOIN a child ON child.__path=owner.__path AND child.id BETWEEN owner.id AND owner.last WHERE owner.__path=n.__path AND owner.id=" + OWNER
    cases = {
        '_non_docstring_stmts': result(where="n.parent=" + OWNER + " AND n.field='body' AND NOT (n.kind='Expr' AND EXISTS(SELECT 1 FROM a value WHERE value.__path=n.__path AND value.parent=n.id AND value.field='value' AND value.kind='Constant' AND value.value_kind='str'))"),
        '_decorator_names': result('a n JOIN single_decorators d ON d.__path=n.__path AND d.root=n.id', 'd.fn=' + OWNER + " AND d.kind IN ('Name','Attribute')", 'd.name') + ' GROUP BY d.name',
        '_find_class': result(where="n.kind='ClassDef' AND n.parent=" + OWNER + " AND n.field='body' AND n.name=" + SYMBOL) + ' ORDER BY n.id LIMIT 1',
        '_find_last_method': result(where="n.kind IN ('FunctionDef','AsyncFunctionDef') AND n.parent=" + OWNER + " AND n.field='body' AND n.name=" + SYMBOL) + ' ORDER BY n.id DESC LIMIT 1',
        '_function_mentions_attr': result(where="n.kind='Module'", value=mention + " AND child.kind='Attribute' AND child.name=" + SYMBOL + ')'),
        '_function_mentions_name': result(where="n.kind='Module'", value=mention + " AND child.kind IN ('Name','Attribute') AND child.name=" + SYMBOL + ')'),
        '_function_mentions_any_name': result(where="n.kind='Module'", value=mention + " AND child.kind IN ('Name','Attribute') AND child.name IN (SELECT value FROM helper_parameters,json_each(data,'$.symbols')))"),
        '_class_has_assignment': result(where="n.kind='Module'", value="EXISTS(SELECT 1 FROM a assignment JOIN a target ON target.__path=assignment.__path AND target.parent=assignment.id AND target.field='targets' AND target.kind='Name' WHERE assignment.__path=n.__path AND assignment.parent=" + OWNER + " AND assignment.field='body' AND assignment.kind='Assign' AND target.name=" + SYMBOL + ')'),
    }
    clause = make_clause('ast-unit-contract', [PARAMETERS], select_entry('SELECT * FROM a WHERE 0', cases))
    return make_document('helper-ast', [clause], ['**/*.py'], exclude=[])


def file_program(name, query, stages):
    clause = {'id': name, 'relational': {'inputs': copy.deepcopy(FILES), 'stages': stages, 'query': query, 'key': ['identity']},
              'violation': {'code': 'HELPER', 'message': 'Native unit contract result', 'anchor': 'witness.span'}}
    return make_document(name, [clause], ['**/*'], exclude=[])


def scopes(source):
    from data_layer_nine.source_contracts import constants
    constants_ = constants(source / 'scripts/lint_schema_introspection_data_paths.py')
    predicate = data_path_predicate(constants_, "json_extract(data,'$.path')")
    base = "SELECT files.__path path,0 start_byte,0 end_byte,0 identity,"
    return file_program('helper-path', base + predicate + ' helper_value FROM files,helper_parameters', [PARAMETERS])


def approvals():
    path = "rx_extract(entry,'\\A([^:]*)',1)"
    entries = S('approval_entries', "SELECT CAST(key AS INTEGER) ordinal,value entry FROM helper_parameters,json_each(data,'$.entries')")
    messages = [("substr(" + path + ",1,5)='impl/'", ': approved schema checks must not be internal impl callsites'),
                ("substr(" + path + ",1,8)='scripts/' OR instr(" + path + ",'/tests/')>0", ': approved schema checks must be production user-facing callsites')]
    queries = ["SELECT files.__path path,0 start_byte,0 end_byte,ordinal*2+" + str(index) + " identity,entry||" + q(message) + ' helper_value FROM files,approval_entries WHERE ' + condition for index, (condition, message) in enumerate(messages)]
    return file_program('helper-approvals', ' UNION ALL '.join(queries), [PARAMETERS, entries])


def parsing():
    syntax = {'name': 'syntax', 'pythonAst': {'source': 'files', 'roots': 'SELECT __path,source_text(__path) source FROM files', 'fields': ['syntax_error']}}
    return file_program('helper-parse', 'SELECT __path path,0 start_byte,length(CAST(source_text(__path) AS BLOB)) end_byte,0 identity,syntax_error helper_value FROM syntax', [syntax])


def matching(source):
    from data_layer_ten.matching import candidate
    production = candidate(source)
    clause = copy.deepcopy(next(c for c in production['clauses'] if c['id'] == 'matching-file-checks'))
    plan = clause['relational']
    plan['query'] = row('HELPER', q('Native matching contract use'), 'matching_context n', "n.kind='Module'", extra=',n.uses_contracts helper_value')
    for stage in plan['stages']:
        if stage['name'] == 'canonical':
            stage['pythonAst']['roots'] = 'SELECT __path,source_text(__path) source FROM files'
    prune(plan)
    clause['violation'] = {'code': 'HELPER', 'message': 'Native matching contract use', 'anchor': 'witness.span'}
    return make_document('helper-matching', [clause], ['**/*.py'], exclude=[])


def generate(source, output):
    documents = {'batch-ten-helper-ast': ast_helpers(), 'batch-ten-helper-path': scopes(source),
                 'batch-ten-helper-approvals': approvals(), 'batch-ten-helper-parse': parsing(),
                 'batch-ten-helper-matching': matching(source)}
    output.write_text(json.dumps(documents, indent=2) + '\n')


if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser()
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    generate(args.source, args.output)
