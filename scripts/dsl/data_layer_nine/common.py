# SPDX-License-Identifier: Apache-2.0
"""Canonical, typed Python relations for batch-nine architecture rules.

CPython supplies syntax facts only. All rule decisions execute in SQLite.
The process protocol is JSON; the published SQL columns are scalar values.
"""
from data_layer_six.common import *
from data_layer_generate_batch_seven import policy
from .source_contracts import constants

FIELDS=('kind','name','alias','module','value','value_kind','preorder','breadth_order','parent','field','index','end','function','op','start','stop','line','context','name_repr','value_repr','annotation','annotation_repr')
NUMERIC={'preorder','breadth_order','depth','parent','index','end','function','class','op_count','start','stop','line'}
ALIASES={'preorder':'id','breadth_order':'bfs','index':'position','end':'last','function':'fn','class':'cls','start':'__start','stop':'__end'}

def base():
    projection=','.join((f"CAST(NULLIF(ast_{f},'') AS INTEGER)" if f in NUMERIC else f'ast_{f}')+' AS '+ALIASES.get(f,f) for f in FIELDS)
    return [
      {'name':'canonical','pythonAst':{'syntax':'cst','roots':"SELECT __path,id FROM cst WHERE kind='module'",'fields':['ast_'+f for f in FIELDS],'expand':'nodes'}},
      S('a','SELECT __path,'+projection+' FROM canonical',('__path','id'),('__path','parent','field'),('__path','kind','name'),('__path','fn')),
      S('calls',"""SELECT c.*,f.kind AS callee_kind,f.name AS callee,f.id AS callee_id,r.id AS receiver,r.kind AS receiver_kind,r.name AS receiver_name
      FROM a c JOIN a f ON f.__path=c.__path AND f.parent=c.id AND f.field='func'
      LEFT JOIN a r ON r.__path=f.__path AND r.parent=f.id AND r.field='value'
      WHERE c.kind='Call'""",('__path','id'),('__path','callee'),('__path','fn')),
      S('functions',"SELECT * FROM a WHERE kind IN ('FunctionDef','AsyncFunctionDef')",('__path','id'),('__path','name')),
      S('methods',"SELECT f.*,c.name AS class_name FROM functions f JOIN a c ON c.__path=f.__path AND c.id=f.parent AND c.kind='ClassDef' WHERE f.field='body'",('__path','parent','name'),('__path','id')),
      S('decorators',"""WITH RECURSIVE d(__path,fn,root,id) AS (
      SELECT __path,parent,id,id FROM a WHERE field='decorator_list'
      UNION ALL SELECT d.__path,d.fn,d.root,n.id FROM d JOIN a n ON n.__path=d.__path AND n.parent=d.id AND n.field='func'
      JOIN a c ON c.__path=d.__path AND c.id=d.id AND c.kind='Call')
      SELECT d.__path,d.fn,d.root,n.id,n.kind,n.name FROM d JOIN a n ON n.__path=d.__path AND n.id=d.id WHERE n.kind IN ('Name','Attribute')""",('__path','fn','name')),
      S('qualified',"""WITH RECURSIVE q(__path,id,name) AS (
      SELECT __path,id,name FROM a WHERE kind='Name'
      UNION ALL SELECT p.__path,p.id,CASE WHEN p.kind='Attribute' THEN q.name||'.'||p.name ELSE q.name END
      FROM q JOIN a child ON child.__path=q.__path AND child.id=q.id
      JOIN a p ON p.__path=child.__path AND p.id=child.parent
      WHERE p.kind='Attribute' AND child.field='value' OR p.kind='Call' AND child.field='func') SELECT * FROM q""",('__path','id')),
      S('body',"""SELECT b.* FROM a b WHERE field='body' AND NOT (b.position=0 AND b.kind='Expr' AND EXISTS(SELECT 1 FROM a v WHERE v.__path=b.__path AND v.parent=b.id AND v.field='value' AND v.kind='Constant' AND v.value_kind='str'))""",('__path','parent','position'),('__path','id')),
      S('bodies',"""SELECT f.__path,f.id,count(b.id) AS body_count,min(b.id) AS first_body,max(b.id) AS last_body FROM functions f LEFT JOIN body b ON b.__path=f.__path AND b.parent=f.id GROUP BY f.__path,f.id""",('__path','id')),
      {'name':'raw_annotation_names','pythonAst':{'syntax':'cst','roots':"SELECT __path,id FROM cst WHERE kind='module'",'fields':['annotation_names'],'expand':'annotations'}},
      S('annotation_names','SELECT __path,CAST(annotation AS INTEGER) AS annotation,name FROM raw_annotation_names',('__path','annotation','name')),
      {'name':'lines','pythonAst':{'syntax':'cst','roots':"SELECT __path,id FROM cst WHERE kind='module'",'fields':['source_lines']}},
    ]

def same(left,right):return f'{left}.__path={right}.__path'
def child(parent,role,alias='v',condition='1'):
    if parent==alias:raise ValueError('Child alias shadows its owner: '+parent)
    return f"EXISTS(SELECT 1 FROM a {alias} WHERE {same(alias,parent)} AND {alias}.parent={parent}.id AND {alias}.field={q(role)} AND ({condition}))"
def desc(parent,table='a',alias='n',condition='1'):
    if parent==alias:raise ValueError('Descendant alias shadows its owner: '+parent)
    return f"EXISTS(SELECT 1 FROM {table} {alias} WHERE {same(alias,parent)} AND {alias}.id BETWEEN {parent}.id AND {parent}.last AND ({condition}))"
def param(alias='p'):
    return f"{alias}.kind='arg'"
