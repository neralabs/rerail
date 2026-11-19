const m = require(process.env.P_ADDON); const fs = require("fs"); const glob = (d) => fs.readdirSync(d).filter((f) => f.endsWith(".json")).map((f) => `${d}/${f}`);
const caps = fs.readFileSync("/Users/jeremy/orca/workspaces/rerail/Pulling-it-all-together/native/src/code_rule_dsl/capabilities.json", "utf8");
const pack = "/Users/jeremy/orca/workspaces/rerail/wolfeel/packs/python";
const docs = [...glob(`${pack}/rules`), ...glob(`${pack}/candidates/rules`)].sort().map((f) => JSON.parse(fs.readFileSync(f, "utf8")));
const doc = (clauses, i) => ({ format: "rerail.policy/v1", id: `lab-${i}`, revision: 1, mode: "steer", description: "Python pack clauses, for the anchor lab.", scope: { include: ["**/*"], exclude: [] }, clauses });
const ok = (clauses) => { try { m.CodeRuleDslRuntime.compile(JSON.stringify(doc(clauses, 0)), "json", caps); return true; } catch { return false; } };
const chunks = [];
for (const d of docs) {
  const mine = d.clauses.map((c) => ({ ...c, id: `${d.id}--${c.id}` }));
  const at = chunks.find((c) => ok([...c, ...mine]));
  if (at) at.push(...mine); else chunks.push(mine);
}
const paths = chunks.map((c, i) => { const p = `${process.argv[2]}/lab-${i}.json`; fs.writeFileSync(p, JSON.stringify(doc(c, i))); return p; });
fs.writeFileSync(`${process.argv[2]}/policies.txt`, paths.join(":"));
console.log(chunks.length, "documents,", chunks.reduce((a, c) => a + c.length, 0), "clauses");
