"""Project the integrated engine's State Tripwires relations into native findings."""
import ast
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile

CHECK = 'serverplan_state_tripwires'
MAIN_MESSAGES = {
    'production-ir-class': 2433, 'graph-id-config': 2459,
    'queue-auth-definitions': 2658, 'terminal-hydration-definitions': 2672,
    'queue-auth-calls': 2757, 'lazy-callback-terminal': 2806,
    'graph-terminal-chain': 2819, 'private-lazy-ref': 2841,
    'mcp-default-graph': 2854, 'large-column-reentry': 2867,
    'large-column-hydration': 2880, 'preprocess-io': 2918,
    'provider-io': 2946, 'direct-planning-io': 2960,
    'query-planning-io': 2980, 'resolved-ref-access': 2999,
    'nested-plan-execution': 3013, 'executor-plan-execution': 3033,
    'compiled-ir-value': 3052, 'executor-ir-dispatch': 3071,
    'production-ir-construction': 3085, 'test-only-acknowledgement': 3098,
    'legacy-preop': 3116, 'serverplan-ddl': 3135,
    'step-aware-eager-helper': 3154, 'materialized-pipeline': 3172,
    'context-authority': 3192, 'queue-udf-symbols': 3210,
}
SIGNATURE_MESSAGES = {'SPT-015': 2512, 'SPT-016': 2529, 'SPT-017': 2550,
                      'SPT-013': 2577, 'SPT-014': 2591, 'SPT-018': 2643}
FLOW_MESSAGES = [3381, 3269, 3291, 3482, 3313, 3402, 3422, 3442, 3462,
                 3502, 3523, 3545, 3567, 3607, 3357, 3329,
                 2788, 2788, 2788, 2788, 2770, 2894]


def source_messages(source):
    """Read diagnostic text offline; runtime rules never invoke the source linter."""
    result = {}
    for node in ast.walk(ast.parse(source.read_text())):
        if not isinstance(node, ast.Call) or not isinstance(node.func, ast.Name) or node.func.id != 'Violation':
            continue
        values = {k.arg: k.value for k in node.keywords}
        if not isinstance(values.get('code'), ast.Constant):
            continue
        text = values['message']
        if isinstance(text, ast.JoinedStr):
            parts = []
            for part in text.values:
                if isinstance(part, ast.Constant):
                    parts.append(part.value)
                elif isinstance(part.value, ast.Name):
                    parts.append('{detail}' if part.value.id != 'call_name' else '{callee}')
                else:
                    raise ValueError(f'Unsupported diagnostic interpolation at {node.lineno}')
            text = ''.join(parts)
        else:
            text = ast.literal_eval(text)
        result[node.lineno] = (values['code'].value, text)
    return result


def specialize(value, utils, options):
    if isinstance(value, list):
        return [specialize(v, utils, options) for v in value]
    if not isinstance(value, dict):
        return value
    if '$option' in value:
        choice = str(options[value['$option']]).lower()
        return specialize(value.get('cases', {}).get(choice, value.get('default')), utils, options)
    if 'matches' in value and value['matches'] not in utils:
        raise ValueError('Unknown matcher utility: ' + value['matches'])
    return {k: specialize(v, utils, options) for k, v in value.items()}


def converted_parts(root):
    """Use Final's maintained core-relation converter at authoring time only."""
    with tempfile.TemporaryDirectory(prefix='rerail-state-author-') as scratch:
        subprocess.run([sys.executable, str(root / 'scripts/dsl/data_layer_rr_port.py'),
                        str(root / 'native/src/code_rule_dsl/logic/programs/data_layer'),
                        CHECK, scratch], check=True)
        return {p.name: p.read_text() for p in Path(scratch).glob('*.rr')}


def namespace(text, prefix, names=None):
    if names is None:
        names = set(re.findall(r'^\.(?:decl|lattice)\s+(\w+)', text, re.M))
    tokens = re.compile(r'//[^\n]*|"(?:\\.|[^"\\])*"|[A-Za-z_]\w*')
    def replace(match):
        token = match.group()
        if token in names and (match.start() == 0 or text[match.start() - 1] != '@'):
            return prefix + token
        return token
    return tokens.sub(replace, text)


