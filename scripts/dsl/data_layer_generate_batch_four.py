#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Generate fourth-batch native candidates from the pinned upstream constants."""
from __future__ import annotations
import argparse
import importlib
import json
from pathlib import Path
import re
import sys

PIN = '2b1504adc4291daaf080b2e932962b134da1a69a'
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--source', type=Path, required=True)
p.add_argument('--repo', type=Path, required=True)
p.add_argument('--output', type=Path, default=Path('packs/data-layer/candidates/rules'))
p.add_argument('--signature-output', type=Path, help='Write the pinned public verification material for the host adapter')
args = p.parse_args()
sys.path.insert(0, str(args.source.resolve()))
from data_layer_batch_four import BATCH_FOUR
from data_layer_probe_record import verify_sources
modules = {name: importlib.import_module('scripts.lint_' + name) for name in BATCH_FOUR}
verify_sources(args.source.resolve(), args.repo, PIN)
args.output.mkdir(parents=True, exist_ok=True)

def A(*r): return {'all': list(r)}
def O(*r): return {'any': list(r)}
def N(r): return {'not': r}
def K(k): return {'kind': k}
def RX(p): return {'regex': p}
def P(c): return {'pattern': '$' + c}
def H(r, field=None, stop='neighbor'): return {'has': {**r, 'stopBy': stop, **({'field': field} if field else {})}}
def I(r, stop='end', field=None): return {'inside': {**r, 'stopBy': stop, **({'field': field} if field else {})}}
def F(f, r): return H(r, f)
def M(n, **kw):
 if n=='build-string' and not kw: kw={'DUMMY':'UNBOUND'}
 return {'matches': {n: {k: '$'+v for k,v in kw.items()}}} if kw else {'matches': n}
def names(v, kind='identifier'): return A(K(kind), RX('^(?:'+'|'.join(re.escape(x) for x in sorted(v))+')$' if v else r'\b\B'))
def opt(n): return {'$option': n, 'escape': 'regex', 'join': '|', 'template': '^(?:{})$'}
def onames(n): return A(K('identifier'), RX(opt(n)))
def term(v): return O(names(v), A(K('attribute'), F('attribute', names(v))))
def fn(n=None): return A(K('function_definition'), F('name', names([n]))) if n else K('function_definition')
def sync(n=None): return A(fn(n), RX(r'^def\s'))
def direct_method(r): return F('body', H(O(r, A(K('decorated_definition'), F('definition', r)))))
def C(id, r, code, anchor='witness.span', each=None, when=None, file=None):
 q={'language':'python','syntax':'valid','rule':r}
 if when is not None: q['when']={'option':when}
 if file:q['file']={'path':file}
 return {'id':id,'forbid':{'ast':q},'violation':{'code':code,'anchor':anchor,'message':code+': '+id,**({'each':each} if each else {})}}
def R(id, pattern, code, paths=None, **kw):
 q={'source':'file.content','pattern':pattern,'lines':{'first':True},**kw}
 if paths:q['paths']=paths
 return {'id':id,'forbid':{'regex':q},'violation':{'code':code,'anchor':'witness.span','message':code+': '+id}}
def G(id, rule, project, stages, predicate, code, each='ROW', groups=None, anchor='witness.span'):
 return {'id':id, 'budget':{'query':{'evidence':{'as':'row', 'query':{'ast':{'language':'python','syntax':'valid','rule':rule}},
  'project':{'path':'file.path',**project},'each':each,'anchor':anchor,'closure':stages,'where':predicate}},
  'count':{'lte':0},'group_by':groups or ['row.path','row.position']},
  'violation':{'code':code,'anchor':'witness.span','message':code+': '+id}}
def astquery(clause):
 return clause.get('forbid',{}).get('ast') or clause.get('budget',{}).get('query',{}).get('evidence',{}).get('query',{}).get('ast')
