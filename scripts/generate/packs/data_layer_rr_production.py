"""Expose typed production contracts from the same relations as public findings."""
import ast
from dataclasses import fields
import json
import re

from data_layer_rr_text import module, character_class, search

CLAUSES = {
    'removed_compat_surfaces': ['Removed'], 'versioned_lifecycle_legacy_surfaces': ['Lifecycle'],
    'no_api_root_imports': ['RootImport', 'RootExample'],
    'backend_type_strings': ['BackendBT00'+str(i) for i in range(6)],
    'hasattr_async': ['Asynchasattr', 'Asyncgetattr'],
    'no_source_inspection': ['Inspection'+str(i) for i in range(5)],
    'serverplan_name_inference': ['NameInference'+str(i) for i in range(1,5)],
    'no_blanket_grammar_bypass': ['BypassConstant', 'BypassArgument', 'BypassPublic'],
    'serverplan_grammar_surface': ['Surface'], 'permission_error': ['BuiltinPermission'],
}


def arguments(text):
    result, start, depth, quoted, escaped = [], 0, 0, False, False
    for i, c in enumerate(text):
        if quoted:
            if escaped: escaped = False
            elif c == '\\': escaped = True
            elif c == '"': quoted = False
        elif c == '"': quoted = True
        elif c in '([{': depth += 1
        elif c in ')]}': depth -= 1
        elif c == ',' and depth == 0: result.append(text[start:i].strip()); start = i+1
    result.append(text[start:].strip())
    return result


def closing(text, start):
    depth, quoted, escaped = 0, False, False
    for i in range(start, len(text)):
        c = text[i]
        if quoted:
            if escaped: escaped = False
            elif c == '\\': escaped = True
            elif c == '"': quoted = False
        elif c == '"': quoted = True
        elif c == '(': depth += 1
        elif c == ')':
            depth -= 1
            if depth == 0: return i
    raise ValueError('Unclosed generated head')


def interpolate(text, columns):
    pieces = []
    for i, value in enumerate(re.split(r'\{(\w+)\}', text)):
        if i % 2:
            name, kind = columns[value]
            pieces.append(name if kind == 'symbol' else 'to_string('+name+')')
        elif value: pieces.append(json.dumps(value))
    return pieces[0] if len(pieces) == 1 else 'cat('+','.join(pieces)+')'