def body(text):
    policy = json.loads(next(l[8:] for l in text.splitlines() if l.startswith('.policy ')))
    define = policy['clauses'][0]['define']
    options = {name: v['default'] for name, v in define.get('options', {}).items()}
    # The source scanner accepts supplied code independently of repository discovery.
    # Production scope is enforced separately, before these scanner relations.
    options.update(oracleAnalyzer=True, entryPoint='scan_project_sources')
    text = '\n'.join(l for l in text.splitlines()
                     if not l.startswith(('.policy ', '.output ', '//')))
    decoder = json.JSONDecoder()
    pieces, offset = [], 0
    for match in re.finditer(r'ast\(\w+,\s*\w+,\s*(?=\{)', text):
        if match.end() < offset:
            continue
        value, length = decoder.raw_decode(text[match.end():])
        pieces.extend([text[offset:match.end()], json.dumps(specialize(value, define['utils'], options))])
        offset = match.end() + length
    text = ''.join(pieces) + text[offset:]
    # Explicit expression parses serve the printer below. They are not files
    # submitted to the original scanner.
    return '\n'.join(line[:-1] + ', doc_role(d,"primary").' if 'ast(d,' in line and line.endswith('.') else line
                     for line in text.splitlines())


def ordered_sinks(program):
    """Match the source's ordered, shared DFS memo instead of shortest paths."""
    start=program.index('.lattice shortlex_min<symbol>')
    end=program.index('.decl signed_exceptions',start)
    replacement='''
.decl source_edge(f: node, target: node, line: number, name: symbol)
source_edge(f,t,l,q) :- call_graph(f,c,t), line(c,l), ctx_display(t,q).
.decl edge_before(f: node, target: node, line: number, name: symbol, earlier: node, earlier_line: number, earlier_name: symbol)
edge_before(f,t,l,q,u,k,s) :- source_edge(f,t,l,q), source_edge(f,u,k,s), (k < l; k = l, s < q).
.decl edge_rank(f: node, target: node, line: number, name: symbol, ordinal: number)
edge_rank(f,t,l,q,i) :- source_edge(f,t,l,q), i = count : { edge_before(f,t,l,q,_,_,_) }.
.decl ordered_edge(f: node, target: node, ordinal: number)
ordered_edge(f,t,i) :- edge_rank(f,t,_,_,i).
.decl transitive_sinks(fn: node, n: node, kind: symbol, origin: node)
transitive_sinks(f,n,k,f) :- direct_sinks(f,n,k), reachable(f).
'''
    conditions={
        'eager_sql':['ctx_planning(f)','ctx_preprocess(f)','ctx_provider_helper(f)'],
        'ctx_access':['ctx_planning(f)','ctx_helper(f)'],
        'large_column':['ctx_planning(f)'], 'resolved_refs':['ctx_planning(f)'],
        'nested_execute':['ctx_planning(f)'], 'materialized_pipeline':['ctx_planning(f)'],
        'runtime_authority':['ctx_planning(f)'], 'runtime_value':['ctx_planning(f)'],
        'provider_authority':['ctx_planning(f)'], 'runtime_callback':['ctx_planning(f)'],
        'runtime_semantic_projection':['ctx_planning(f)'],
        'runtime_sequence_projection':['ctx_materializer(f), !ctx_planning(f)'],
    }
    for kind,roots in conditions.items():
        target,root='target_'+kind,'roots_'+kind
        replacement+=f'.decl {target}(f: node)\n{target}(f) :- direct_sinks(f,_,"{kind}").\n'
        replacement+=f'.decl {root}(f: node, ordinal: number)\n'
        for condition in roots:
            replacement+=f'{root}(f,i) :- {condition}, start(f,i).\n'
        replacement+=f'''transitive_sinks(f,c,"{kind}",t) :- {root}(f,_), !{target}(f),
    dfs_first_memo(f,t,i,ordered_edge,{target},{root}), edge_rank(f,u,l,_,i), call_graph(f,c,u), line(c,l).
'''
    return program[:start]+replacement+program[end:]