def W(name,clauses,utils=None,options=None,roots=None,exclude=None,mode='block'):
 options={'oracleAnalyzer':{'type':'boolean','default':False},'sourceGate':{'type':'boolean','default':True},**(options or {})}
 definition={'options':options}
 if utils:definition.update(language='python',utils=utils)
 if exclude is None and name not in ('render_fstrings','render_raw_strings'):
  exclude=['**/.git/**','**/.venv/**','**/venv/**','**/__pycache__/**','**/tmp/**','**/tests/**','**/test/**']
 d={'format':'rerail.policy/v1','id':'data-layer-'+name.replace('_','-'),'revision':1,'mode':mode,
    'description':'Candidate port of nera-data-layer '+name+' at '+PIN+'.',
    'scope':{'include':roots or ['impl/common/l2100_sql_grammar/**/*.py'],
             'exclude': ['**/.git/**','**/.venv/**','**/venv/**','**/__pycache__/**','**/tmp/**'] if exclude is None else exclude},
    'clauses':[{'id':'shared','define':definition},*clauses]}
 if name=='sql_grammar_escape_hatches':
  if not modules[name]._verify_allowlist_integrity():
   raise ValueError('Escape-hatch allowlist signature verification failed')
  definition['report_when']={'count':{'gt':modules[name].MAX_ESCAPE_HATCH_VIOLATIONS}}


 if name in ('grammar_render_methods','sql_grammar_escape_hatches'):
  for clause in clauses:
   query=clause.get('forbid',{}).get('ast')
   if query is None: continue
   if name=='sql_grammar_escape_hatches' and clause['id']=='forbidden-class':
    query['rule']=A(query['rule'],N({'layout':{'line':r'(?:'+ '|'.join(re.escape(x) for x in sorted(modules[name].FORBIDDEN_ESCAPE_HATCHES))+r')\(|import (?:'+ '|'.join(re.escape(x) for x in sorted(modules[name].FORBIDDEN_ESCAPE_HATCHES))+r')|, (?:'+ '|'.join(re.escape(x) for x in sorted(modules[name].FORBIDDEN_ESCAPE_HATCHES))+r')'}}))
    continue
   base=query['rule']; key='earlier-'+clause['id']
   if name=='grammar_render_methods' and clause['id']=='fstring':
    # Compare the opening token, whose range precedes the format spec, to avoid overlapping spans.
    opening=H(A(K('string_start'),{'layout':{'after':{'to':'capture.FS_SELF','breaks':{'eq':0}}}}))
    parent_start=O(A(K('string'),N(I(K('concatenated_string'),'neighbor')),opening),
     A(K('concatenated_string'),H(A(K('string'),{'nthChild':{'position':1,'ofRule':K('string')}},opening))))
    base=A(base,P('FS_SELF'),N(A(K('format_specifier'),I(parent_start))))
   definition.setdefault('language','python')
   definition.setdefault('utils',{})[key]={'params':['CURRENT'], 'rule': I(A(K('module'), H(A(base,{'layout':{'after':{'to':'capture.CURRENT','breaks':{'eq':0}}}}),stop='end')))}
   query['rule']=A(base,P('CURRENT'),N(M(key,CURRENT='CURRENT')))
 # Fact materialization skips files without any source trigger.
 specs=[v for v in vars(modules[name]).values() if getattr(v,'extractor_id','')==name+'.violations']
 triggers=next((getattr(v,'source_triggers',()) for v in specs),())
 if triggers:
  gate={'$option':'sourceGate','cases':{'true':'|'.join(re.escape(x) for x in triggers),'false':r'(?s).*'}}
  for clause in clauses:
   q=astquery(clause)
   if q is not None:q['rule']=A(q['rule'],I(A(K('module'),RX(gate))))
 (args.output/(d['id']+'.json')).write_text(json.dumps(d,indent=2)+'\n')
def arrays(module, *keys):return {key:{'type':'array','default':sorted(getattr(module,key))} for key in keys}
FPREFIX=A(K('string'),RX(r'(?i)^[rub]*f'))
STR=O(A(K('string'),N(RX(r'(?i)^[rub]*f'))),A(K('concatenated_string'),N(H(FPREFIX))))
FS=O(A(FPREFIX,N(I(K('concatenated_string'),'neighbor'))),A(K('concatenated_string'),H(FPREFIX)),K('format_specifier'))
# A Python Constant has one node for implicit adjacent literals.
CONST=A(STR,N(I(K('concatenated_string'),'neighbor')),{'value':{'type':'str'}})
RENDER=I(A(fn(), F('name', RX(r'^render(?:_|$)'))))
W('render_fstrings',[C('fstring',A(FS,RENDER),'render_fstring')])
# Raw strings: special calls report at the call and suppress only their first constant argument.
special=O(names(['Template']), A(K('attribute'), F('object',names(['SqlFragment'])),F('attribute',names(['static']))))
firstconst={'arguments':{'signature':['arg','/'],'bind':{'ARG':'arg'},'mode':'syntactic'}}
specialcall=A(K('call'), F('function',special),firstconst,H(A(CONST,P('ARG')),field='arguments',stop='end'))
allowed=['format','str','repr','SqlAliasName','SqlTableName','SqlColumnName','SqlIndexName','SqlQuotedIdentifier','SqlQualifiedIdentifier','SqlIntegerLiteral','append','join','replace','startswith','endswith']
allowedarg=O(I(A(K('argument_list'),I(A(K('call'),F('function',term(allowed))),'neighbor')),'neighbor'),
 A(K('comment'),RX(r'\b\B')))
# Only the first statement of the outer active render function is its docstring.
doc=A(I(A(K('expression_statement'),{'nthChild':{'position':1,'ofRule':N(K('comment'))}},I(A(K('block'),I(A(fn(),F('name',RX(r'^render(?:_|$)')),N(I(A(fn(),F('name',RX(r'^render(?:_|$)')))))),'neighbor')),'neighbor')),'neighbor'))
suppress_special=I(A(K('argument_list'),I(A(specialcall,{'arguments':{'signature':['arg','/'],'bind':{'FIRST':'arg'},'mode':'syntactic'}}),'neighbor')),'neighbor')
raw=A(CONST,{'value':{'regex':r'(?s)^.{2}'}},RENDER,N(doc),N(I(O(K('raise_statement'),K('assert_statement')))),N(allowedarg),
 N(A(P('CURRENT'),suppress_special,{'compare':{'left':'CURRENT','right':'FIRST','op':'eq','text':'node'}})))
raw_fstring=A(FS,RENDER,N(I(O(K('raise_statement'),K('assert_statement')))),
 {'parse':{'grammar':'pythonFString','has':{'kind':'literal','value':{'regex':r'(?s)^.{2}'},'capture':'PART'}}})
W('render_raw_strings',[C('literal',raw,'string_literal'),C('fstring-literal',raw_fstring,'string_literal',anchor='capture.PART',each='PART'),C('template',A(specialcall,F('function',names(['Template'])),RENDER),'template_call'),
 C('static',A(specialcall,F('function',K('attribute')),RENDER),'static_call')])
