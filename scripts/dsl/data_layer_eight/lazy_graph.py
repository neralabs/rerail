# SPDX-License-Identifier: Apache-2.0
"""Share ordered graph producers between source and supplied-fact consumers."""
from data_layer_six.common import S,q


def traversal(name,roots,edges,sinks,on_edge=False):
    return {'name':name,'reachability':{'selection':'memoized-first','roots':roots,'edges':edges,'sinks':sinks,'match':'edge' if on_edge else 'node'}}


def apply(stages,vals):
    get=lambda name:next(stage for stage in stages if stage['name']==name)
    insertion=stages.index(get('rollups'))
    shared=[
      S('graph_functions',"SELECT f.* FROM function_flags f WHERE NOT EXISTS(SELECT 1 FROM function_flags later WHERE later.symbol=f.symbol AND later.fact_order>f.fact_order)",('symbol',),('__path','id')),
      S('graph_edges',"SELECT owner.__path,owner.symbol src,target.symbol dst,e.visit_key ordinal,e.__start location FROM architecture_edges e JOIN graph_functions owner ON owner.__path=e.__path AND owner.id=e.fn JOIN graph_functions target ON target.__path=e.target_path AND target.id=e.target",('__path','src')),
      S('graph_categories',"SELECT 'plan' kind UNION ALL SELECT 'intent' UNION ALL SELECT 'native' UNION ALL SELECT 'dolt' UNION ALL SELECT 'orchestration'",('kind',)),
      S('local_graph_events',"SELECT c.kind,e.__path,e.fn,e.__start location,e.visit_key,e.channel FROM function_events e JOIN graph_categories c ON c.kind=e.category OR c.kind='orchestration' AND e.category='plan' AND e.detail NOT GLOB 'native_cursor.*'",('__path','fn','kind')),
      S('graph_sinks',"SELECT NULL __path,kind||':'||symbol id,location FROM (SELECT e.*,f.symbol,row_number() OVER(PARTITION BY f.symbol,e.kind ORDER BY e.visit_key,e.channel) chosen FROM local_graph_events e JOIN graph_functions f ON f.__path=e.__path AND f.id=e.fn) WHERE chosen=1"),
      traversal('graph_rollups',"SELECT c.kind||':'||f.__path __path,c.kind||':'||f.symbol id,f.fact_order ordinal FROM function_flags f CROSS JOIN graph_categories c", "SELECT c.kind||':'||e.__path __path,c.kind||':'||e.src src,c.kind||':'||e.dst dst,e.ordinal,e.location FROM graph_edges e CROSS JOIN graph_categories c",'SELECT __path,id,location FROM graph_sinks'),
      traversal('graph_legacy',"SELECT f.__path,f.symbol id,f.fact_order ordinal FROM function_flags f WHERE f.pure",'SELECT __path,src,dst,ordinal,location FROM graph_edges',"SELECT NULL __path,target.symbol id,target.__start location FROM graph_functions target WHERE target.legacy",True),
      traversal('graph_permission',"SELECT f.__path,f.symbol id,f.fact_order ordinal FROM function_flags f",'SELECT __path,src,dst,ordinal,location FROM graph_edges',"SELECT NULL __path,target.symbol id,target.__start location FROM graph_functions target WHERE target.symbol IN "+vals('PREPROCESS_PERMISSION_IO_ENTRYPOINTS'),True),
    ]
    stages[insertion:insertion]=shared
    get('rollups')['query']="SELECT f.__path,f.id root,c.kind category,r.location FROM graph_rollups r CROSS JOIN graph_categories c JOIN function_flags f ON r.__path=c.kind||':'||f.__path AND r.root=c.kind||':'||f.symbol"
    get('legacy_matches')['query']='SELECT f.__path,f.id root,r.location FROM graph_legacy r JOIN function_flags f ON f.__path=r.__path AND f.symbol=r.root'
    index=stages.index(get('transitive_matches'))
    stages[index:index]=[
      traversal('graph_eager',"SELECT f.__path,f.symbol id,f.fact_order ordinal FROM function_flags f WHERE f.pure AND NOT EXISTS(SELECT 1 FROM graph_legacy l WHERE l.__path=f.__path AND l.root=f.symbol)", 'SELECT __path,src,dst,ordinal,location FROM graph_edges',"SELECT NULL __path,f.symbol id,i.location FROM intrinsic i JOIN graph_functions f ON f.__path=i.__path AND f.id=i.id"),
      traversal('graph_nonlazy',"SELECT f.__path,f.symbol id,f.fact_order ordinal FROM function_flags f WHERE f.is_lazy AND NOT(f.pure AND EXISTS(SELECT 1 FROM graph_legacy l WHERE l.__path=f.__path AND l.root=f.symbol))", "SELECT e.* FROM graph_edges e WHERE e.dst NOT IN "+vals('SANCTIONED_PURE_HELPER_SYMBOLS'), "SELECT NULL __path,f.symbol id,f.__start location FROM graph_functions f WHERE NOT f.is_lazy",True),
    ]
    get('transitive_matches')['query']="SELECT f.__path,f.id root,r.location eager,r.node target,i.reason FROM graph_eager r JOIN function_flags f ON f.__path=r.__path AND f.symbol=r.root JOIN graph_functions target ON target.symbol=r.node JOIN intrinsic i ON i.__path=target.__path AND i.id=target.id"
    get('non_lazy_matches')['query']="SELECT f.__path,f.id root,r.location,r.node target FROM graph_nonlazy r JOIN function_flags f ON f.__path=r.__path AND f.symbol=r.root"
