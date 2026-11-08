"""Source-derived reporting cases retain ordering, formatting, errors, and duplicate occurrences."""
import copy


CATEGORIES=['unused_functions','unused_intents','unused_grammar_classes','unused_other_classes',
            'high_confidence_functions','high_confidence_intents','high_confidence_grammar_classes','high_confidence_other_classes']
SYMBOLS=[dict(sid='b::one',path='z.py',line=7,qualname='Outer.inner'),
         dict(sid='a::two',path='a.py',line=2,qualname='Different.from.sid'),
         dict(sid='c::three',path='z.py',line=9),
         dict(sid='uni::emoji',path='é/😀.py',line=-3,qualname='C.é😀'),
         dict(sid='quoted::name',path='a"b\\c\t\n.py',line=0,qualname='q"\\\n\t\f')]
RESULTS={name:[] for name in CATEGORIES}
RESULTS.update(unused_functions=['b::one','a::two','b::one','uni::emoji','quoted::name'],
               unused_intents=['c::three'],unused_grammar_classes=['c::three'],
               high_confidence_functions=['a::two','b::one','b::one','c::three','uni::emoji'],
               high_confidence_other_classes=['quoted::name'])


def case(name,entry,results=None,symbols=None,**arguments):
    values=dict(graph=dict(symbols=copy.deepcopy(SYMBOLS if symbols is None else symbols)),**arguments)
    if results is not None:values['results']=copy.deepcopy(results)
    return dict(id='survey-report-'+name,check='internal_dead_code',entry='graph_'+entry,files=[],arguments=values)


def cases():
    rows=[]
    for limit in [0,1,2,100,-1,-2,-100,None,2**100,-(2**100)]:
        rows.append(case('text-limit-'+str(limit),'print_text',RESULTS,limit=limit))
    rows.append(case('text-empty','print_text',{name:[] for name in CATEGORIES},limit=10))
    rows.append(case('text-reversed-categories','print_text',dict(reversed(list(RESULTS.items()))),limit=2))
    for name,results in [('normal',RESULTS),('reversed',dict(reversed(list(RESULTS.items())))),
                         ('empty',{}),('empty-categories',{name:[] for name in CATEGORIES}),
                         ('arbitrary-keys',{'é😀"\n':['uni::emoji','quoted::name'],'empty':[]}),
                         ('summary-collision',{'unused_functions':['a::two'],'summary':['b::one','b::one']})]:
        rows.append(case('json-'+name,'print_json',results))
    for limit in [0,1,2,20,-1,None,2**100,-(2**100)]:
        rows.append(case('worst-limit-'+str(limit),'worst_files',symbol_ids=RESULTS['high_confidence_functions'],limit=limit))
    rows.append(case('worst-empty','worst_files',symbol_ids=[]))
    many=[dict(sid=f'm{i}::f',path=f'p{30-i:02}.py') for i in range(26)]
    ids=[s['sid'] for s in many]
    rows.append(case('worst-default-tie-limit','worst_files',symbols=many,symbol_ids=ids))
    many_results={name:[] for name in CATEGORIES};many_results['high_confidence_functions']=ids
    rows.append(case('text-worst-tie-limit','print_text',many_results,symbols=many,limit=1))
    rows.append(case('worst-first-missing','worst_files',symbol_ids=['a::two','missing::first','missing::second']))
    for name,category,limit in [('worst','high_confidence_functions',1),('detail','unused_intents',100),('sliced-away','unused_intents',0)]:
        results=copy.deepcopy(RESULTS);results[category]=['a::two','missing::first','missing::second']
        rows.append(case('text-missing-symbol-'+name,'print_text',results,limit=limit))
    for missing in ['unused_functions','unused_grammar_classes','high_confidence_functions','high_confidence_other_classes']:
        results=copy.deepcopy(RESULTS);del results[missing]
        rows.append(case('text-missing-category-'+missing,'print_text',results,limit=3))
    rows.append(case('json-first-missing','print_json',{'earlier':['a::two','missing::first'],'later':['missing::second']}))
    return rows


CASES=cases()