# Rendering method restrictions use the nearest class's bases when render() is entered.
m=modules['grammar_render_methods']
utils={'base-sql':O(A(term([]),RX('Sql')),A(K('identifier'),RX('Sql')),A(K('attribute'),F('attribute',RX('Sql'))),A(K('subscript'),F('value',M('base-sql'))))}
grammar=A(K('class_definition'),F('superclasses',H(M('base-sql'))),N(F('name',onames('LEGACY_ALLOWLISTED_CLASSES'))))
context=I(A(fn('render'),I(grammar,K('class_definition'))))
clauses=[C('fstring',A(FS,context,N(I(A(K('call'),F('function',names(['GrammarException'])))))),'F_STRING'),
 C('join',A(K('call'),F('function',A(K('attribute'),F('attribute',names(['join'])),F('object',O(CONST,K('integer'),K('float'),K('true'),K('false'),K('none'),K('ellipsis'))))),context),'STRING_JOIN'),
 C('format',A(K('call'),F('function',A(K('attribute'),F('attribute',names(['format'])))),context),'STRING_FORMAT'),
 C('template',A(K('call'),F('function',names(['t','Template'])),context),'T_STRING'),
 C('concat',A(K('binary_operator'),F('operator',RX(r'^\+$')),F('left',O(CONST,FS)),F('right',O(CONST,FS)),context),'STRING_CONCAT')]
# Enumerate runtime positional args; keyword args are outside the upstream check.
arg=A(N(O(K('comment'),K('keyword_argument'),K('dictionary_splat'))),N(CONST),P('ARG'))
staticfunc=O(names(['static']),A(K('attribute'),F('attribute',names(['static'])),F('object',names(['SqlFragment','SF']))),A(K('attribute'),F('attribute',names(['static'])),F('object',K('attribute'))))
clauses.append(C('runtime-static',A(K('call'),F('function',staticfunc),context,
 I(A(K('class_definition'),N(F('name',onames('ALLOWLISTED_STATIC_CLASSES')))),K('class_definition')),F('arguments',H(arg))),'STATIC_RUNTIME_VALUE'))
W('grammar_render_methods',clauses,utils,arrays(m,'ALLOWLISTED_STATIC_CLASSES','LEGACY_ALLOWLISTED_CLASSES'))
# Bare-str parameters and their synthetic _parameter slots.
m=modules['grammar_str_params'];utils={}
utils['str-annotation']=O(names(['str']),A(K('binary_operator'),F('operator',RX(r'^\|$')),O(F('left',M('str-annotation')),F('right',M('str-annotation')))),A(K('union_type'),H(M('str-annotation'))),A(K('type'),H(M('str-annotation'))),A(K('parenthesized_expression'),H(M('str-annotation'))))
grammar=A(K('class_definition'),N(F('name',onames('ALLOWLISTED_STR_CLASSES'))),O(F('superclasses',H(O(A(K('identifier'),RX(r'^(?:Sql|_Sql)')),A(K('attribute'),F('attribute',RX(r'^(?:Sql|_Sql)')))))),direct_method(sync('render'))))
param=A(K('typed_parameter'),H(A(K('identifier'),N(names(['self'])),P('PARAM'))),F('type',M('str-annotation')))
# Positional-only and keyword-only constructor parameters are intentionally omitted by this oracle.
normal=N(O(K('list_splat_pattern'),K('dictionary_splat_pattern'),A(K('typed_parameter'),H(O(K('list_splat_pattern'),K('dictionary_splat_pattern'))))))
param=A(O(param,A(K('typed_default_parameter'),F('name',A(K('identifier'),N(names(['self'])),P('PARAM'))),F('type',M('str-annotation')))),normal,N({'follows':{**O(K('positional_separator'),K('keyword_separator'),K('list_splat_pattern'),A(K('typed_parameter'),H(K('list_splat_pattern')))),'stopBy':'end'}}))
# args.args excludes nodes before '/' too.
param=A(param,N({'precedes':{**K('positional_separator'),'stopBy':'end'}}))
allowedpairs=[A(F('name',names([cl])),direct_method(A(sync('__init__'),F('parameters',H(A(param,H(names([pa])))))))) for cl,pa in sorted(m.ALLOWLISTED_STR_PARAMS)]
# Check each exception on the captured parameter, rather than exempting the whole class.
pair_exclusion=O(*[A(H(names([pa])),I(A(sync('__init__'),I(A(K('class_definition'),F('name',names([cl]))),K('class_definition'))))) for cl,pa in sorted(m.ALLOWLISTED_STR_PARAMS)])
utils['str-param']=A(param,N(pair_exclusion))
param_context=I(A(sync('__init__'),I(grammar,K('class_definition'))),K('function_definition'))
clauses=[C('parameter',A(M('str-param'),param_context),'STR_PARAM')]
# Compare self._slot against the constructor name through an underscore-stripping transform.
slot=A(K('attribute'),F('object',names(['self'])),F('attribute',A(K('identifier'),P('SLOT'))))
initparam=A(sync('__init__'),F('parameters',H(A(M('str-param'),{'compare':{'left':'PARAM','right':'SLOT_NAME','op':'eq'}}))))
slotclass=A(grammar,O(N(direct_method(sync('__init__'))),direct_method(initparam)))
strcall=A(K('call'),F('function',names(['str'])),F('arguments',H(slot)),I(A(sync('render'),I(slotclass,K('class_definition')))))
# The transform is evaluated on demand in compare, and uses the capture's source.
for id,rule,code in [('str-call',strcall,'STR_CALL_IN_RENDER'),('str-concat',A(K('binary_operator'),F('operator',RX(r'^\+$')),O(F('left',strcall),F('right',strcall))),'STR_CONCAT_IN_RENDER')]:
 c=C(id,rule,code,each='SLOT');c['forbid']['ast']['transform']={'SLOT_NAME':{'replace':{'source':'$SLOT','replace':r'^_','by':''}}};clauses.append(c)
