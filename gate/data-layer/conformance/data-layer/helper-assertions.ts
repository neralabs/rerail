/** Native replay of original helper results; Python AST objects are represented by canonical node IDs. */
import {readFileSync} from "node:fs";
import {codeRuleSchema,loadNativeBinding,RESULT_PAGE_BYTES,type NativeCompiledCodeRuleDsl} from "../../src/engine/index.js";
import {capabilitiesJson} from "../engine.js";
import {replayExecution} from "./execution.js";

export type HelperRecord={test:string;check:string;entry:string;files:{path:string;source:string}[];parameters:Record<string,unknown>;expected:unknown};
const contracts=JSON.parse(readFileSync("fixtures/host/data-layer-port/batch-10-helper-contracts.json","utf8"));
const booleans=new Set(["_function_mentions_attr","_function_mentions_name","_function_mentions_any_name","_class_has_assignment","_is_data_path","_tree_uses_matching_contracts"]);
export const helperContract=(entry:string):string=>"batch-ten-helper-"+(({ _parse_source:"parse",_is_data_path:"path",_approval_entry_errors:"approvals",_tree_uses_matching_contracts:"matching"} as Record<string,string>)[entry]??"ast");
type Row={node_id?:number;helper_value:string|number;identity:string|number};
export class HelperReplay{
 private readonly execution=replayExecution();
 private readonly compiled=new Map<string,NativeCompiledCodeRuleDsl>();
 close():void{this.execution.close();}
 async run(record:HelperRecord,mode:"snapshot"|"workspace",disabled=false):Promise<unknown>{
  const document=structuredClone(contracts[helperContract(record.entry)]);
  for(const clause of document.clauses)if(clause.define?.options)clause.define.config={...clause.define.config,entryPoint:record.entry,parameters:JSON.stringify(record.parameters)};
  if(disabled)document.clauses=document.clauses.filter((clause:{define?:unknown})=>clause.define).concat([{id:"disabled",forbid:{regex:{source:"file.content",pattern:"\\b\\B"}},violation:{code:"CONTROL",message:"Disabled",anchor:"witness.span"}}]);
  const key=JSON.stringify(codeRuleSchema.parse(document));let rule=this.compiled.get(key);
  if(!rule){rule=loadNativeBinding().CodeRuleDslRuntime.compile(key,"json",capabilitiesJson());this.compiled.set(key,rule);}
  const files=record.files.map(file=>({...file,language:"python"}));
  let result:{violations:{variables:{row:Row}}[];failures:unknown[]};
  if(mode==="snapshot")result=JSON.parse(this.execution.evaluate(rule,JSON.stringify({format:"rerail.repository-snapshot/v1",revision:0,files,facts:[]}),"report"));
  else{
   const workspace=this.execution.createWorkspace();result={violations:[],failures:[]};
   try{
    workspace.addCodeRule(rule);await workspace.applyAsync(JSON.stringify({revision:1,files,removed:[],quiet:true}));
    for(let cursor=0;;){const page=JSON.parse(await workspace.resultPageAsync(cursor,RESULT_PAGE_BYTES));result.violations.push(...page.violations);result.failures.push(...page.failures);if(page.done)break;if(page.cursor<=cursor)throw new Error("Native helper cursor did not advance");cursor=page.cursor;}
   }finally{workspace.close();}
  }
  if(result.failures.length)throw new Error(JSON.stringify(result.failures));
  const rows=result.violations.map(finding=>finding.variables.row);
  if(record.entry==="_parse_source"){
   if(rows.length!==1)throw new Error("Native syntax contract did not publish its result");
   const error=JSON.parse(String(rows[0]!.helper_value));
   return error===null?{module:true}:{exception:{type:"SyntaxError",message:`${error.msg} (<unknown>, line ${error.lineno})`,...error}};
  }
  if(booleans.has(record.entry)){
   if(rows.length!==1||typeof rows[0]!.helper_value!=="number"||![0,1].includes(rows[0]!.helper_value))throw new Error("Native Boolean contract did not publish its result");
   return Boolean(rows[0]!.helper_value);
  }
  if(record.entry==="_decorator_names")return rows.map(row=>String(row.helper_value)).sort((a,b)=>Buffer.compare(Buffer.from(a),Buffer.from(b)));
  if(record.entry==="_approval_entry_errors")return rows.sort((a,b)=>Number(a.identity)-Number(b.identity)).map(row=>row.helper_value);
  rows.sort((a,b)=>a.node_id!-b.node_id!);
  if(record.entry==="_non_docstring_stmts")return rows.map(row=>row.node_id);
  if(rows.length>1)throw new Error("Native selection returned multiple nodes");
  return rows.length?rows[0]!.node_id:null;
 }
}
