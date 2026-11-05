import {expect,it} from "vitest";
import {loadNativeBinding} from "../../../../src/engine/index.js";
it("publishes the Python import level independently of source whitespace",()=>{
 const source="from \\\n    .socket import thing\nfrom socket import thing\n";
 const response=JSON.parse(loadNativeBinding().CodeRuleDslRuntime.pythonAstBatch(JSON.stringify([{source,requests:[{start:0,end:Buffer.byteLength(source),kind:"module",fields:["ast_nodes"],columns:["ast_kind","ast_level"]}]}])));
 const rows=JSON.parse(response.documents[0][0].ast_nodes);
 expect(rows.ast_kind.flatMap((kind:string,i:number)=>kind==="ImportFrom"?[rows.ast_level[i]]:[])).toEqual(["1","0"]);
});