def generate(source, target, header):
    root = Path(__file__).resolve().parents[3]
    messages = source_messages(source / ('scripts/lint_' + CHECK + '.py'))
    parts = converted_parts(root)
    metadata = json.loads(next(l[8:] for l in parts[CHECK + '.rr'].splitlines() if l.startswith('.policy ')))
    shared = metadata['clauses'][0]['define']
    settings = {name: v['default'] for name,v in shared['options'].items()}
    settings.update(oracleAnalyzer=True, entryPoint='scan_project_sources')
    # Recursive matcher utilities are metadata in Final's current syntax. They
    # carry no detector, SQL stage, reporter, or runtime option interpretation.
    utilities = specialize(shared['utils'], shared['utils'], settings)
    text = header(CHECK, target)
    text += '#include "python/print.rr"\n'
    text += '#include "data-layer/purity-ast.rr"\n'
    text += '.policy ' + json.dumps(dict(format='rerail.policy/v1',id='data-layer-state-matchers',
        revision=1,mode='block',clauses=[dict(id='shared',define=dict(language='python',utils=utilities))])) + '\n'
    text += '''
.decl spt_frame_kind(k: symbol)
spt_frame_kind("function_definition"). spt_frame_kind("class_definition").
.decl spt_frame_name(n: node, name: symbol)
spt_frame_name(n, @nfkc(raw)) :- kind(n,k), spt_frame_kind(k), def_name(n,raw).
.decl spt_frame_qual(n: node, qualname: symbol)
spt_frame_qual(n,name) :- spt_frame_name(n,name), !nearest(n,_,spt_frame_kind).
spt_frame_qual(n,cat(q,".",name)) :- spt_frame_name(n,name), nearest(n,p,spt_frame_kind), spt_frame_qual(p,q).
.decl spt_prefix(n: node, prefix: symbol)
spt_prefix(n,cat(q,".")) :- node(_,n,_,_,_), nearest(n,p,spt_frame_kind), spt_frame_qual(p,q).
spt_prefix(n,"") :- node(_,n,_,_,_), !nearest(n,_,spt_frame_kind).
.decl spt_callee(n: node, name: symbol)
spt_callee(n,@nfkc(t)) :- ast_name(n,t).
spt_callee(n,@nfkc(t)) :- kind(n,"attribute"), field(n,"attribute",a), text(a,t).
spt_callee(n,s) :- kind(n,"parenthesized_expression"), unparen(n,r), n != r, spt_callee(r,s).
.decl spt_leaf(n: node, name: symbol)
spt_leaf(n,c) :- kind(n,"call"), field(n,"function",f), spt_callee(f,c).
spt_leaf(n,"<call>") :- kind(n,"call"), field(n,"function",f), !spt_callee(f,_).
spt_leaf(n,@nfkc(t)) :- kind(n,"attribute"), field(n,"attribute",a), text(a,t).
spt_leaf(n,t) :- spt_frame_name(n,t).
spt_leaf(n,"<string>") :- kind(n,"string").
.decl spt_qual(n: node, qualname: symbol)
spt_qual(n,cat(p,l)) :- spt_prefix(n,p), spt_leaf(n,l).
.decl spt_ir_arg(n: node, value: node)
spt_ir_arg(n,v) :- kind(n,"call"), field(n,"arguments",a), child(a,k,_), kind(k,"keyword_argument"),
    field(k,"name",kn), text(kn,"ir"), field(k,"value",v).
.decl spt_ir_name(n: node, name: symbol)
spt_ir_name(n,s) :- spt_callee(n,s).
spt_ir_name(n,s) :- kind(n,"call"), field(n,"function",f), spt_ir_name(f,s).
spt_ir_name(n,s) :- kind(n,"parenthesized_expression"), child(n,c,_), spt_ir_name(c,s).
.decl spt_ir_display(n: node, text: symbol)
spt_ir_display(n,s) :- spt_ir_arg(_,n), spt_ir_name(n,s).
.decl spt_expression_tape(n: node, t: tape)
.decl spt_expression_request(n: node, request: symbol)
spt_expression_request(n,cat("def _spt_expression(value: ",raw,"):\\n    pass\\n")) :-
    spt_ir_arg(_,n), !spt_ir_name(n,_), text(n,raw).
spt_expression_tape(n,t) :- spt_expression_request(n,request), parse_of(request,"python",t).
spt_ir_display(n,s) :- spt_expression_tape(n,t), tape_of(d,t), kind(f,"function_definition"), docof(f,d),
    field(f,"parameters",ps), child(ps,p,_), field(p,"type",a), annotation_text(a,s).
.decl spt_result(n: node, qualname: symbol, code: symbol, message: symbol, owner: node, phase: number, suborder: number, facts: number)
'''
    generated = json.loads((source / 'scripts/serverplan_return_kinds.json').read_text())
    text += '.decl spt_generated(name: symbol, kind: symbol)\n'
    for name,kinds in generated.items():
        for kind in kinds:
            text += f'spt_generated({json.dumps(name)},{json.dumps(kind)}).\n'
    def projection(ident, line, predicate, qual, extra=''):
        nonlocal text
        code, message = messages[line]
        if '{detail}' in message and line == 3052:
            extra += ', spt_ir_arg(n,ir), spt_ir_display(ir,display), detail = @python_repr(display)'
        elif '{detail}' in message:
            extra += ', detail = raw_detail'
        chunks = re.split(r'(\{(?:detail|callee)\})', message)
        pieces = [part[1:-1] if part.startswith('{') else json.dumps(part) for part in chunks if part]
        expression = pieces[0] if len(pieces) == 1 else 'cat(' + ','.join(pieces) + ')'
        owner,phase,suborder,facts='n',line,0,0
        if ident.startswith('StateSignature'):
            owner='f'
            if line==2591:
                phase,suborder=2577,1
        elif ident.startswith('StateFlow') and int(ident.removeprefix('StateFlow'))<16:
            owner,facts='f',1
        elif ident=='StateProject':
            owner,facts='f',1
        elif line==2459:
            owner='c'
        text += f'// {ident}: pinned source line {line}.\n'
        text += f'spt_result(n,q,{json.dumps(code)},{expression},{owner},{phase},{suborder},{facts}) :- {predicate}, {qual}{extra}.\n'
    for index, (name, raw) in enumerate(sorted(parts.items())):
        prefix = f'spt_{index}_'
        program = body(raw)
        if name.endswith('state_flow_1.rr') or name.endswith('state_flow_project_1.rr'):
            # Runtime API maps replace, rather than augment, generated categories.
            program = re.sub(r'return_kind_entry\(("(?:\\.|[^"\\])*"), ("(?:\\.|[^"\\])*")\)\.', '', program)
            program += '''
return_kind_entry(n,k) :- option_item("returnKindNames",i,n), option_item("returnKindValues",i,k).
return_kind_entry(n,k) :- spt_generated(n,k), !option("entryPoint",_).
'''
        if name.endswith('state_flow_1.rr'):
            from data_layer_rr_state_analysis import scoped
            program = scoped(program)
            program = ordered_sinks(program)
            program = re.sub(r', !signed\("SPT-\d+", f\)', '', program)
            # Keep the root function. A transitive witness alone loses its owner.
            pattern = re.compile(r'finding\("state-flow-1", "(SPT-\d+)", n\) :- (.*?)(?=\.\s*(?:\n|$))\.', re.S)
            rules = list(pattern.finditer(program))
            names = set(re.findall(r'^\.(?:decl|lattice)\s+(\w+)', program, re.M))
            if len(rules) != len(FLOW_MESSAGES):
                raise ValueError(f'Expected {len(FLOW_MESSAGES)} flow branches, got {len(rules)}')
            for ordinal, (match, line) in enumerate(zip(rules, FLOW_MESSAGES)):
                definition = match.group(2)
                # Direct getattr diagnostics use the call's qualified name.
                qual = 'spt_qual(n,q)' if ordinal >= 16 else 'spt_frame_qual(f,q)'
                if ordinal == 21:
                    qual = 'spt_qual(n,q)'
                projection(f'StateFlow{ordinal}', line, namespace(definition,prefix,names), qual)
            program = pattern.sub('', program)
        text += namespace(program, prefix) + '\n'
        if name.endswith('state_flow_1.rr'):
            text += f'''
.violation StateAnalysisContext(at: span, family: symbol, line: number, name: symbol, class_scope: symbol) code "SPT_ANALYSIS_CONTEXT" "{{family}}: {{name}}"
StateAnalysisContext(@place(p,l),"facts",l,name,cls) :- option("entryPoint","_analysis_contexts"),
    {prefix}fact_context(f), {prefix}contexts(f,name,cls), docof(f,d), doc(d,p,_), line(f,l).
StateAnalysisContext(@place(p,l),"runtime",l,name,cls) :- option("entryPoint","_analysis_contexts"),
    {prefix}runtime_context(f), {prefix}contexts(f,name,cls), docof(f,d), doc(d,p,_), line(f,l).
'''
        if name == CHECK + '.rr':
            for ordinal, (clause, line) in enumerate(MAIN_MESSAGES.items()):
                pred = (f'{prefix}events_detail({json.dumps(clause)},n,raw_detail)' if clause in
                        {'compiled-ir-value','executor-ir-dispatch','graph-id-config'} else
                        f'{prefix}events({json.dumps(clause)},n)')
                qual = 'spt_qual(n,q)'
                extra = ', spt_callee(fn,callee), field(n,"function",fn)' if line == 3085 else ''
                if clause == 'graph-id-config':
                    qual = 'nearest(n,c,spt_frame_kind), spt_frame_name(c,cls), q = cat(cls,".",raw_detail)'
                elif clause == 'production-ir-class':
                    qual = 'spt_frame_name(n,q)'
                projection(f'StateDirect{ordinal}', line, pred, qual, extra)
        elif name.endswith('state_signatures_1.rr'):
            for ordinal, (code, line) in enumerate(SIGNATURE_MESSAGES.items()):
                qual = 'nearest(n,f,spt_frame_kind), spt_frame_qual(f,q)'
                projection(f'StateSignature{ordinal}', line, f'{prefix}finding(_,"{code}",n)', qual)
        elif name.endswith('state_projection_recovery_2.rr'):
            projection('StateRecovery',2718,f'{prefix}finding(_,"SPT-029",n)',
                       'spt_prefix(n,p), q = cat(p,"if")')
        elif name.endswith('state_flow_project_1.rr'):
            projection('StateProject',3607,f'{prefix}project_builder_rows(f,n), option("entryPoint","scan_project_sources")',
                       'spt_frame_qual(f,q)')
    from data_layer_rr_author import constants
    settings=constants(source / ('scripts/lint_' + CHECK + '.py'))
    text += '\n.decl spt_function_token(token: symbol)\n'
    for token in sorted(settings['FUNCTION_FACT_SCAN_TOKENS']):
        text += f'spt_function_token({json.dumps(token)}).\n'
    text += '''
.decl spt_needs_facts(d: number)
spt_needs_facts(d) :- doc(d,_,_), option("sourceProvided","false").
spt_needs_facts(d) :- doc(d,"impl/common/l2070_server_plan/backend_profiles.py",_).
spt_needs_facts(d) :- source(d,sp), text(sp,s), spt_function_token(t), contains(t,s).
.decl spt_scan_doc(d: number)
.decl spt_scanner_mode()
spt_scanner_mode() :- !option("entryPoint",_).
spt_scanner_mode() :- option("entryPoint","scan_source").
spt_scanner_mode() :- option("entryPoint","scan_tree").
spt_scanner_mode() :- option("entryPoint","scan_project_sources").
spt_scan_doc(d) :- doc(d,_,_), spt_scanner_mode(), !option("entryPoint","scan_project_sources").
spt_scan_doc(d) :- doc(d,_,_), option("entryPoint","scan_project_sources"), source(d,sp), text(sp,s), spt_scan_token(t), contains(t,s).
.decl spt_output(path: symbol, line: number, qualname: symbol, code: symbol, message: symbol, mode: symbol, visit: number, phase: number, ordinal: number)
// The source's fast return preserves visitor order and multiplicity. Its fact
// path deduplicates complete diagnostics and sorts them after analysis.
spt_output(p,l,q,c,m,"raw",i,phase,j*2+sub) :- spt_result(n,q,c,m,owner,phase,sub,0),
    docof(n,d), doc(d,p,_), line(n,l), spt_scan_doc(d), !spt_needs_facts(d), visit_index(owner,i), visit_index(n,j).
spt_output(p,l,q,c,m,"sorted",0,0,0) :- spt_result(n,q,c,m,_,_,_,_),
    docof(n,d), doc(d,p,_), line(n,l), spt_scan_doc(d), spt_needs_facts(d).
'''
    for code in sorted({code for code, _ in messages.values()}):
        name = code.replace('-', '')
        text += f'.violation State{name}(at: span, qualname: symbol, text: symbol, mode: symbol, visit: number, phase: number, ordinal: number) code "{code}"\n    "{{text}}"\n'
        text += f'State{name}(@place(p,l),q,m,mode,i,phase,j) :- spt_output(p,l,q,"{code}",m,mode,i,phase,j).\n'
    from data_layer_rr_state_helpers import program as helper_program
    from data_layer_rr_state_plugin import program as plugin_program
    return text + helper_contracts(source) + helper_program() + plugin_program(source)


