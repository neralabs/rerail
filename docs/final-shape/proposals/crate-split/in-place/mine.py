exec(open('assign.py').read().split("if __name__")[0])
import sys
AST = ' '.join('logic::front::' + x for x in 'operator Term Atom AggOp Literal cmp_binds join Rule LatticeDecl Value Program Again QUALIFIERS OPTION_GLOB GLOBAL LIBRARY globals SCOPE_NOT SpanIn Native Error RESERVED AXIS DESCENDANTS NEAREST is_axis is_transduced axis EVERY_KIND'.split())
MINE = f"""
prof: prof phase_clock logic::profile
rrsyn: logic::rr::tape logic::rr::syntax {AST} logic::magic::Report logic::magic::Gate
cpython: cpython
lang: language identifier json lines budget module_path semver package_manifest npm_ignore module_graph tape tree value::python::words value::text_regex
facts: facts producers text logic::strings sql_profile logic::plan::sketch
rr: logic::front logic::magic logic::rr
logic: logic typing waves::owner waves::schedule waves::storage threads logic::rr::from_tape
sessions: sql_executor
transducer: demand residency space transducer transducer::packed transducer::packed::metal_batch_abi
metal: transducer::packed::metal_backend transducer::packed::metal_grid transducer::packed::metal_program_backend residency::metal residency::slabs
w: residency::admission relational blade calls compile embed eval findings import_target language::adapter literal matcher model origin php_literal php_view placement query scope soundness stats suppress value violations vue workspace facts::facts_a facts::producers::python::import_screen facts::producers::python::python_facts hooks test_support crypto profile
top: dsl_root slice tools prefilter waves waves::matches waves::findings waves::policies waves::relational logic::convert logic::plan::corpus::references transducer::packed::compile_matcher transducer::semantic_facts
"""
# metal_* files generically
import re
mods = set(m.replace('crate::code_rule_dsl::', '') for m in D['mods'].values())
extra = sorted({'::'.join(m.split('::')[:3]) for m in mods if m.startswith('transducer::packed::metal_') and not m.startswith('transducer::packed::metal_batch_abi')})
MINE = MINE.replace('metal: ', 'metal: ' + ' '.join(extra) + ' ')
DAG = {'prof': [], 'store': [], 'rrsyn': [], 'cpython': ['prof'], 'lang': ['prof', 'rrsyn'], 'facts': ['lang', 'store'], 'rr': ['rrsyn', 'prof', 'facts'],
  'logic': ['rr', 'facts'], 'sessions': ['logic'], 'transducer': ['facts'], 'metal': ['transducer'], 'w': ['facts'],
  'top': ['w', 'sessions', 'metal', 'cpython']}
ALLOW = closure(DAG)
if __name__ == '__main__':
    run(MINE, ALLOW, 'mine', detail='-d' in sys.argv)
