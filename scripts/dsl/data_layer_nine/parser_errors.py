# SPDX-License-Identifier: Apache-2.0
"""Parser facts execute in the native batch; SQL emits production diagnostics."""
from .common import *


def clause_(code,prefix,gate):
 invalid=E(gate,{'path':'file.path'})
 invalid['evidence']['query']['ast']['syntax']='invalid'
 return {'id':'reported-parser-error','relational':{
  'inputs':{'invalid':invalid},
  'requireNonemptyInput':'invalid',
  'stages':[{'name':'parsed','pythonAst':{'source':'invalid','roots':'SELECT __path,source_text(__path,0,__end) AS source FROM invalid','fields':['syntax_error']}}],
  'query':"SELECT i.__path path,0 start_byte,0 end_byte,1 line,"+q(code)+" issue,"+q(prefix)+"||json_extract(p.syntax_error,'$.msg')||CASE WHEN json_extract(p.syntax_error,'$.lineno') IS NULL THEN '' ELSE ' ('||rx_replace(i.__path,'^.*/','')||', line '||json_extract(p.syntax_error,'$.lineno')||')' END message,i.__path identity,0 phase,0 ordering,'' symbol,'' sort_name FROM invalid i JOIN parsed p ON p.__path=i.__path WHERE p.syntax_error!='null'",
  'key':['identity'],'code':'issue','message':'message'},'violation':{'code':code,'message':'Parser error','anchor':'witness.span'}}
