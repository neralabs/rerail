import {expect,it} from "vitest";
import {loadNativeBinding} from "../../../../src/engine/index.js";
it("keeps an invalid module out of canonical relations without losing other documents or its parse error",()=>{
 const documents=["if True:\n","value=1\n"].map(source=>({source,requests:[
  {start:0,end:Buffer.byteLength(source),kind:"module",fields:["ast_nodes"],columns:["ast_kind"]},
  {start:0,end:Buffer.byteLength(source),kind:"module",fields:["syntax_error"]},
 ]}));
 const response=JSON.parse(loadNativeBinding().CodeRuleDslRuntime.pythonAstBatch(JSON.stringify(documents)));
 expect(JSON.parse(response.documents[0][0].ast_nodes)).toEqual({ast_kind:[],ast_start:[],ast_stop:[]});
 expect(JSON.parse(response.documents[0][1].syntax_error)).toMatchObject({lineno:1});
 expect(JSON.parse(response.documents[1][0].ast_nodes).ast_kind).toContain("Assign");
 expect(response.documents[1][1].syntax_error).toBe("null");
});
it("retains strict failure for scalar AST facts on invalid source",()=>{
 const source="if True:\n";
 expect(()=>loadNativeBinding().CodeRuleDslRuntime.pythonAstBatch(JSON.stringify([{source,requests:[{start:0,end:Buffer.byteLength(source),kind:"module",fields:["dump"]}]}]))).toThrow(/CPython AST batch failed/);
});