W('grammar_str_params',clauses,utils,arrays(m,'ALLOWLISTED_STR_CLASSES'))
# Rust boundary is deliberately line based, including comments and string literals.
m=modules['rust_sql_grammar_boundary']
W('rust_sql_grammar_boundary',[R('pattern-'+str(i),p.pattern,'rust_sql_grammar_boundary') for i,(p,_) in enumerate(m.FORBIDDEN_PATTERNS)],roots=['rust/nera_compiler_sql/src/**/*.rs'],exclude=[])
# Zero maximums make nonempty literal sets violations. Keep the reporter's variable grouping.
m=modules['grammar_allowlist_burndown'];utils={}
utils['set-string']=O(A(K('set'),H(CONST)),A(K('binary_operator'),F('operator',RX(r'^\|$')),O(F('left',M('set-string')),F('right',M('set-string')))))
value=O(M('set-string'),A(K('call'),F('arguments',H(A(M('set-string'),{'nthChild':{'position':1,'ofRule':N(K('comment'))}})))))
clauses=[]
for var,(_,maximum) in m._allowlist_specs().items():
 assert maximum==0
 # Zero maxima require one finding for a nonempty variable, irrespective of its entry count.
 query={'ast':{'language':'python','syntax':'valid','rule':A(K('module'),H(A(K('assignment'),F('left',names([var])),F('right',value)),stop='end'))}}
 relation={'evidence':{'as':'usage','query':query,'project':{'path':'file.path'}}}
 clauses.append({'id':'allowlist-'+var.lower(),'budget':{'query':relation,'count':{'lte':0}},
   'violation':{'code':'allowlist_grew','anchor':'witness.span','message':'Allowlist '+var+' is nonempty'}})
W('grammar_allowlist_burndown',clauses,utils,roots=['scripts/**/*.py'])
# SQL regex classification is ordered. Decode the regex argument before classifying it.
m=modules['regex_sql_parsing']
def V(p): return {'value':{'regex':p}}
def vrx(p):
 source=p.pattern.replace('(?<!%)',r'(?:^|[^%])')
 return V('(?i:'+source+')' if p.flags & re.I else source)
def merged(patterns):
 return '|'.join('(?:'+vrx(pattern)['value']['regex']+')' for pattern in patterns)
utils={'regex-false-positive':V(merged(m._FALSE_POSITIVE_INDICATORS)),
       'regex-error-message':V(merged(m._ERROR_MSG_INDICATORS)),
       'regex-sql-syntax':V(merged(m._SQL_SYNTAX_INDICATORS))}
false=M('regex-false-positive');error=M('regex-error-message');sql=M('regex-sql-syntax')
ordered=[('ERROR_MSG_PARSE',error),('SQL_COMMENT_STRIP',A(sql,V(r'--\[?\\?\^|/\\\*'))),
 ('SQL_PARAM_NORMALIZE',A(sql,O(A(V(r'(?i)(?:^|[^%])%S|\\?\\\$\\?\\?D|\\\?'),N(V(r'(?i)SELECT|INSERT|UPDATE|DELETE|FROM|WHERE|JOIN'))),V(r'(?i)\\bIN\\s\*\\\(\\s\*\\\?|\\bIN\\s\*\\\(\\s\*%s')))),
 ('CONSTRAINT_PARSE',A(sql,V(r'(?i)ON\s+DELETE|ON\s+UPDATE|ON DELETE|ON UPDATE|CHECK|json_valid|=\s*ANY\s*\('))),
 ('CONSTRAINT_PARSE',A(sql,V(r'(?i)IN\s*\\\(|\\s\+IN\\s'),N(V(r'(?i)SELECT|INSERT|DELETE|FROM|WHERE')))),
 ('TRIGGER_PARSE',A(sql,V(r'(?i)BEFORE|AFTER|INSTEAD|WHEN.*BEGIN'))),
 ('SQL_STRUCTURE_PARSE',A(sql,V(r'(?i)LIKE')))]
clauses=[];previous=[]
methods=['search','sub','match','findall','fullmatch','split','compile']
def inline_regex_utils(rule):
 if isinstance(rule,list):return [inline_regex_utils(x) for x in rule]
 if not isinstance(rule,dict):return rule
 if set(rule)=={'matches'}:return inline_regex_utils(utils[rule['matches']])
 return {k:inline_regex_utils(v) for k,v in rule.items()}
for i,(code,test) in enumerate(ordered+[('SQL_REWRITE',sql),('SQL_STRUCTURE_PARSE',sql)]):
 use_methods=['sub'] if i==len(ordered) else ([x for x in methods if x!='sub'] if i==len(ordered)+1 else methods)
 classifier=A(N(false),test,*(N(old) for old in previous))
 classified=O(A(CONST,classifier),A(FS,{'parse':{'grammar':'pythonFString','interpolation':'{...}',**inline_regex_utils(classifier)}}))
 # Parentheses disappear from Python's AST, including around an adjacent JoinedStr.
 pattern='regex-pattern-'+str(i)
 utils[pattern]=O(classified,A(K('parenthesized_expression'),H(M(pattern))))
 rule=A(K('call'),F('function',A(K('attribute'),F('object',names(['re'])),F('attribute',names(use_methods)))),
  F('arguments',H(A(M(pattern),{'nthChild':{'position':1,'ofRule':N(K('comment'))}}))))
 clauses.append(C('category-'+str(i),rule,code))
 if i<len(ordered):previous.append(test)
