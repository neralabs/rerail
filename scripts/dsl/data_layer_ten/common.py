# SPDX-License-Identifier: Apache-2.0
"""Shared canonical relations for the tenth batch. SQLite makes rule decisions."""
from data_layer_nine.common import *
from data_layer_nine.evidence import PIN
from data_layer_nine.common import base as nine_base

CHECKS=('graph_lazy_sql_plan','bounded_composed_graph_paths','graphql_plan_ownership','plan_optimizer_v2','conditional_architecture','schema_introspection_data_paths','matching_architecture','hnsw_architecture','embedding_vector_orchestration','generated_index_sources')
EXCLUDES=['**/'+part+'/**' for part in ['.git','.venv','venv','__pycache__','tmp','tests','test']]
def python_character_class(matches):
    import unicodedata
    if unicodedata.unidata_version!='15.1.0':raise ValueError('Rule generation requires CPython 3.13 Unicode 15.1')
    ranges=[];start=last=None
    for point in range(0x110000):
        if matches(chr(point)):
            if start is None:start=point
            last=point
        elif start is not None:ranges.append((start,last));start=None
    if start is not None:ranges.append((start,last))
    return '['+''.join(chr(a) if a==b else chr(a)+'-'+chr(b) for a,b in ranges)+']'

PY_WORD=python_character_class(lambda c:c=='_' or c.isalnum())
PY_SPACE=python_character_class(str.isspace)
PY_NONWORD='[^'+PY_WORD[1:]
PY_LEFT=r'(?:\A|'+PY_NONWORD+')'
PY_RIGHT=r'(?:\z|'+PY_NONWORD+')'

def python_word(pattern,prefix=False):
    # Use CPython's character set, independently of the Rust Unicode version.
    # A prefix capture includes the consumed delimiter for correct line numbers.
    return (r'(?s)\A(.*?'+PY_LEFT+r')' if prefix else PY_LEFT)+'(?:'+pattern+')'+PY_RIGHT

def python_upper_prefix(words):
    """Preimage of uppercase ASCII prefixes under CPython's full upper mapping.

    A final character may expand beyond the requested prefix, as with ẗ -> T̈.
    Keep Unicode case operations out of SQLite's ASCII-only upper function.
    """
    import re
    from functools import lru_cache
    assert all(word and word.isascii() and word==word.upper() for word in words)
    images={}
    for point in range(0x110000):
        char=chr(point);upper=char.upper()
        if upper[0].isascii():images.setdefault(upper,[]).append(char)
    @lru_cache(None)
    def suffix(remaining):
        arms={}
        for image,chars in images.items():
            if image.startswith(remaining):tail=''
            elif remaining.startswith(image):tail=suffix(remaining[len(image):])
            else:continue
            arms.setdefault(tail,[]).extend(chars)
        return '(?:'+'|'.join('['+''.join(re.escape(c) for c in sorted(chars))+']'+tail for tail,chars in arms.items())+')'
    return '(?:'+'|'.join(suffix(word) for word in words)+')'

def python_decimal_patterns():
    """The decimal digits accepted by CPython regex and int, with their values."""
    digits=[[] for _ in range(10)]
    for point in range(0x110000):
        char=chr(point)
        if char.isdecimal():digits[int(char)].append(char)
    return python_character_class(str.isdecimal),['['+''.join(chars)+']' for chars in digits]


def extended():
    stages=nine_base()
    for stage in stages:
        if stage['name']=='canonical':stage['pythonAst']['fields'].extend(['ast_class','ast_level'])
        if stage['name']=='a':stage['query']=stage['query'].replace(' FROM canonical',',CAST(NULLIF(ast_class,\'\') AS INTEGER) AS cls,CAST(NULLIF(ast_level,\'\') AS INTEGER) AS level FROM canonical')
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
