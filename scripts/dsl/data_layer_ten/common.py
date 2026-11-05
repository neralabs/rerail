# SPDX-License-Identifier: Apache-2.0
"""Shared canonical relations for the tenth batch. SQLite makes rule decisions."""
from data_layer_nine.common import *
from data_layer_nine.evidence import PIN
from data_layer_nine.common import base as nine_base

CHECKS=('graph_lazy_sql_plan','bounded_composed_graph_paths','graphql_plan_ownership','plan_optimizer_v2','conditional_architecture','schema_introspection_data_paths','matching_architecture','hnsw_architecture','embedding_vector_orchestration','generated_index_sources')
EXCLUDES=['**/'+part+'/**' for part in ['.git','.venv','venv','__pycache__','tmp','tests','test']]


def extended():
    stages=nine_base()
    for stage in stages:
        if stage['name']=='canonical':stage['pythonAst']['fields'].append('ast_class')
        if stage['name']=='a':stage['query']=stage['query'].replace(' FROM canonical',',CAST(NULLIF(ast_class,\'\') AS INTEGER) AS cls FROM canonical')
    stages += [S('classes',"SELECT * FROM a WHERE kind='ClassDef'",('__path','id')),
      S('single_decorators',"SELECT d.__path,d.parent fn,d.id root,coalesce(c.kind,d.kind) kind,coalesce(c.name,d.name) name FROM a d LEFT JOIN a c ON c.__path=d.__path AND c.parent=d.id AND c.field='func' AND d.kind='Call' WHERE d.field='decorator_list'",('__path','fn','name')),
      S('qual_functions',"SELECT f.*,(SELECT group_concat(name,'.')||'.' FROM (SELECT name FROM classes c WHERE c.__path=f.__path AND f.id BETWEEN c.id AND c.last ORDER BY c.id))||f.name qualified FROM functions f",('__path','id')),
      S('expression_roots',"WITH RECURSIVE r(__path,id,name) AS (SELECT __path,id,name FROM a WHERE kind='Name' UNION ALL SELECT p.__path,p.id,r.name FROM r JOIN a child ON child.__path=r.__path AND child.id=r.id JOIN a p ON p.__path=child.__path AND p.id=child.parent WHERE p.kind='Attribute' AND child.field='value' OR p.kind='Call' AND child.field='func') SELECT * FROM r",('__path','id'))]
    return stages


def make_clause(name,stages,query,partition=True,locations=None):
    import json,re
    available=extended()+stages;catalog={s['name']:s for s in available};needed=set();pending=[json.dumps(query)]
    while pending:
        for table in re.findall(r'(?i)\b(?:FROM|JOIN)\s+([a-z_]\w*)',pending.pop()):
            if table in catalog and table not in needed:needed.add(table);pending.append(json.dumps(catalog[table]))
    item=clause(name,[],query,partition=partition,locations=locations)
    item['relational']['stages']=[s for s in available if s['name'] in needed]
    return item


def make_document(name,clauses,scope,exclude=EXCLUDES):
    p=policy(name,scope);p['description']=f'Port of nera-data-layer {name} at {PIN}.'
    p['scope']['exclude']=list(exclude)
    p['clauses']=[{'id':'configuration','define':{'options':{'entryPoint':{'type':'string','default':'all'},'parameters':{'type':'string','default':'{}'}}}},*clauses]
    return p


def select_entry(query,cases):
    return {'$option':'entryPoint','cases':cases,'default':query}


def triggered(item,triggers):return source_triggers(item,triggers)