def source_program(text):
    """A supplied source API has no repository discovery exclusions."""
    from data_layer_rr_cli import api_scope
    return api_scope(text)


def helper_contracts(source):
    from data_layer_rr_author import constants
    values = constants(source / ('scripts/lint_' + CHECK + '.py'))
    for node in ast.parse((source / ('scripts/lint_' + CHECK + '.py')).read_text()).body:
        if (isinstance(node,ast.AnnAssign) and isinstance(node.target,ast.Name)
                and isinstance(node.value,ast.Call) and isinstance(node.value.func,ast.Name)
                and node.value.func.id=='frozenset' and not node.value.args and not node.value.keywords):
            values[node.target.id]=frozenset()
    text = '\n.decl spt_planning_name(name: symbol)\n'
    for name in sorted(values['PURE_SCOPE_NAMES']):
        text += f'spt_planning_name({json.dumps(name)}).\n'
    text += '.decl spt_provider_path(path: symbol)\n'
    text += f'spt_provider_path({json.dumps(values["BACKEND_PROVIDER_PURITY_FILE"])}).\n'
    text += '.decl spt_provider_token(token: symbol)\n'
    for token in sorted(values['BACKEND_PROVIDER_PURITY_NAME_TOKENS']):
        text += f'spt_provider_token({json.dumps(token)}).\n'
    text += '.decl spt_native_name(name: symbol)\n'
    for name in sorted(values['FORBIDDEN_NATIVE_PYTHON_CALL_NAMES']):
        text += f'spt_native_name({json.dumps(name)}).\n'
    text += '.decl spt_native_pair(base: symbol, member: symbol)\n'
    for base,member in sorted(values['FORBIDDEN_NATIVE_PYTHON_CALL_CHAINS']):
        text += f'spt_native_pair({json.dumps(base)},{json.dumps(member)}).\n'
    text += '.decl spt_native_method(name: symbol)\n'
    for name in sorted(values['FORBIDDEN_NATIVE_PYTHON_METHOD_NAMES']):
        text += f'spt_native_method({json.dumps(name)}).\n'
    text += '''
.decl spt_chain_last(n: node, name: symbol)
spt_chain_last(n,s) :- spt_callee(n,s).
spt_chain_last(n,s) :- kind(n,"call"), field(n,"function",f), spt_chain_last(f,s).
spt_chain_last(n,s) :- kind(n,"parenthesized_expression"), unparen(n,r), n != r, spt_chain_last(r,s).
.decl spt_chain_previous(n: node, name: symbol)
spt_chain_previous(n,s) :- attr_of(n,o,_), spt_chain_last(o,s).
spt_chain_previous(n,s) :- kind(n,"call"), field(n,"function",f), spt_chain_previous(f,s).
spt_chain_previous(n,s) :- kind(n,"parenthesized_expression"), unparen(n,r), n != r, spt_chain_previous(r,s).
.decl spt_forbidden_native_call(n: node)
spt_forbidden_native_call(n) :- kind(n,"call"), field(n,"function",f), spt_callee(f,name), spt_native_name(name).
spt_forbidden_native_call(n) :- kind(n,"call"), field(n,"function",f), spt_chain_last(f,last),
    spt_chain_previous(f,previous), spt_native_pair(previous,last).
spt_forbidden_native_call(n) :- kind(n,"call"), field(n,"function",f), unparen(f,a),
    attr_of(a,_,raw), spt_native_method(@nfkc(raw)).
.decl spt_scan_token(token: symbol)
'''
    for token in sorted(values['STATE_TRIPWIRE_SCAN_TOKENS']):
        text += f'spt_scan_token({json.dumps(token)}).\n'
    text += '''
.violation StateSourceTrigger(at: span) code "SPT_TRIGGER" "source trigger"
StateSourceTrigger(@place("input.py",1)) :- option("entryPoint","_source_may_have_state_tripwire_hit"),
    option("inputText",s), spt_scan_token(t), contains(t,s).
.violation StateReturnKind(at: span, name: symbol, kind: symbol) code "SPT_RETURN_KIND" "{name}: {kind}"
StateReturnKind(@place("input.py",1),n,k) :- option("entryPoint","_load_generated_return_kinds"), spt_generated(n,k).
.decl spt_work_allow(key: symbol)
'''
    for name in ['SERVERPLAN_NATIVE_PYTHON_WORK_ALLOWLIST', 'SERVERPLAN_PREPROCESS_RUNTIME_VALUE_ALLOWLIST',
                 'SERVERPLAN_RUNTIME_CALLBACK_ALLOWLIST']:
        for key in sorted(values[name]):
            text += f'spt_work_allow({json.dumps(key)}).\n'
    return text + '''
.decl spt_filter_path(i: number, path: symbol)
spt_filter_path(i,p) :- option_item("filterPaths",i,p), !prefix("/",p).
spt_filter_path(i,r) :- option_item("filterPaths",i,p), option("moduleRoot",root), prefix(cat(root,"/"),p),
    r = @sql_substr2(p,@sql_length(root)+2).
spt_filter_path(i,r) :- option_item("filterPaths",i,p), option("moduleRoot",root), prefix("/",p),
    !prefix(cat(root,"/"),p), r = @rx_extract(p,"(?:^|/)((?:impl|data_layer|_internal|l[0-9]{4}_[^/]+)(?:/.*|$))",1).
spt_filter_path(i,p) :- option_item("filterPaths",i,p), option("moduleRoot",root), prefix("/",p),
    !prefix(cat(root,"/"),p), @rx(p,"(?:^|/)(?:impl|data_layer|_internal|l[0-9]{4}_[^/]+)(?:/|$)") = 0.
.decl spt_filter_key(i: number, key: symbol)
spt_filter_key(i,cat(c,"::",q,"::",p)) :- spt_filter_path(i,p), option_item("filterCodes",i,c), option_item("filterQualnames",i,q).
.violation StateFilter(at: span, ordinal: number) code "SPT_FILTER" "retained finding {ordinal}"
StateFilter(@place("input.py",1),i) :- option("entryPoint","_filter_signed_python_work_allowlist"),
    spt_filter_key(i,k), !spt_work_allow(k).
'''
