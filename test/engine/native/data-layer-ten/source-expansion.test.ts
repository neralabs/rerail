import {expect,it} from "vitest";
import {readFileSync} from "node:fs";
import {codeRuleSchema,loadNativeBinding} from "../../../../src/engine/index.js";
it("expands SQL-selected Python modules without a CST or an AST index",()=>{
 const authored={format:"rerail.policy/v1",id:"source-expansion",revision:1,mode:"log",clauses:[{id:"nodes",relational:{
  inputs:{files:{evidence:{as:"f",query:{regex:{source:"file.content",pattern:"(?s)\\A.*\\z"}},project:{path:"file.path"}}}},
  stages:[{name:"nodes",pythonAst:{source:"files",roots:"SELECT __path,source_text(__path) source FROM files WHERE __path!='excluded.py'",fields:["ast_kind","ast_name","ast_start","ast_stop"],expand:"nodes"}}],
  query:"SELECT __path path,CAST(ast_start AS INTEGER) start_byte,CAST(ast_stop AS INTEGER) end_byte,ast_name message FROM nodes WHERE ast_kind='Name'",message:"message",
 },violation:{code:"NAME",message:"Name",anchor:"witness.span"}}]};
 const files=[{path:"valid.py",source:"# 🦉\nvalue=1\n"},{path:"bad.py",source:"if True:\n"},{path:"excluded.py",source:"excluded=2\n"}].map(f=>({...f,language:"text"}));
 function evaluate(program=authored){
  const rule=loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(codeRuleSchema.parse(program)),"json",readFileSync("native/src/code_rule_dsl/capabilities.json","utf8"));
  return JSON.parse(rule.evaluate(JSON.stringify({format:"rerail.repository-snapshot/v1",revision:0,files}),"report"));
 }
 const result=evaluate();
 expect(result.failures).toEqual([]);
 expect(result.violations).toMatchObject([{path:"valid.py",message:"value",span:{startByte:7,endByte:12,startLine:1}}]);
 expect(result.violations).toHaveLength(1);
 const fabricated=structuredClone(authored);
 fabricated.clauses[0]!.relational.query=fabricated.clauses[0]!.relational.query.replace("CAST(ast_start AS INTEGER)","CAST(ast_start AS INTEGER)+1");
 expect(evaluate(fabricated)).toMatchObject({violations:[],failures:[{message:"relational results must anchor to an input witness span"}]});
 const transformed=structuredClone(authored);
 transformed.clauses[0]!.relational.stages[0]!.pythonAst.roots=transformed.clauses[0]!.relational.stages[0]!.pythonAst.roots.replace("source_text(__path)","'# 🦉\nother=1\n'");
 expect(evaluate(transformed)).toMatchObject({violations:[],failures:[{message:"relational results must anchor to an input witness span"}]});
});