def pure(alias='f'):
    return f"EXISTS(SELECT 1 FROM decorators d WHERE d.__path={alias}.__path AND d.fn={alias}.id AND d.name='pure_lazy')"
def named(values,alias='n'):
    return f"{alias}.kind IN ('Name','Attribute') AND {alias}.name IN {literals(values)}"
def row(code,message,source='a n',where='1',node='n',order=None,phase=0,symbol="''",extra='',sort_name="''"):
    return f"SELECT {node}.__path AS path,{node}.__start AS start_byte,{node}.__end AS end_byte,{node}.line,{q(code)} AS issue,{message} AS message,{node}.id||':{code}:'||({order or node+'.id'}) AS identity,{phase} AS phase,{order or node+'.id'} AS ordering,{symbol} AS symbol,{sort_name} AS sort_name{extra} FROM {source} WHERE {where}"

def clause(name,stages,query,locations=None,options=None,partition=True):
    import json,re
    common=base();common=[s for s in common if 'pythonAst' in s]+[s for s in common if 'pythonAst' not in s]
    all_stages=common+stages
    catalog={s['name']:s for s in all_stages};needed=set();pending=[json.dumps(query)]
    while pending:
        text=pending.pop()
        for table in re.findall(r'(?i)\b(?:FROM|JOIN)\s+([a-z_]\w*)',text):
            if table in catalog and table not in needed:
                needed.add(table);pending.append(json.dumps(catalog[table]))
    stages=[s for s in all_stages if s['name'] in needed]
    result={'id':name,'relational':{'inputs':{'gate':E(K('module'),{'path':'file.path'})},
       'syntax':{'name':'cst','gate':'gate','columns':['__path','id','kind','__start','__end'],'excludeKinds':[]},
       'stages':stages,'query':query,'key':['issue','identity'],'code':'issue','message':'message'},
       'violation':{'code':'architecture','message':'Architecture contract violation','anchor':'witness.span'}}
    result['relational']['inputs']['gate']['evidence']['query']['ast']['syntax']='valid'
    result['relational']['requireNonemptyInput']='gate'
    if partition:result['relational']['partitionBy']='file'
    if locations:result['relational']['locations']=locations
    return result

def document(name,clauses,scope=None,options=None):
    p=policy(name,scope or ['**/*.py']);p['description']=p['description'].replace('Candidate port','Exact port')
    if name in {'schema_bootstrap_contracts','bootstrap_lazy_sql','bootstrap_eager_resolve','bootstrap_cache_health','plan_composition_architecture','connection_service_architecture'}:
        error_rule=K('module')
        if name=='bootstrap_cache_health':
            error_rule=clauses[0]['relational']['inputs']['gate']['evidence']['query']['ast']['rule']
        if name=='connection_service_architecture':
            error_rule=A(error_rule,{'$option':'entryPoint','cases':{e:TRUE for e in ['_lint_schema_transaction_facades_source','_lint_sql_services','_lint_backend_cores']},'default':FALSE})
        if name=='bootstrap_engine_invariants':error_rule=A(error_rule,O({'filePath':{'regex':r'bootstrap|schema_transaction\.py$|sql_connection\.py$'}},RX('Bootstrap|bootstrap|schema_transaction|materialized_pipeline|physical_table|reconciliation')))
        clauses.append({'id':'source-syntax','forbid':{'ast':{'language':'python','syntax':'invalid','rule':error_rule}},'violation':{'code':'NATIVE-SYNTAX','message':'Python source cannot be parsed','anchor':'witness.span'}})
    p['clauses']=[{'id':'configuration','define':{'options':{'entryPoint':{'type':'string','default':'all'},'parameters':{'type':'string','default':'{}'},**(options or {})}}},*clauses]
    return p


def source_triggers(item,triggers):
    import re
    rule=item['relational']['inputs']['gate']['evidence']['query']['ast']['rule']
    item['relational']['inputs']['gate']['evidence']['query']['ast']['rule']=A(rule,RX('|'.join(re.escape(t) for t in sorted(triggers))))
    item['relational']['requireNonemptyInput']='gate'
    return item


def project_columns(item):
    """Use SQLite column demand to omit unused canonical producer columns.

    Empty SQL tables model producer schemas during authoring. They do not run
    source checks. The same VDBE analysis already narrows the earlier ports.
    """
    import copy,re
    from data_layer_six.sql_columns import narrow_columns
    plan=item['relational'];analysis=copy.deepcopy(plan)
    analysis['syntax'].setdefault('fields',[])
    for stage in analysis['stages']:
        if 'pythonAst' in stage:
            spec=stage.pop('pythonAst');fields=['annotation','name'] if spec.get('expand')=='annotations' else ['name'] if spec.get('expand')=='annotation_names' else spec['fields']
            stage['query']='SELECT '+','.join('CAST(NULL AS TEXT) AS '+name for name in ['__path','id',*fields])+' WHERE 0'
        elif isinstance(stage.get('query'),dict) and 'template' in stage['query']:
            stage['query']=stage['query']['template'].replace('{}',"'{}'")
    report=narrow_columns(analysis)
    unused=next((set(row['nulled']) for row in report if row['stage']=='canonical'),set())
    if not unused:return
    for stage in plan['stages']:
        if stage['name']=='canonical':
            stage['pythonAst']['fields']=[field for field in stage['pythonAst']['fields'] if field not in unused]
        if stage['name']=='a':
            # Logical padding keeps SELECT-star schemas stable. The unused
            # source columns no longer exist and never cross the process pipe.
            for field in unused:
                if field.startswith('ast_'):stage['query']=re.sub(r'\b'+re.escape(field)+r'\b','NULL',stage['query'])
