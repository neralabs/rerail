# SPDX-License-Identifier: Apache-2.0
"""Shared eager exemption and canonical-wrapper relations."""
from data_layer_six.common import q


def ignored_sql(path,pattern,symbol,text,vals,prefixes):
    return f"""({path} IN ('l2055_lazy_result/lazy_result.py','l2055_lazy_result/lazy_seq.py') AND ({symbol} IN {vals('LOCKED_PUBLIC_LAZY_FACADE_SYMBOLS')} OR rx_matches({symbol},{prefixes('LOCKED_PUBLIC_LAZY_FACADE_SYMBOL_PREFIXES')})))
      OR ({path}='l2070_batching/server_ref.py' AND {pattern} IN {vals('SERVER_REF_CORE_SLOT_PATTERN_NAMES')})
      OR ({path}='l2070_batching/step.py' AND {pattern}='transitive:*._ref_id' AND {symbol} GLOB 'l2070_batching.step.StepRef.*')
      OR ({path}='l2055_lazy_result/lazy_result.py' AND {pattern}='.resolve()' AND {symbol}='l2055_lazy_result.lazy_result.LazyResult.result')
      OR (NOT rx_matches({path},{prefixes('SERVERPLAN_LAZY_REL_PREFIXES')}) AND (instr({pattern},'__dict__')>0 OR instr({pattern},'getattr_static(')>0 OR {pattern} GLOB 'transitive:peek_*' OR {pattern} GLOB 'peek_*'))
      OR ({pattern}='.resolve()' AND rx_matches({symbol},{prefixes('NON_LAZY_RESOLVE_SYMBOL_PREFIXES')}))
      OR ({pattern}='.result()' AND {symbol} IN {vals('NON_LAZY_RESULT_SYMBOLS')})
      OR ({symbol} IN (SELECT symbol FROM approved_external) AND (replace({pattern},'transitive:','') IN ('*.resolve()','*.resolve_async()','*.result()','.resolve()','.resolve_async()','.result()') OR replace({pattern},'transitive:','') GLOB 'non-lazy-call:*'))
      OR ({pattern}='.resolve()' AND instr({text},'ResolveVersionedForeignKeyTargetIntent(')>0)"""


def apply(stages,vals,prefixes):
    get=lambda name:next(stage for stage in stages if stage['name']==name)
    wrappers=get('canonical_wrappers')
    wrappers['query']=wrappers['query'].replace('SELECT f.__path,f.id FROM',"SELECT f.__path,f.id,rx_replace(substr(lazy.full_callee,1,length(lazy.full_callee)-5),'^self\\.','') family FROM",1)
    wrappers['query']+=""" UNION ALL SELECT f.__path,f.id,rx_replace(substr(lazy.full_callee,1,length(lazy.full_callee)-5),'^self\\.','') family
      FROM function_info f JOIN meaningful_statements s ON s.__path=f.__path AND s.fn=f.id
      JOIN assignments a ON a.__path=s.__path AND a.parent=s.id AND a.target_kind='identifier' AND a.type_start IS NULL
      JOIN calls lazy ON lazy.__path=a.__path AND lazy.id=a.value
      JOIN meaningful_statements r ON r.__path=f.__path AND r.fn=f.id AND r.__start>s.__start AND r.kind='return_statement'
      JOIN nodes value ON value.__path=r.__path AND value.parent=r.id AND value.ordinal=0
      LEFT JOIN nodes awaited ON awaited.__path=value.__path AND awaited.parent=value.id AND value.kind='await'
      JOIN calls c ON c.__path=value.__path AND c.id=CASE WHEN rx_matches(f.text,'^async\\b') THEN awaited.id ELSE value.id END
      JOIN arguments arg ON arg.__path=c.__path AND arg.call=c.id AND arg.position=0
      JOIN attributes terminal ON terminal.__path=arg.__path AND terminal.id=arg.id
      JOIN nodes receiver ON receiver.__path=terminal.__path AND receiver.id=terminal.object AND receiver.kind='identifier' AND receiver.text=a.target_text
      WHERE lazy.callee GLOB '*_lazy' AND NOT rx_matches(lazy.full_callee,'[()]') AND c.callee_kind='identifier'
      AND c.callee=CASE WHEN rx_matches(f.text,'^async\\b') THEN 'run_in_legacy_execution_boundary_async' ELSE 'run_in_legacy_execution_boundary' END
      AND terminal.attr=CASE WHEN rx_matches(f.text,'^async\\b') THEN 'resolve_async' ELSE 'resolve' END
      AND (SELECT count(*) FROM arguments x WHERE x.__path=c.__path AND x.call=c.id)=1
      AND NOT EXISTS(SELECT 1 FROM keywords k WHERE k.__path=c.__path AND k.call=c.id)
      AND (SELECT count(*) FROM meaningful_statements other WHERE other.__path=f.__path AND other.fn=f.id)=2"""
    direct=get('direct_patterns')
    direct['query']="SELECT l.__path,l.__start,l.__end,l.issue,l.__start||':'||l.issue identity,l.symbol FROM direct_sites l WHERE NOT ("+ignored_sql('l.__path','l.issue','l.symbol','l.text',vals,prefixes)+')'