W('regex_sql_parsing',clauses,utils,roots=['data_layer/**/*.py','impl/**/*.py','_internal/**/*.py','l[0-9][0-9][0-9][0-9]_*/**/*.py'])
# Escape-hatch detector clauses share a rule-wide aggregate reporting ceiling.
m=modules['sql_grammar_escape_hatches'];utils={}
options=arrays(m,'ALLOWLISTED_CLASSES','DATA_VALUE_PARAMS','OPERATOR_PARAMS','SCHEMA_METADATA_CLASSES','CALLSITE_PROTECTED_CLASSES')
options.update(grammar_class_names={'type':'array','default':['SqlRenderable','SqlClause','SqlFragment','SqlParam']},safe_param_types={'type':'array','default':sorted(m._SAFE_PARAM_TYPES_STATIC)})
utils['type-name']=O(K('identifier'),A(K('attribute'),F('attribute',M('type-name'))),A(K('subscript'),F('value',M('type-name'))))
utils['base-name']=O(A(K('identifier'),P('BASE')),A(K('attribute'),F('attribute',A(K('identifier'),P('BASE')))),
 A(K('subscript'),F('value',M('base-name'))))
# Leaf types exclude transparent container names and qualified-name prefixes.
name_exempt=I(A(O(K('typed_parameter'),K('typed_default_parameter')),O(H(onames('DATA_VALUE_PARAMS')),F('name',onames('DATA_VALUE_PARAMS')),H(onames('OPERATOR_PARAMS')),F('name',onames('OPERATOR_PARAMS')))),O(K('typed_parameter'),K('typed_default_parameter')))
leaf=A(K('identifier'),N(RX(opt('safe_param_types'))),N(names(m.TRANSPARENT_CONTAINER_TYPES)),N(A(names(['str']),name_exempt)),P('LEAF'),P('ROW'))
utils['unsafe-leaf']=O(leaf,
 A(K('attribute'),F('attribute',M('unsafe-leaf'))),
 A(O(K('type'),K('generic_type'),K('union_type'),K('type_parameter'),K('tuple'),K('parenthesized_expression')),H(M('unsafe-leaf'))),
 A(K('subscript'),O(F('value',M('unsafe-leaf')),F('subscript',M('unsafe-leaf')))),
 A(K('binary_operator'),F('operator',RX(r'^\|$')),O(F('left',M('unsafe-leaf')),F('right',M('unsafe-leaf')))),
 A(STR,{'parse':{'grammar':'identifierWords','has':{'kind':'word','all':[N(RX(opt('safe_param_types'))),N(RX('^(?:'+'|'.join(re.escape(x) for x in sorted(m.TRANSPARENT_CONTAINER_TYPES))+')$'))],
   'capture':'LEAF','any':[{'capture':'ROW'}],'not':A(RX('^str$'),{'host':name_exempt})}}}),
 A(O(K('integer'),K('float'),K('true'),K('false')),P('LEAF'),P('ROW')))
forbidden=names(m.FORBIDDEN_ESCAPE_HATCHES)
skip=O(*(onames(n) for n in ('ALLOWLISTED_CLASSES','SCHEMA_METADATA_CLASSES','CALLSITE_PROTECTED_CLASSES')),forbidden)
param=A(O(K('typed_parameter'),K('typed_default_parameter')),P('PARAMETER'),N(H(names(['self']))),F('type',M('unsafe-leaf')),
 normal,
 N({'precedes':{**K('positional_separator'),'stopBy':'end'}}),
 N({'follows':{**O(K('keyword_separator'),K('list_splat_pattern'),A(K('typed_parameter'),H(K('list_splat_pattern')))),'stopBy':'end'}}))
# A base expression is one graph edge, so multiple bases remain separate rows. The parameter
# row is anchored back to its constructor, rather than to the base expression's source line.
class_decl=A(K('class_definition'),P('CLASS'),F('name',P('CLASS_NAME')))
classbase=A(M('base-name'),I(A(K('argument_list'),I(class_decl,'neighbor')),'neighbor'))
classedge=A(classbase,P('ROW'))
subject=A(classbase,I(A(K('class_definition'),N(F('name',skip)),direct_method(A(sync('__init__'),F('parameters',H(param))))),K('class_definition')))
utils['alias-target']=O(A(K('identifier'),P('ALIAS_FROM'),P('ROW')),A(K('attribute'),F('attribute',A(K('identifier'),P('ALIAS_FROM'),P('ROW')))),
 A(K('binary_operator'),F('operator',RX(r'^\|$')),O(F('left',M('alias-target')),F('right',M('alias-target')))),A(K('parenthesized_expression'),H(M('alias-target'))))
aliasedge=A(K('assignment'),F('left',A(K('identifier'),P('ALIAS_TO'))),F('right',M('alias-target')))
clauses=[C('forbidden-class',A(K('class_definition'),F('name',forbidden)),'escape_hatch'),
 G('unsafe-parameter',O(classedge,subject,aliasedge),
  {'base':'capture.BASE','class':'capture.CLASS_NAME','alias_from':'capture.ALIAS_FROM','alias_to':'capture.ALIAS_TO','leaf':'capture.LEAF.text',
   'position':'capture.PARAMETER.startLine','owner':'capture.CLASS_NAME','parameter':'capture.PARAMETER'},
  [{'name':'classes','seeds':{'$option':'grammar_class_names'},'edges':{'from':'base','to':'class'},'contains':{}},
   {'name':'aliases','extends':['classes'],'edges':{'from':'alias_from','to':'alias_to'},'contains':{'grammar':'base','safe':'leaf'}}],
  {'all':[{'eq':['row.grammar',True]},{'eq':['row.safe',False]},{'ne':['row.parameter',None]}]},'escape_hatch',
  groups=['row.path','row.position','row.owner'],anchor='capture.PARAMETER')]
for i,name in enumerate(sorted(m.FORBIDDEN_ESCAPE_HATCHES)):
 c=R('usage-'+str(i),re.escape(name)+r'\(|import '+re.escape(name)+r'|, '+re.escape(name),'escape_hatch');c['forbid']['regex']['lines'].update(terminators='lf',exclude=[r'^\s*#']);clauses.append(c)
