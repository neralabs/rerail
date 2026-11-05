/** Replay complete batch-ten observations through the native execution APIs. */
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { codeRuleSchema, loadNativeBinding, RESULT_PAGE_BYTES, type NativeCompiledCodeRuleDsl } from "../../src/engine/index.js";
import { capabilitiesJson } from "../engine.js";
import { replayExecution } from "./execution.js";
import type { NineOracle, NineRecord } from "./batch-nine.js";
export type TenOracle = NineOracle;
export type TenRecord = NineRecord;
type Row = Record<string, string | number | null>;
type Finding = { code: string; message: string; path: string; span: { startLine: number }; variables: { row: Row } };
const compare = (left: unknown, right: unknown): number => typeof left === "number" && typeof right === "number" ? left - right : Buffer.compare(Buffer.from(String(left)), Buffer.from(String(right)));
export const canonical = (values: unknown): unknown => Array.isArray(values) ? [...values].sort((a,b) => compare(JSON.stringify(a),JSON.stringify(b))) : values;
export function tenResult(record: TenRecord, findings: Finding[]): unknown {
  const rows = findings.map(f => f.variables.row);
  const order = (r: Row) => record.check === "conditional_architecture" ? [r.issue,r.path,r.line,r.message]
    : ["hnsw_architecture","bounded_composed_graph_paths","matching_architecture"].includes(record.check) ? [r.path,r.line,r.issue,r.message]
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
      case "conditional_architecture":return {rule:code,file:path,line,message};
      case "matching_architecture":return {code,path,line,symbol:row.symbol,message};
      case "plan_optimizer_v2":return {file_path:path,line,code,message};
      default:return {code,path,line,message};
    }
  });
}
export class TenReplay {
  private readonly compiled=new Map<string,NativeCompiledCodeRuleDsl>();
  private readonly execution=replayExecution();
  constructor(private readonly directory="packs/data-layer/rules",private readonly mutant=false){}
  close():void{this.execution.close();}
  async run(oracle:TenOracle,record:TenRecord,mode:"snapshot"|"workspace",production=false):Promise<unknown>{
    const authored=JSON.parse(readFileSync(`${this.directory}/data-layer-${record.check.replaceAll("_","-")}.json`,"utf8"));
    if(!production)for(const clause of authored.clauses)if(clause.define?.options!==undefined)clause.define.config={...clause.define.config,entryPoint:record.entry,parameters:JSON.stringify(record.parameters)};
    if(this.mutant)authored.clauses=authored.clauses.filter((c:{define?:unknown})=>c.define!==undefined).concat([{id:"disabled",forbid:{regex:{source:"file.content",pattern:"\\b\\B"}},violation:{code:"CONTROL",message:"Disabled",anchor:"witness.span"}}]);
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
    for(const finding of result.violations){const row=finding.variables.row;
      if(finding.code!==row.issue||finding.message!==row.message||finding.path!==row.path||finding.span.startLine+1!==row.line)throw new Error(`Public diagnostic differs from DTO: ${JSON.stringify(finding)}`);
    }
    if(production)return canonical(result.violations.map(f=>({path:f.path,line:f.span.startLine+1,code:f.code,message:f.message})));
    const decoded={...record,files:record.files.map(f=>({...f,source:f.source??oracle.sources[f.sourceSha256]!}))};
    const value=tenResult(decoded,result.violations);return record.check==="graph_lazy_sql_plan"?canonical(value):value;
  }
}
