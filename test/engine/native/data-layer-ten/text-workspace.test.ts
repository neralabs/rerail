import { readFileSync } from "node:fs";
import { expect,it } from "vitest";
import { codeRuleSchema,loadNativeBinding,RESULT_PAGE_BYTES } from "../../../../src/engine/index.js";
it("keeps text evidence complete beside an invalid AST and projects captures in workspace execution",async()=>{
 const authored={format:"rerail.policy/v1",id:"text-and-syntax",revision:1,mode:"log",clauses:[{
  id:"syntax",forbid:{ast:{language:"python",syntax:"valid",rule:{kind:"module"}}},violation:{code:"VALID",message:"Valid",anchor:"witness.span"},
 },{id:"text",relational:{inputs:{text:{evidence:{as:"t",query:{regex:{source:"file.content",pattern:"(?s)\\A(?P<TEXT>.*)\\z"}},project:{text:"capture.TEXT"}}}},query:"SELECT __path path,__start start_byte,__end end_byte,text message FROM text",message:"message"},violation:{code:"TEXT",message:"Text",anchor:"witness.span"}}]};
 const rule=loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(codeRuleSchema.parse(authored)),"json",readFileSync("native/src/code_rule_dsl/capabilities.json","utf8"));
 const workspace=new (loadNativeBinding().CodeRuleDslWorkspace)();try{
  workspace.addCodeRule(rule);await workspace.applyAsync(JSON.stringify({revision:1,files:[{path:"bad.py",language:"python",source:"def f(scope: PlanScope):\n    return (\n"}],removed:[],quiet:true}));
  const page=JSON.parse(await workspace.resultPageAsync(0,RESULT_PAGE_BYTES));expect(page.failures).toEqual([]);expect(page.violations).toEqual([expect.objectContaining({code:"TEXT",message:"def f(scope: PlanScope):\n    return (\n"})]);
  await workspace.applyAsync(JSON.stringify({revision:2,files:[{path:"bad.py",language:"python",source:"pass\n"}],removed:[],quiet:true}));
  const changed=JSON.parse(await workspace.resultPageAsync(0,RESULT_PAGE_BYTES));expect(changed.failures).toEqual([]);expect(changed.violations.find((f:{code:string})=>f.code==="TEXT").message).toBe("pass\n");
  await workspace.applyAsync(JSON.stringify({revision:3,files:[],removed:["bad.py"],quiet:true}));
  const removed=JSON.parse(await workspace.resultPageAsync(0,RESULT_PAGE_BYTES));expect(removed.failures).toEqual([]);expect(removed.violations).toEqual([]);
 }finally{workspace.close();}
});
