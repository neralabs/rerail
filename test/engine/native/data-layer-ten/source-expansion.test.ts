import {expect,it} from "vitest";
import {readFileSync} from "node:fs";
import {codeRuleSchema,loadNativeBinding} from "../../../../src/engine/index.js";
it("expands SQL-selected Python modules without a CST or an AST index",()=>{
 const authored={format:"rerail.policy/v1",id:"source-expansion",revision:1,mode:"log",clauses:[{id:"nodes",relational:{
  inputs:{files:{evidence:{as:"f",query:{regex:{source:"file.content",pattern:"(?s)\\A.*\\z"}},project:{path:"file.path"}}}},
  stages:[{name:"nodes",pythonAst:{source:"files",roots:"SELECT __path,source_text(__path) source FROM files WHERE __path!='excluded.py'",fields:["ast_kind","ast_name","ast_start","ast_stop"],expand:"nodes"}}],
  query:"SELECT __path path,CAST(ast_start AS INTEGER) start_byte,CAST(ast_stop AS INTEGER) end_byte,ast_name message FROM nodes WHERE ast_kind='Name'",message:"message",
 },violation:{code:"NAME",message:"Name",anchor:"witness.span"}}]};
 const rule=loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(codeRuleSchema.parse(authored)),"json",readFileSync("native/src/code_rule_dsl/capabilities.json","utf8"));
 const files=[{path:"valid.py",source:"# offset\nvalue=1\n"},{path:"bad.py",source:"if True:\n"},{path:"excluded.py",source:"excluded=2\n"}].map(f=>({...f,language:"text"}));
 const result=JSON.parse(rule.evaluate(JSON.stringify({format:"rerail.repository-snapshot/v1",revision:0,files}),"report"));
 expect(result.failures).toEqual([]);
 expect(result.violations).toMatchObject([{path:"valid.py",message:"value",span:{startByte:9,endByte:14,startLine:1}}]);
 expect(result.violations).toHaveLength(1);
});
