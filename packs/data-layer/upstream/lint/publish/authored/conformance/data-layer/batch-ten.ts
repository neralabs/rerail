/** Replay complete batch-ten observations through the native execution APIs. */
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { gunzipSync } from "node:zlib";
import { codeRuleSchema, loadNativeBinding, RESULT_PAGE_BYTES, type NativeCompiledCodeRuleDsl } from "../../src/engine/index.js";
import { capabilitiesJson } from "../engine.js";
import { replayExecution } from "./execution.js";
import type { NineOracle, NineRecord } from "./batch-nine.js";
export type TenOracle = NineOracle;
export type TenRecord = NineRecord;
export function readTenOracle(path:string):TenOracle{
  const oracle=JSON.parse((path.endsWith(".gz")?gunzipSync(readFileSync(path)):readFileSync(path)).toString("utf8")) as TenOracle;
  // check_file embeds pytest's changing temporary root in its string result.
  for(const record of oracle.records)if(record.check==="generated_index_sources"&&Array.isArray(record.expected))record.expected=record.expected.map(value=>typeof value==="string"?value.replace(/\/[^ ]*\/pytest-[0-9]+\/[^/]+\//g,""):value);
  return oracle;
}
type Row = Record<string, string | number | null>;
type Finding = { code: string; message: string; path: string; span: { startLine: number }; variables: { row: Row } };
const compare = (left: unknown, right: unknown): number => typeof left === "number" && typeof right === "number" ? left - right : Buffer.compare(Buffer.from(String(left)), Buffer.from(String(right)));
export const canonical = (values: unknown): unknown => Array.isArray(values) ? [...values].sort((a,b) => compare(JSON.stringify(a),JSON.stringify(b))) : values;
function sqlChoice(value:unknown,rewrite:(sql:string)=>string):unknown{
 if(typeof value==="string")return rewrite(value);
 const choice=value as {cases:Record<string,unknown>;default:unknown};
 return {...choice,cases:Object.fromEntries(Object.entries(choice.cases).map(([key,sql])=>[key,sqlChoice(sql,rewrite)])),default:sqlChoice(choice.default,rewrite)};
}
export function observedCode(record:TenRecord,finding:unknown):string{
 if(typeof finding==="string")return "missing_source";
 const value=finding as {code?:string;rule?:string};
 return value.code??value.rule??(record.check==="conditional_architecture"?"CONDA-005":"SIDP001");
}
export function positiveFamilies(oracle:TenOracle):{check:string;code:string;record:TenRecord}[]{
 const families=new Map<string,{check:string;code:string;record:TenRecord}>();
 for(const record of oracle.records)if(Array.isArray(record.expected))for(const finding of record.expected){
  const code=observedCode(record,finding);
  const key=record.check+":"+code;if(!families.has(key))families.set(key,{check:record.check,code,record});
 }
 return [...families.values()];
}
export function tenResult(record: TenRecord, findings: Finding[]): unknown {
  const rows = findings.map(f => f.variables.row);
  const order = (r: Row) => record.check === "conditional_architecture" ? record.entry==="conditional_architecture_findings"?[r.path,r.line,r.message]:[r.issue,r.path,String(r.line),r.message]
    : record.check === "matching_architecture" ? record.entry==="_scan_tree"?[r.phase,r.sort_name,r.ordering,r.issue]:[r.path,r.line,r.issue,...(record.entry==="lint"?[r.symbol]:[]),r.phase,r.sort_name,r.ordering]
    : ["hnsw_architecture","bounded_composed_graph_paths"].includes(record.check) ? [r.path,r.line,r.issue,r.message]
    : record.check === "plan_optimizer_v2" && record.entry === "find_violations" ? [r.path,r.line,r.issue]
    : [r.phase ?? 0,r.sort_name ?? "",r.ordering ?? 0,r.issue];
  rows.sort((a,b) => { const l=order(a),r=order(b); for(let i=0;i<l.length;i++){const n=compare(l[i],r[i]);if(n)return n;} return 0; });
  return rows.map(row => {
    const { path,line,issue:code,message }=row;
    switch(record.check){
      case "generated_index_sources":return `${path}:${line}: ${message}`;
      case "graph_lazy_sql_plan":return {code,file_path:path,line_number:line,message};
      case "graphql_plan_ownership":return {linter_name:record.check,code,path,line,message,severity:"error"};
      case "schema_introspection_data_paths":return {path,line,call_name:row.call_name,message};
      case "embedding_vector_orchestration":{
        const file=record.files.find(f=>f.path===path)!;
        const source=file.source;
        if(record.entry==="scan_source" && source===undefined)throw new Error("Missing decoded scan source");
        const digest=createHash("sha256").update(String(path)).update("\0").update(source??"").digest("hex").slice(0,16);
        return {file_path:record.entry==="scan_source"?`tmp/embedding_vector_orchestration_scan/${digest}/${path}`:path,line,qualname:row.qualname,code,message};
      }
      case "conditional_architecture":return record.entry==="conditional_architecture_findings"?{path,line,message}:{rule:code,file:path,line,message};
      case "matching_architecture":return {code,path,line,symbol:row.symbol,message};
      case "plan_optimizer_v2":return {file_path:path,line,code,message};
      default:return {code,path,line,message};
    }
  });
}
export class TenReplay {
  private readonly compiled=new Map<string,NativeCompiledCodeRuleDsl>();
  private readonly execution=replayExecution();
  constructor(private readonly directory="packs/data-layer/rules",private readonly mutant:boolean|string=false){}
  close():void{this.execution.close();}
  async run(oracle:TenOracle,record:TenRecord,mode:"snapshot"|"workspace",production=false,selectionOnly=false):Promise<unknown>{
    const authored=JSON.parse(readFileSync(`${this.directory}/data-layer-${record.check.replaceAll("_","-")}.json`,"utf8"));
    if(selectionOnly){
      if(record.check!=="matching_architecture"||!production)throw new Error("Selection probe requires matching production defaults");
      const stages=authored.clauses.find((c:{id:string})=>c.id==="matching-file-checks").relational.stages;
      const parser=stages.find((s:{name:string})=>s.name==="canonical").pythonAst;
      const roots=parser.roots;
      // Observe the actual SQL source predicate before AST validity removes files.
      const inputs={files:{evidence:{as:"file",query:{regex:{source:"file.content",pattern:"(?s)\\A(?P<TEXT>.*)\\z"}},project:{path:"file.path"}}}};
      const probe=[stages.find((s:{name:string})=>s.name==="matching_parameters"),...(parser.syntax?[{name:"cst",query:"SELECT __path,0 id,'module' kind FROM files"}]:[])];
      authored.clauses=authored.clauses.filter((c:{define?:unknown})=>c.define!==undefined).concat([{id:"selection-observation",relational:{inputs,stages:probe,query:sqlChoice(roots,sql=>`SELECT DISTINCT __path path,0 start_byte,0 end_byte FROM (${sql})`)},violation:{code:"SELECTION",message:"Selected source",anchor:"witness.span"}}]);
    }
    if(!production)for(const clause of authored.clauses)if(clause.define?.options!==undefined)clause.define.config={...clause.define.config,entryPoint:record.entry,parameters:JSON.stringify(record.parameters)};
    if(this.mutant===true)authored.clauses=authored.clauses.filter((c:{define?:unknown})=>c.define!==undefined).concat([{id:"disabled",forbid:{regex:{source:"file.content",pattern:"\\b\\B"}},violation:{code:"CONTROL",message:"Disabled",anchor:"witness.span"}}]);
    if(typeof this.mutant==="string"){
      const literal="'"+this.mutant.replaceAll("'","''")+"'";
      for(const clause of authored.clauses)if(clause.relational)clause.relational.query=sqlChoice(clause.relational.query,sql=>`SELECT * FROM (${sql}) WHERE issue != ${literal}`);
    }
    if(!production)authored.scope={include:["**/*"],exclude:[]};
    const program=JSON.stringify(codeRuleSchema.parse(authored));let rule=this.compiled.get(program);
    if(rule===undefined){rule=loadNativeBinding().CodeRuleDslRuntime.compile(program,"json",capabilitiesJson());this.compiled.set(program,rule);}
    const files=record.files.map(f=>({path:f.path,language:f.path.endsWith(".py")?"python":"text",source:f.source??oracle.sources[f.sourceSha256]!}));
    let result:{violations:Finding[];failures:unknown[]};
    if(mode==="snapshot")result=JSON.parse(this.execution.evaluate(rule,JSON.stringify({format:"rerail.repository-snapshot/v1",revision:0,files,facts:[]}),"report"));
    else{
      const workspace=this.execution.createWorkspace();result={violations:[],failures:[]};
      try{
        workspace.addCodeRule(rule);await workspace.applyAsync(JSON.stringify({revision:1,files,removed:[],quiet:true}));
        for(let cursor=0;;){const page=JSON.parse(await workspace.resultPageAsync(cursor,RESULT_PAGE_BYTES));result.violations.push(...page.violations);result.failures.push(...page.failures);if(page.done)break;if(page.cursor<=cursor)throw new Error("Result cursor did not advance");cursor=page.cursor;}
      }finally{workspace.close();}
    }
    if(result.failures.length)throw new Error(`${record.test}: ${JSON.stringify(result.failures)}`);
    if(selectionOnly)return [...new Set(result.violations.map(f=>f.path))].sort();
    for(const finding of result.violations){const row=finding.variables.row;
      if(finding.code!==row.issue||finding.message!==row.message||finding.path!==row.path||finding.span.startLine+1!==row.line)throw new Error(`Public diagnostic differs from DTO: ${JSON.stringify(finding)}`);
    }
    if(production)return canonical(result.violations.map(f=>({path:f.path,line:f.span.startLine+1,code:f.code,message:f.message})));
    const decoded={...record,files:record.files.map(f=>({...f,source:f.source??oracle.sources[f.sourceSha256]!}))};
    const value=tenResult(decoded,result.violations);return record.check==="graph_lazy_sql_plan"?canonical(value):value;
  }
}