W('sql_grammar_escape_hatches',clauses,utils,options,roots=['api/**/*.py','impl/**/*.py'],mode='block')
if args.signature_output is not None:
 args.signature_output.write_text(json.dumps({'source_commit':PIN,'source':'scripts/lint_sql_grammar_escape_hatches.py',
  'public_key_hex':m.ALLOWLIST_PUBLIC_KEY_HEX,'signature':m.ALLOWLIST_SIGNATURE},indent=2)+'\n')
# Callable hooks and constructor fields are matched by name, as the original visitor does.
utils={}
utils['constructor-name']=O(A(K('identifier'),P('CTOR')),A(K('attribute'),F('attribute',M('constructor-name'))),A(K('subscript'),F('value',M('constructor-name'))),A(K('call'),F('function',M('constructor-name'))))
utils['contains-callable']=O(term(['Callable']),H(M('contains-callable')))
parameter=O(A(K('identifier'),P('PARAM')),A(O(K('typed_parameter'),K('typed_default_parameter'),K('default_parameter')),O(F('name',P('PARAM')),H(A(K('identifier'),P('PARAM'))))))
callableparam=A(parameter,F('type',M('contains-callable')))
init=A(sync('__init__'),I(A(K('class_definition'),P('CLASS'),direct_method(sync('render'))),K('class_definition')))
field=A(K('attribute'),F('object',names(['self'])),F('attribute',A(K('identifier'),P('FIELD'))))
assignment=A(K('assignment'),F('left',field),F('right',A(K('identifier'),P('PARAM'))))
called=direct_method(A(sync('render'),P('RENDER_METHOD'),H(A(K('call'),F('function',field)),stop='end')))
cls=A(K('class_definition'),direct_method(sync('render')))
clauses=[C('namespace-render',A(K('call'),F('function',term(['SimpleNamespace'])),F('arguments',H(A(K('keyword_argument'),F('name',names(['render'])))))),'SIMPLE_NAMESPACE_RENDER_HOOK'),
 C('dynamic-render',A(K('assignment'),F('left',A(K('attribute'),F('attribute',names(['render']))))),'DYNAMIC_RENDER_ASSIGNMENT'),
 C('callable-param',A(callableparam,I(init,K('function_definition'))),'CALLABLE_INIT_PARAM')]
clauses.append(C('callable-field',A(init,F('parameters',H(callableparam)),H(assignment,stop='end')),'CALLABLE_FIELD',each='FIELD'))
clauses.append(C('called-field',A(sync('render'),I(A(cls,direct_method(A(sync('__init__'),P('INIT'),H(assignment,stop='end')))),K('class_definition')),H(A(K('call'),F('function',field)),stop='end')),'CONSTRUCTOR_FIELD_CALLED_IN_RENDER',each='FIELD'))
for id,ann,code in [('untyped',N(F('type',RX(r'(?s).'))),'UNTYPED_CALLED_CONSTRUCTOR_FIELD'),('any',F('type',A(K('type'),H(term(['Any'])))),'ANY_CALLED_CONSTRUCTOR_FIELD')]:
 # Existence of a field assignment still reports untyped if no matching parameter exists.
 paramtest=F('parameters',H(A(parameter,ann)))
 if id=='untyped':paramtest=O(paramtest,N(F('parameters',H(parameter))))
 clauses.append(C(id+'-called-field',A(init,H(assignment,stop='end'),paramtest,I(A(cls,called),K('class_definition'))),code,each='FIELD'))
# Lexical function-symbol lookup follows direct nested functions before local bindings.
utils['stored-name']=O(I(O(K('assignment'),K('augmented_assignment'),K('for_statement'),K('for_in_clause')),'neighbor',field='left'),I(A(O(K('pattern_list'),K('tuple_pattern'),K('list_pattern')),M('stored-name')),'neighbor'))
local=O(F('parameters',H(A(parameter,normal))),H(A(K('identifier'),P('NAME'),M('stored-name')),stop='end'))
directfunction=direct_method(A(sync(),F('name',P('NAME'))))
utils['symbol-scope']={'params':['NAME'],'rule':O(A(K('module'),H(A(sync(),F('name',P('NAME'))))),
 A(sync(),directfunction),A(sync(),N(local),I(M('symbol-scope',NAME='NAME'),O(sync(),K('module')))))}
functionarg=A(K('identifier'),P('NAME'),I(M('symbol-scope',NAME='NAME'),O(sync(),K('module'))))
actualarg=O(K('lambda'),functionarg)
# The oracle's registry is global and syntactic: no import or lexical binding is required.
renderclass=A(K('class_definition'),P('ROW'),F('name',P('SEED')),direct_method(sync('render')),
 O(I(K('module'),'neighbor'),I(A(K('decorated_definition'),I(K('module'),'neighbor')),'neighbor')))
ctorcall=A(K('call'),F('function',M('constructor-name')))
ctorarg=A(actualarg,P('ROW'),O(
 I(A(K('argument_list'),I(ctorcall,'neighbor')),'neighbor'),
 I(A(K('keyword_argument'),I(A(K('argument_list'),I(ctorcall,'neighbor')),'neighbor')),'neighbor')))
clauses.append(G('constructor-arg',O(renderclass,ctorarg),
 {'seed':'capture.SEED','constructor':'capture.CTOR','position':'capture.ROW.startByte'},
 [{'name':'renderables','seed_field':'seed','contains':{'known':'constructor'}}],
 {'eq':['row.known',True]},'CALLABLE_CONSTRUCTOR_ARG'))