def production(source, check, text):
    original = module(source, check)
    plugin = getattr(original, check+'_plugin')() if check == 'serverplan_grammar_surface' else getattr(original, check+'_plugin')(source)
    assert len(plugin.required_facts) == 1
    spec = plugin.required_facts[0]
    calls = [n for n in ast.walk(ast.parse((source/('scripts/lint_'+check+'.py')).read_text()))
             if isinstance(n, ast.Call) and isinstance(n.func, ast.Name) and n.func.id == 'StructuredViolation']
    severities = {kw.value.value for call in calls for kw in call.keywords
                  if kw.arg == 'severity' and isinstance(kw.value, ast.Constant)}
    default = next(f.default for f in fields(original.StructuredViolation) if f.name == 'severity')
    assert len(severities) <= 1
    severity = next(iter(severities), default)
    dynamic = check in {'backend_type_strings', 'no_api_root_imports', 'no_blanket_grammar_bypass'}
    if dynamic: roots, scope = [], 'production'
    elif check == 'hasattr_async': roots, scope = [p.rstrip('/') for p in original.DIRECTORIES_TO_CHECK], 'async'
    elif plugin.file_scope.broad_repo_scope: roots, scope = ['.'], 'static'
    else:
        roots = [p.relative_to(source).as_posix() if p.is_absolute() else p.as_posix() for p in plugin.file_scope.directories]
        scope = 'static'
    config = dict(name=check, description=plugin.description, enforcement=plugin.enforcement, findingSeverity=severity,
        extractor=spec.extractor_id, parser=spec.parser_mode, scope=scope,
        include_tests=str(plugin.file_scope.include_tests).lower(),
        fact_kind='surface' if check == 'serverplan_grammar_surface' else 'list',
        layer_regex='l'+character_class(str.isdecimal)+r'{4}_.+(?s:.*)')
    if check == 'permission_error': config['relative_reports'] = 'true'
    text = re.sub(r'^\.description .*$', '.description '+json.dumps(plugin.description), text, flags=re.M)
    # Preserve the root-relative file alias alongside the shared physical parse.
    text = re.sub(r'\bdoc\((\w+),\s*(\w+|_),\s*_\)', r'pp_doc(\1,\2)', text)
    text = re.sub(r'!option\("entryPoint",\s*_\)', 'pp_run()', text)
    text = re.sub(r'(?<!!)option\("entryPoint",\s*_\)', 'pp_api()', text)
    if check == 'no_source_inspection':
        text = re.sub(r'(?m)^inspection_excluded\([^\n]*?\.\n',
                      lambda m: m.group()[:-2]+', pp_api().\n', text)
    selector = {'removed_compat_surfaces':'removed_selected', 'versioned_lifecycle_legacy_surfaces':'lifecycle_selected',
        'no_api_root_imports':'nari_file', 'backend_type_strings':'bts_doc', 'hasattr_async':'async_selected',
        'no_source_inspection':'inspection_selected', 'serverplan_name_inference':'name_selected',
        'no_blanket_grammar_bypass':'bypass_selected', 'serverplan_grammar_surface':'sgp_admitted',
        'permission_error':'permission_selected'}[check]
    pattern = re.compile(r'(?m)^'+selector+r'\([^\n]* :- [\s\S]*?\.\n')
    text = pattern.sub(lambda m: '' if 'pp_run()' in m.group() else m.group(), text)
    text += f'\n{selector}(d) :- pp_run(), pp_doc(d,_), pp_fact_present(d).\n'
    # Filename decisions belong to the alias, not to the canonical parse path.
    if check == 'serverplan_name_inference':
        text = text.replace('file_name(d, "sql_column_type_hints.py")', 'suffix("/sql_column_type_hints.py",cat("/",p))')
    suffix = '\n#include "data-layer/plugin-production.rr"\n'
    suffix += ''.join(f'pp_config({json.dumps(k)},{json.dumps(v)}).\n' for k,v in config.items())
    suffix += ''.join(f'pp_static_root({json.dumps(p)}).\n' for p in roots)
    suffix += ''.join(f'pp_trigger({json.dumps(p)}).\n' for p in spec.source_triggers)
    suffix += ''.join(f'pp_trigger_group({i},{json.dumps(p)}).\n' for i,g in enumerate(spec.source_trigger_groups) for p in g)
    suffix += '.decl pp_fact_allowed(path: symbol)\n.decl pp_report_allowed(path: symbol)\n'
    suffix += 'pp_fact_allowed(p) :- pp_file(p), !pp_fact_excluded(p).\n.decl pp_fact_excluded(path: symbol)\n'
    if check == 'backend_type_strings':
        suffix += 'pp_fact_excluded(p) :- pp_absolute(p,full), bts_config("SKIP_PATTERNS",_,part), contains(part,full).\n'
    elif check == 'no_source_inspection':
        suffix += 'pp_fact_excluded(p) :- pp_absolute(p,full), inspection_skip(part), contains(cat("/",part,"/"),cat(full,"/")).\n'
    suffix += 'pp_report_allowed(p) :- pp_file(p), !pp_report_excluded(p).\n.decl pp_report_excluded(path: symbol)\n'
    if check == 'hasattr_async':
        for pattern in original.EXCLUDE_PATTERNS:
            suffix += 'pp_report_excluded(p) :- pp_absolute(p,full), match('+json.dumps(search(re.compile(pattern)))+',full).\n'
    declarations = re.compile(r'(?ms)^\.violation (\w+)\((.*?)\) code (\{\w+\}|"(?:\\.|[^"\\])*")\s+("(?:\\.|[^"\\])*")')
    found = {m.group(1): m for m in declarations.finditer(text)}
    fact_decl = None
    for clause in CLAUSES[check]:
        declaration = found[clause]; cols = [tuple(s.strip() for s in p.split(':')) for p in arguments(declaration.group(2))]
        node = cols[0][1] == 'node'; tail = cols[1:]
        events = []
        for match in list(re.finditer(r'(?m)^'+clause+r'\(', text)):
            start = match.start(); end = closing(text, match.end()-1); stop = text.index('.\n', end)+2
            terms = arguments(text[match.end():end]); body = text[end+1:stop].strip()
            assert body.startswith(':-'); body = body[2:].strip()[:-1]
            if node:
                path, line = 'ppe_path', 'ppe_line'
                body += f', docof({terms[0]},ppe_doc), pp_doc(ppe_doc,ppe_path), line({terms[0]},ppe_line)'
            else:
                assert terms[0].startswith('@place('), clause
                path, line = arguments(terms[0][7:-1])
            names = {k: t for (k,_),t in zip(tail,terms[1:])}
            if check == 'removed_compat_surfaces': order = [names['ordinal'],'0','0']
            elif check == 'versioned_lifecycle_legacy_surfaces':
                body += f', visit_index(c,ppe_class_order), visit_index({terms[0]},ppe_method_order)'
                order = ['ppe_class_order','ppe_method_order','0']
            elif check == 'no_api_root_imports': order = [names['ordinal'], '-1' if clause == 'RootImport' else names['piece']+'*2+'+names['pattern'], names['occurrence']]
            elif check == 'backend_type_strings': order = [line,names['pattern'],names['occurrence']]
            elif check == 'hasattr_async': order = [line,'0' if clause == 'Asynchasattr' else '1','0']
            elif check == 'no_source_inspection': order = [line,names['ordinal'],'0']
            elif check == 'serverplan_name_inference': order = [line,str(int(clause[-1])-1),'0']
            elif check == 'no_blanket_grammar_bypass':
                if node:
                    body += f', visit_index({terms[0]},ppe_method_order)'; order = ['1','ppe_method_order','0']
                else: order = ['0',line,names['ordinal']]
            elif check == 'serverplan_grammar_surface': order = [names['ordinal'],names['witness'],'0']
            else:
                body += f', visit_index({terms[0]},ppe_raise_order)'; order = ['ppe_raise_order','0','0']
            relation = 'ppe_'+clause
            schema = [('path','symbol'),('line','number'),('event0','number'),('event1','number'),('event2','number')]+tail
            if node: schema += [('anchor','node')]
            if not events: suffix += '.decl '+relation+'('+', '.join(k+': '+t for k,t in schema)+')\n'
            values = [path,line,*order,*terms[1:]] + ([terms[0]] if node else [])
            events.append((start,stop,relation+'('+','.join(values)+') :- '+body+'.\n'))
        assert events, clause
        for start, stop, replacement in reversed(events): text = text[:start]+replacement+text[stop:]
        vars = {k:(k,t) for k,t in tail}
        code = declaration.group(3)
        code = vars[code[1:-1]][0] if code.startswith('{') else code
        message = interpolate(json.loads(declaration.group(4)),vars)
        event = 'ppe_'+clause+'('+','.join(['p','l','o0','o1','o2',*[k for k,_ in tail]]+(['anchor'] if node else []))+')'
        # Keep the original anchor for APIs. Three ordinal columns retain duplicates at placed production spans.
        newcols = cols + [('collector_order0','number'),('collector_order1','number'),('collector_order2','number')]
        newdecl = '.violation '+clause+'('+', '.join(k+': '+t for k,t in newcols)+') code '+declaration.group(3)+' '+declaration.group(4)
        text = text.replace(declaration.group(),newdecl,1)
        api_anchor = 'anchor' if node else '@place(p,l)'
        suffix += clause+'('+','.join([api_anchor,*[k for k,_ in tail],'o0','o1','o2'])+') :- '+event+', pp_api().\n'
        suffix += clause+'('+','.join(['@place(p,l)',*[k for k,_ in tail],'o0','o1','o2'])+') :- '+event+', pp_run(), pp_fact_allowed(p), pp_report_allowed(p).\n'
        pkey = '@replace_all(p,"/","\\u0000")' if check in {'hasattr_async','serverplan_name_inference'} else 'p'
        sort = [pkey,'l',message,'0']
        if check == 'backend_type_strings': sort = ['p','l',code,'0']
        elif check in {'removed_compat_surfaces','serverplan_grammar_surface'}: sort = ['p','o0','""','o1']
        elif check in {'hasattr_async','serverplan_name_inference'}: sort = [pkey,'o0','""','o1']
        elif check == 'permission_error': sort = ['p','l','""','o0']
        suffix += 'pp_report('+','.join(['p','l',code,message,*sort,'o0','o1','o2'])+') :- '+event+', pp_run(), pp_fact_allowed(p), pp_report_allowed(p).\n'
        if check == 'removed_compat_surfaces': payload = [('path','p','symbol'),('line','l','number'),('name','name','symbol')]
        elif check == 'versioned_lifecycle_legacy_surfaces': payload = [('file_path','p','symbol'),('line','l','number'),('class_name','cls','symbol'),('method_name','method','symbol'),('message',message,'symbol')]
        elif check == 'no_api_root_imports': payload = [('file_path','full','symbol'),('line','l','number'),('message',message,'symbol')]
        elif check == 'backend_type_strings': payload = [('file','full','symbol'),('line','l','number'),('column','column','number'),('message',message,'symbol'),('code',code,'symbol'),('snippet','snippet','symbol')]
        elif check == 'hasattr_async': payload = [('line','l','number'),('pattern_type',code,'symbol'),('method_name','method','symbol')]
        elif check == 'no_source_inspection': payload = [('file_path','full','symbol'),('line','l','number'),('line_content','snippet','symbol'),('pattern_name','label','symbol')]
        elif check == 'serverplan_name_inference': payload = [('line','l','number'),('code',code,'symbol'),('message',message,'symbol')]
        elif check == 'no_blanket_grammar_bypass': payload = [('file_path','p','symbol'),('line','l','number'),('message',message,'symbol')]
        elif check == 'serverplan_grammar_surface': payload = [('kind','kind','symbol'),('line','l','number'),('symbol','name','symbol'),('message',message,'symbol')]
        else: payload = [('path','p','symbol'),('line','l','number'),('message',message,'symbol')]
        fact_schema = [('at','span'),('path','symbol'),('path_order','symbol'),('order0','number'),('order1','number'),('order2','number')]+[('f_'+k,t) for k,_,t in payload]
        decl = '.violation ProductionFact('+', '.join(k+': '+t for k,t in fact_schema)+') code "DL_FACT" "production fact"\n'
        if fact_decl is None: suffix += decl; fact_decl = decl
        else: assert fact_decl == decl
        suffix += 'ProductionFact('+','.join(['@place(p,l)','p','@replace_all(p,"/","\\u0000")','o0','o1','o2',*[v for _,v,_ in payload]])+') :- '+event+', option("entryPoint","_production_contract"), pp_fact_allowed(p), pp_absolute(p,full).\n'
    from data_layer_rr_supplied_facts import supplied_facts
    return text+suffix+supplied_facts(original, check)
