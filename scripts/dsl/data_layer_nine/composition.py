# SPDX-License-Identifier: Apache-2.0
from .common import *

def candidate(root):
 c=constants(root/'scripts/lint_plan_composition_architecture.py')
 stages=[S('imports',"SELECT n.*,CASE WHEN n.kind='ImportFrom' THEN n.module ELSE (SELECT group_concat(s.name,',') FROM (SELECT name FROM a s WHERE s.__path=n.__path AND s.parent=n.id AND s.kind='alias' ORDER BY position) s) END AS imported FROM a n WHERE n.kind IN ('Import','ImportFrom')",('__path','id')),
 S('composition_methods',"SELECT m.* FROM methods m JOIN a c ON c.__path=m.__path AND c.id=m.parent WHERE m.class_name='SemanticPlanBuilder' AND c.parent=0 AND m.kind='FunctionDef'",('__path','id')),
 S('actual_annotations',"SELECT id AS record_id,0 AS ordinal,imported AS annotation FROM imports"),]
 from data_layer_eight.python_repr import annotations
 stages.append(annotations())
 queries=[row('SPC-001',"'semantic plan imports runtime module '||r.repr",'imports n JOIN annotation_reprs r ON r.record_id=n.id', ' OR '.join('instr(n.imported,'+q(part)+')>0' for part in c['_FORBIDDEN_IMPORT_PARTS'])+" OR substr(n.imported,-12)='.lazy_result' OR substr(n.imported,-9)='.lazy_seq'",order='n.bfs'),
 row('SPC-002',"'semantic field uses runtime type '||v.annotation_repr",'a n JOIN a v ON v.__path=n.__path AND v.parent=n.id AND v.field=\'annotation\'',"n.kind='AnnAssign' AND ("+' OR '.join('instr(v.annotation,'+q(part)+')>0' for part in c['_FORBIDDEN_RUNTIME_TYPE_PARTS'])+')',order='n.bfs'),
 row('SPC-004',"'semantic builder exposes mutable return from '||n.name",'composition_methods n',"substr(n.name,1,1)!='_' AND EXISTS(SELECT 1 FROM a v LEFT JOIN a root ON root.__path=v.__path AND root.parent=v.id AND root.field='value' WHERE v.__path=n.__path AND v.parent=n.id AND v.field='returns' AND (v.kind IN ('Name','Attribute') AND v.name IN ('dict','list','set') OR v.kind='Subscript' AND root.kind IN ('Name','Attribute') AND root.name IN ('dict','list','set')))",phase=1,order='n.id'),
 row('SPC-003',"'semantic builder calls runtime terminal '||n.callee",'composition_methods m JOIN calls n ON n.__path=m.__path AND n.id BETWEEN m.id AND m.last',f"n.callee_kind='Attribute' AND n.callee IN {literals(c['_FORBIDDEN_BUILDER_CALLS'])}",phase=1,order='m.id+n.bfs*0.000001+0.000001'),
 row('SPC-005',q('fragment import must use typed exact ref remapping'),'composition_methods n',"n.name='add_fragment' AND NOT EXISTS(SELECT 1 FROM composition_methods m WHERE m.__path=n.__path AND m.parent=n.parent AND m.name='add_fragment' AND m.id<n.id) AND (NOT "+desc('n',condition="n.kind='Name' AND n.name='RefRemap'",alias='n2').replace("n.kind='Name' AND n.name='RefRemap'","n2.kind='Name' AND n2.name='RefRemap'")+" OR NOT "+desc('n',condition="n2.kind='Name' AND n2.name='rewrite_exact_plan_refs_payload'",alias='n2')+')',phase=1,order='(SELECT cls.last FROM a cls WHERE cls.__path=n.__path AND cls.id=n.parent)+0.999999')]
 return document('plan_composition_architecture',[clause('plan-composition',stages,' UNION ALL '.join(queries)),live_clause()],['l2070_batching/semantic_plan.py'])

def live_clause():
    """The host observes the actual builder; SQL compares its typed observations.

    Production `all` requires the observation batch. Source-only entry points
    retain the upstream source API. Missing runtime input fails execution.
    """
    valid="""SELECT CASE WHEN json_type(data,'$.fingerprints')='array' AND json_array_length(data,'$.fingerprints')=2
      AND NOT EXISTS(SELECT 1 FROM json_each(data,'$.fingerprints') WHERE type!='text')
      AND json_type(data,'$.firstRefs')='array' AND json_type(data,'$.secondRefs')='array'
      AND NOT EXISTS(SELECT 1 FROM json_each(data,'$.firstRefs') WHERE type!='text')
      AND NOT EXISTS(SELECT 1 FROM json_each(data,'$.secondRefs') WHERE type!='text')
      THEN data ELSE json_extract('[]','$[composition observations required]') END data FROM supplied"""
    empty="SELECT '{\"fingerprints\":[\"same\",\"same\"],\"firstRefs\":[],\"secondRefs\":[]}' data"
    location='l2070_batching/semantic_plan.py'
    common=f"SELECT {q(location)} path,0 start_byte,0 end_byte,1 line,"
    first=common+"'SPC-012' issue,'equivalent semantic plans produced different fingerprints' message,'fingerprints' identity,0 phase,0 ordering,'' symbol,'' sort_name FROM observations WHERE json_extract(data,'$.fingerprints[0]')!=json_extract(data,'$.fingerprints[1]')"
    second=common+"'SPC-005' issue,'colliding fragment imports reused destination refs' message,'fragment-refs' identity,0 phase,1 ordering,'' symbol,'' sort_name FROM observations WHERE EXISTS(SELECT 1 FROM json_each(data,'$.firstRefs') first JOIN json_each(data,'$.secondRefs') second ON first.value=second.value)"
    active=first+' UNION ALL '+second
    return {'id':'composition-runtime-observations','relational':{
      'inputs':{'empty':E(K('module'),{'path':'file.path'})},'locations':[location],
      'stages':[S('supplied',{'$option':'parameters','escape':'sql','template':'SELECT {} data'}),S('observations',{'$option':'entryPoint','cases':{'all':valid,'run_behavior_probes':valid},'default':empty})],
      'query':{'$option':'entryPoint','cases':{'all':active,'run_behavior_probes':active},'default':common+"'SPC-012' issue,'' message,'' identity,0 phase,0 ordering,'' symbol,'' sort_name WHERE 0"},
      'key':['identity'],'code':'issue','message':'message'},'violation':{'code':'composition-observation','message':'Composition runtime observations','anchor':'witness.span'}}