W('grammar_callable_constructors',clauses,utils,roots=['impl/common/l2100_sql_grammar/**/*.py'],exclude=['**/tests/**','**/fragments.py'])
# Fragment ownership: recursive structural values, aliases, captured fields, and helper calls.
m=modules['grammar_fragment_usage'];utils={}
def module_has(r): return I(A(K('module'), H(r, stop='end')))
utils['has-fragment-import']=H(A(K('import_from_statement'),H(A(K('aliased_import'),
 F('name',RX(r'(?:^|\.)SqlFragment$'))))),stop='end')
fragment_alias=A(K('identifier'),P('ALIAS'),I(A(K('module'),M('has-fragment-import'),
 H(A(K('import_from_statement'),H(A(K('aliased_import'),F('name',RX(r'(?:^|\.)SqlFragment$')),F('alias',P('ALIAS'))))),stop='end'))))
utils['fragment-type']=O(
 names(['SqlFragment']), A(K('attribute'), F('attribute', names(['SqlFragment']))), fragment_alias,
 A(STR,V(r'(?:^|[^A-Za-z0-9_])SqlFragment(?:$|[^A-Za-z0-9_])')),
 A(STR,I(A(K('module'),M('has-fragment-import'))),{'parse':{'grammar':'identifierWords',
  'has':{'kind':'word','capture':'ANNOTATION_TOKEN','host':module_has(A(K('import_from_statement'),H(A(K('aliased_import'),
   F('name',RX(r'(?:^|\.)SqlFragment$')),F('alias',A(P('IMPORTED_TOKEN'),
   {'compare':{'left':'ANNOTATION_TOKEN','right':'IMPORTED_TOKEN','op':'eq'}}))))))}}}),
 A(O(*(K(k) for k in ['type','generic_type','type_parameter','union_type','binary_operator','list','tuple','subscript','parenthesized_expression'])), H(M('fragment-type')))
)
utils['grammar-base'] = {'params': ['NAME'], 'rule': O(
 A(K('identifier'), RX(r'^(?:Sql|_Sql)')),
 A(K('identifier'), module_has(A(K('import_from_statement'), H(A(
  K('aliased_import'), F('name', RX(r'^(?:Sql|_Sql)')), F('alias', P('NAME'))))))),
 A(K('identifier'), module_has(A(K('assignment'), F('left', P('NAME')),
  F('right', A(P('BASE'), M('grammar-base', NAME='BASE')))))),
 A(K('identifier'), module_has(A(K('class_definition'), F('name', P('NAME')), M('grammar-class')))),
 A(K('attribute'), F('attribute', RX(r'^(?:Sql|_Sql)')))
)}
utils['grammar-class']=A(K('class_definition'),N(F('name',names(m.FORBIDDEN_GRAMMAR_CLASSES|{'SqlFragment'}))),
 O(F('name',RX(r'^(?:Sql|_Sql)')),direct_method(fn('render')),F('superclasses',H(A(P('BASE'),M('grammar-base',NAME='BASE'))))))
stringmethods=['casefold','format','format_map','join','lower','lstrip','removeprefix','removesuffix','replace','rstrip','strip','translate','upper']
utils['build-string']=O(CONST,FS,
 A(O(K('list'),K('set'),K('tuple')),H(M('build-string'))),A(K('dictionary'),H(A(K('pair'),F('value',M('build-string'))))),
 A(K('binary_operator'),F('operator',RX(r'^\+$')),O(F('left',M('build-string')),F('right',M('build-string')))),
 A(K('conditional_expression'),O(H(A(M('build-string'),{'nthChild':1})),H(A(M('build-string'),{'nthChild':3})))),
 A(K('parenthesized_expression'),H(M('build-string'))),A(K('call'),F('function',O(names(['str']),A(K('attribute'),F('attribute',names(stringmethods)),F('object',M('build-string')))))),
 A(K('identifier'),P('STRING_NAME'),P('STRING_USE'),I(A(K('module'),H(A(K('assignment'),F('left',P('STRING_NAME')),F('right',M('build-string')),
 {'layout':{'after':{'to':'capture.STRING_USE','breaks':{'min':0}}}},N(I(K('class_definition')))),stop='end'))))
)
# Keep a frame around variable recursion so every alias can bind its own reference.

# Use framed calls at every expression edge (a framed empty mapping is named explicitly).
# Utilities with no params need no external bindings, but local aliases must not unify with callers.
utils['build-string']={'params':['DUMMY'], 'rule':utils['build-string']}
# String aliases inside the same constructor are local. Other methods do not supply local names.
build_rule=utils['build-string']['rule']
build_rule['any'][-1]['all'][-1] = module_has(A(
 K('assignment'), F('left', P('STRING_NAME')), F('right', M('build-string')),
 {'layout': {'after': {'to': 'capture.STRING_USE', 'breaks': {'min': 0}}}},
 O(N(I(K('class_definition'))), I(sync('__init__'), K('function_definition')))
))
field=A(K('attribute'),F('object',names(['self'])),F('attribute',P('FIELD')))
clsfield=O(
 A(K('assignment'), F('left', P('FIELD')),
   O(F('right', M('build-string')), F('type', A(K('type'), H(names(['str'])))))),
 A(K('assignment'), F('left', field), F('right', M('build-string')))
)
# Direct class storage and constructor assignment sources are independent.
tuple_target=F('left', A(O(K('tuple_pattern'),K('list_pattern'),K('pattern_list')), H(A(field,P('TARGET_ITEM')))))
tuple_value=F('right', A(O(K('tuple'),K('list'),K('expression_list')), H(A(M('build-string'), P('VALUE_ITEM'),
 {'compare':{'left':'TARGET_ITEM','right':'VALUE_ITEM','op':'eq','text':'index'}}))))
