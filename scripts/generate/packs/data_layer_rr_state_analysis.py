"""Select State Tripwires function contexts before rich inference."""
import re


def replace_rules(program, name, replacement):
    pattern=rf'^{name}\(.*?(?=\.\s*(?:\n|$))\.\s*\n'
    result,count=re.subn(pattern,'',program,flags=re.M|re.S)
    if not count:
        raise ValueError('Missing source relation: '+name)
    return result+'\n'+replacement+'\n'


def scoped(program):
    # Source collection visits only module bodies and nested class bodies.
    # Definitions beneath a function or control statement are not contexts.
    program=replace_rules(program,'contexts','''
.decl source_scope(body: node, classes: symbol)
source_scope(m,"") :- gnode(m), kind(m,"module").
source_scope(b,@nfkc(name)) :- source_scope(parent,""), body_member(parent,c), kind(c,"class_definition"),
    def_name(c,name), field(c,"body",b).
source_scope(b,cat(prefix,".",@nfkc(name))) :- source_scope(parent,prefix), prefix != "", body_member(parent,c),
    kind(c,"class_definition"), def_name(c,name), field(c,"body",b).
contexts(f,@nfkc(name),classes) :- source_scope(body,classes), body_member(body,f), kind(f,"function_definition"), def_name(f,name).
''')
    program=replace_rules(program,'ctx_planning','''
ctx_planning(f) :- contexts(f,n,_), @rx(n,"_lazy$") = 1.
ctx_planning(f) :- contexts(f,n,_), planning_name(n).
ctx_planning(f) :- contexts(f,"pending_pipeline_effects",_).
ctx_planning(f) :- contexts(f,n,_), prefix("apply_",n), suffix("_effect",n).
ctx_planning(f) :- contexts(f,_,_), decorator(f,e), short_name(e,name), @nfkc(name) = "pure_lazy".
''')
    program=replace_rules(program,'ctx_helper','''
ctx_helper(f) :- contexts(f,name,_), suffix("sql_builder",name), !planning_name(name),
    parameters(f,p,_,_), field(p,"type",ann), walk_name(ann,raw), @nfkc(raw) = "CompilationContext".
''')
    program=replace_rules(program,'ctx_materializer','''
ctx_materializer(f) :- contexts(f,name,_), parameters(f,_,"runtime_refs",_), prefix("materialize_",name), suffix("builder",name).
ctx_materializer(f) :- contexts(f,_,_), parameters(f,_,"runtime_refs",_), parameters(f,p,_,_),
    field(p,"type",ann), walk_name(ann,raw), @nfkc(raw) = "RuntimeBuilderSpec".
''')
    # Function ast.walk includes the function's own decorators.
    program+='''
body_nodes(f,e) :- contexts(f,_,_), decorator(f,e).
body_nodes(f,n) :- contexts(f,_,_), decorator(f,e), within(n,e).
'''
    program=replace_rules(program,'reachable','''
.decl native_work_context(f: node)
native_work_context(f) :- body_nodes(f,c), spt_forbidden_native_call(c).
reachable(f) :- ctx_planning(f).
reachable(f) :- ctx_helper(f).
reachable(f) :- ctx_provider_helper(f).
reachable(f) :- ctx_materializer(f).
// scan_tree already collected native-work calls during its visitor, on any path.
reachable(f) :- native_work_context(f).
// Selection is by called leaf name. Resolution for findings remains class-aware.
reachable(t) :- reachable(f), body_nodes(f,c), kind(c,"call"), field(c,"function",callee), spt_callee(callee,name),
    docof(f,d), contexts(t,name,_), docof(t,d).
.decl fact_context(f: node)
fact_context(f) :- contexts(f,_,_), reachable(f), docof(f,d), spt_needs_facts(d).
.decl runtime_context(f: node)
runtime_context(f) :- fact_context(f).
''')
    # Native-work identification is structural, including parentheses and
    # whitespace around attributes; textual callee regexes are not equivalent.
    pattern=r'^direct_events\(f, n, "native_python_work", 1\) :- .*?(?=\.\s*(?:\n|$))\.\s*\n'
    program,count=re.subn(pattern,'',program,flags=re.M|re.S)
    if count!=2:
        raise ValueError('Expected the two native-work event branches')
    program+='direct_events(f,n,"native_python_work",1) :- body_nodes(f,n), spt_forbidden_native_call(n).\n'
    # These relations form the rich fact producer. Scope its input before
    # building parameter kinds, taints, sink classifications, or return inference.
    for head,variable,scope in [('parameter_kinds','f','fact_context'),('taints','f','fact_context'),
                                ('direct_events','f','fact_context'),('special_attributes','f','fact_context'),
                                ('runtime','c','runtime_context')]:
        pattern=rf'^{head}\((.*?)(?=\.\s*(?:\n|$))\.'
        def guard(match):
            return match.group(0)[:-1]+f', {scope}({variable}).'
        program,count=re.subn(pattern,guard,program,flags=re.M|re.S)
        if not count:
            raise ValueError('Missing rich inference relation: '+head)
    # Local return annotations belong to the selected function list too.
    program=re.sub(r'^(return_kinds\(.*?contexts\(c,.*?)(?=\.\s*(?:\n|$))\.',
                   lambda m:m.group(0)[:-1]+', fact_context(c).',program,flags=re.M|re.S)
    return program
