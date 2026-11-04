# SPDX-License-Identifier: Apache-2.0
"""Legacy callback declarations and recursive receiver-token checks."""
import re
from data_layer_six.common import A,O,N,K,F,H,I,names,call,RX
from data_layer_generate_batch_seven import constant
from data_layer_eight.authority import ANY

def candidate(source,policy):
    module=source/'scripts/lint_legacy_callback_quarantine.py'
    values=lambda name:constant(module,name)
    p=policy('legacy_callback_quarantine',['impl/**/*.py','l*/**/*.py','data_layer/**/*.py','services/**/*.py'])
    p['clauses'].append({'id':'entry','define':{'language':'python','utils':{},'options':{'oracleAnalyzer':{'type':'boolean','default':False},'entryPoint':{'type':'string','default':'scan_repo'}}}})
    utils=p['clauses'][0]['define']['utils']
    def decorators(symbols):
        term=O(names(symbols),A(K('attribute'),F('attribute',names(symbols))),A(K('call'),F('function',{'matches':'callback-decorator-name'})))
        return term
    utils['callback-decorator-name']=O(names(values('LEGACY_MARKERS')),A(K('attribute'),F('attribute',names(values('LEGACY_MARKERS')))),A(K('call'),F('function',{'matches':'callback-decorator-name'})))
    marked=A(K('function_definition'),I(A(K('decorated_definition'),H(A(K('decorator'),H({'matches':'callback-decorator-name'})))),'neighbor'))
    region=O(marked,A(K('decorated_definition'),H(marked)))
    protocol=O(names(['Protocol']),A(K('attribute'),F('attribute',names(['Protocol']))),call(['Protocol']))
    protocol_class=A(K('class_definition'),F('superclasses',H(protocol)))
    explicit={'filePath':{'regex':'^(?:impl/|l|data_layer/|services/)','notRegex':r'(?:^|/)(?:tests|conform)(?:/|$)'}}
    discovered={'filePath':{'notRegex':r'(?:^|/)(?:'+'|'.join(re.escape(v) for v in sorted(values('SKIP_DIRS')))+r')(?:/|$)'}}
    production={'$option':'entryPoint','cases':{'scan_file_content':explicit},'default':discovered}
    def add(id,rule,code):
        p['clauses'].append({'id':id,'forbid':{'ast':{'language':'python','syntax':'valid','rule':A(production,rule)}},'violation':{'code':code,'message':code,'anchor':'witness.span'}})
    def receiver(name,tokens,suffixes):
        token=A(K('identifier'),RX('(?i)^(?:'+'|'.join(re.escape(v) for v in sorted(tokens))+ '|.*(?:'+'|'.join(map(re.escape,suffixes))+'))$'))
        utils[name]=O(token,A(K('attribute'),O(F('attribute',token),F('object',{'matches':name}))),A(K('call'),F('function',{'matches':name})),A(K('subscript'),F('value',{'matches':name})),A(K('parenthesized_expression'),H({'matches':name})))
        return {'matches':name}
    callback_receiver=receiver('callback-receiver',values('CALLBACK_RECEIVER_TOKENS'),values('CALLBACK_RECEIVER_SUFFIXES'))
    conn_receiver=receiver('connection-receiver',values('CONNECTION_RECEIVER_TOKENS'),values('CONNECTION_RECEIVER_SUFFIXES'))
    cbmethods=names(values('CALLBACK_METHODS'));commitmethods=names(values('CONNECTION_COMMIT_METHODS'))
    arguments=F('arguments',H(A(N(K('comment')),N(RX('^[(),]$')))))
    add('callback-call',A(K('call'),F('function',A(K('attribute'),F('attribute',cbmethods),F('object',callback_receiver))),arguments,N(I(region))),'LCB-020')
    add('connection-call',A(K('call'),F('function',A(K('attribute'),F('attribute',commitmethods),F('object',conn_receiver))),N(I(region))),'LCB-031')
    overload=I(A(K('decorated_definition'),H(A(K('decorator'),H(O(names(values('OVERLOAD_DECORATORS')),A(K('attribute'),F('attribute',names(values('OVERLOAD_DECORATORS')))),call(values('OVERLOAD_DECORATORS'))))))),'neighbor')
    parameter=O(names(['fn']),A(O(K('default_parameter'),K('typed_parameter'),K('typed_default_parameter')),H(A(names(['fn']),{'nthChild':1}))))
    add('callback-definition',A(K('function_definition'),F('name',cbmethods),F('parameters',H(parameter)),N(marked),N(overload),N(I(protocol_class))),'LCB-010')
    cls=A(K('class_definition'),F('name',A(K('identifier'),RX('(?:'+'|'.join(map(re.escape,values('CONNECTION_CLASS_SUFFIXES')))+')$'))))
    add('connection-definition',A(K('function_definition'),F('name',commitmethods),I(cls,stop=K('class_definition')),N(I(protocol_class))),'LCB-030')
    return p