tuple_assignment=A(K('assignment'),tuple_target,tuple_value)
utils['captured-field']={'params':['FIELD'], 'rule': A(K('class_definition'), O(
 F('body', H(O(clsfield,A(K('expression_statement'),H(clsfield))))),
 direct_method(A(sync('__init__'),H(clsfield,stop='end'))),
 direct_method(A(sync('__init__'),H(tuple_assignment,stop='end')))
))}
unsafeparam=A(parameter,O(N(F('type',RX(r'(?s).'))),F('type',A(K('type'),H(term(['str','Any']))))))
utils['unsafe-field']={'params':['FIELD'],'rule':A(K('class_definition'),direct_method(A(sync('__init__'),
 H(A(K('assignment'),F('left',field),F('right',A(K('identifier'),P('PARAM')))),stop='end'),F('parameters',H(unsafeparam)))))}
utils['returned-field']={'params':['CLASS_NAME'],'rule':O(A(K('attribute'),F('object',O(names(['self']),P('CLASS_NAME'))),F('attribute',P('RETURNED_FIELD')),
 I(M('captured-field',FIELD='RETURNED_FIELD'),K('class_definition'))),
 A(K('binary_operator'),F('operator',RX(r'^\+$')),O(F('left',M('returned-field',CLASS_NAME='CLASS_NAME')),F('right',M('returned-field',CLASS_NAME='CLASS_NAME')))),
 A(K('conditional_expression'),O(H(A(M('returned-field',CLASS_NAME='CLASS_NAME'),{'nthChild':1})),H(A(M('returned-field',CLASS_NAME='CLASS_NAME'),{'nthChild':3})))),
 A(K('subscript'),F('value',M('returned-field',CLASS_NAME='CLASS_NAME'))),A(K('parenthesized_expression'),H(M('returned-field',CLASS_NAME='CLASS_NAME'))),
 A(K('call'),F('function',O(names(['SqlFragment']),A(K('attribute'),F('object',names(['SqlFragment'])),F('attribute',names(['static','with_params']))))),
 F('arguments',H(A(M('returned-field',CLASS_NAME='CLASS_NAME'),{'nthChild':{'position':1,'ofRule':N(K('comment'))}})))))}
unsafe_return=O(M('returned-field',CLASS_NAME='CLASS_NAME'),A(field,I(M('unsafe-field',FIELD='FIELD'),K('class_definition'))))
owner=A(K('class_definition'),P('OWNER'),F('name',P('CLASS_NAME')),N(F('name',names(m.FORBIDDEN_GRAMMAR_CLASSES))),direct_method(fn('render')))
# Match the enclosing direct method, including calls and returns in its nested bodies, as ast.walk does.
for order,side in [('earlier','after'),('later','before')]:
 utils[order+'-method']={'params':['NAME','CURRENT'],'rule':A(K('class_definition'),direct_method(A(fn(),F('name',P('NAME')),
  {'layout':{side:{'to':'capture.CURRENT','breaks':{'min':0}}}})))}
# The visitor enters the first render method, but its helper dictionary keeps the last definition.
selected_method=O(A(F('name',names(['render'])),N(I(M('earlier-method',NAME='METHOD',CURRENT='METHOD_NODE'),K('class_definition')))),
 A(N(F('name',names(['render']))),N(I(M('later-method',NAME='METHOD',CURRENT='METHOD_NODE'),K('class_definition')))))
method=A(fn(),P('METHOD_NODE'),F('name',P('METHOD')),I(A(owner,direct_method(A(fn(),P('METHOD_NODE')))),K('class_definition')),selected_method)
edge=A(K('call'),P('ROW'),F('function',A(K('attribute'),F('object',names(['self','cls'])),F('attribute',P('TARGET')))),I(method))
risky=A(K('return_statement'),P('ROW'),P('RETURN'),I(A(owner,N(F('name',names(m.FORBIDDEN_GRAMMAR_CLASSES))),direct_method(fn('render'))),K('class_definition')),
 H(unsafe_return),I(method))
clauses=[C('forbidden-class',A(K('class_definition'),F('name',names(m.FORBIDDEN_GRAMMAR_CLASSES))),'sqlfragment_usage'),
 C('constructor-fragment',A(sync('__init__'),I(M('grammar-class'),K('class_definition')),F('parameters',H(A(O(parameter,A(K('typed_parameter'),H(A(O(K('list_splat_pattern'),K('dictionary_splat_pattern')),H(A(K('identifier'),P('PARAM'))))))),N(H(names(['self']))),F('type',M('fragment-type')))))),'sqlfragment_usage',each='PARAM'),
 C('attribute-fragment',A(K('assignment'),F('type',M('fragment-type')),N(I(fn())),I(M('grammar-class'),K('class_definition'))),'sqlfragment_usage'),
 G('captured-return',O(edge,risky),
  {'caller':'capture.METHOD','target':'capture.TARGET','method':'capture.METHOD','owner':'capture.OWNER.startByte','position':'capture.ROW.startByte',
   'return':'capture.RETURN'},
  [{'name':'reachable','seeds':['render'],'partition_by':['path','owner'],'edges':{'from':'caller','to':'target'},'contains':{'reachable':'method'}}],
  {'all':[{'eq':['row.reachable',True]},{'ne':['row.return',None]}]},'sqlfragment_usage')]
parse=C('syntax-error',K('module'),'python_parse_error',anchor='file.start');parse['forbid']['ast']['syntax']='invalid';clauses.append(parse)
W('grammar_fragment_usage',clauses,utils,roots=['api/**/*.py','impl/**/*.py','data_layer/**/*.py','_internal/**/*.py','l[0-9]*/**/*.py'])
