import { cp, mkdir, readFile, writeFile, symlink, stat, readdir, chmod, rename } from 'node:fs/promises';
import { spawn, spawnSync, execFileSync } from 'node:child_process';
import { dirname, resolve, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';

const here = dirname(fileURLToPath(import.meta.url));
const source = resolve(process.env.RERAIL_SOURCE ?? join(here, '../../wolfeel'));
const overlay = join(here, '.cache/engine');
await mkdir(overlay, { recursive: true });
// Snapshot committed sources: the source worktree can have unrelated edits in progress.
const revision = execFileSync('git', ['rev-parse', process.env.RERAIL_REVISION ?? 'f2a67f63e'], { cwd: source, encoding: 'utf8' }).trim();
const hash = createHash('sha256').update(revision).update('rule-observer-v3');
for (const file of ['facts-api.rs', 'facts-plan.rs', 'facts-main.rs', 'facts-source.rs']) hash.update(await readFile(join(here, file)));
const key = hash.digest('hex');
if (await stat(join(here, '.cache/rerail-facts')).then(() => true, () => false) &&
    await readFile(join(here, '.cache/build-key'), 'utf8').catch(() => '') === key) {
  console.log(`Fact transport already built from ${revision.slice(0, 9)}.`);
  process.exit(0);
}
const archive = execFileSync('git', ['archive', revision, 'native', 'Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml'], {
  cwd: source, maxBuffer: 100 * 1024 * 1024,
});
execFileSync('tar', ['-xf', '-', '-C', overlay], { input: archive });
for (const folder of ['languages', 'docs', 'fixtures', 'model', 'packs', 'src', '.cargo']) {
  try { await symlink(join(source, folder), join(overlay, folder)); }
  catch (error) { if (error.code !== 'EEXIST') throw error; }
}
for (const [target, addition] of [
  ['native/src/code_rule_dsl/tools.rs', 'facts-api.rs'],
  ['native/src/code_rule_dsl/logic/plan.rs', 'facts-plan.rs'],
  ['native/src/code_rule_dsl/logic/rr.rs', 'facts-source.rs'],
]) {
  await writeFile(join(overlay, target), (await readFile(join(overlay, target))) + '\n' + (await readFile(join(here, addition))));
}
const planPath = join(overlay, 'native/src/code_rule_dsl/logic/plan.rs');
let plan = await readFile(planPath, 'utf8');
const insertion = '                let n = run.insert_rows(h, &sql)?;';
if (plan.split(insertion).length !== 2) throw new Error('Engine rule insertion hook changed.');
plan = plan.replace(insertion, [
  '                let n = facts_run_rule(db, prog, rule_no(rule), rule, &sql, || run.insert_rows(h, &sql))?;',
].join('\n'));
await writeFile(planPath, plan);
const lowerPath = join(overlay, 'native/src/code_rule_dsl/logic/rr/lower.rs');
let lower = await readFile(lowerPath, 'utf8');
const clause = '        l.clause(&heads, &body).map_err(|e| match e {';
if (lower.split(clause).length !== 2) throw new Error('Engine clause source hook changed.');
lower = lower.replace(clause, '        let first_rule = l.prog.rules.len();\n' + clause);
const lowered = '    }\n    l.three_valued();';
if (lower.split(lowered).length !== 2) throw new Error('Engine clause completion hook changed.');
lower = lower.replace(lowered, '        super::record_rule_sources(&l.prog.rules[first_rule..], line);\n' + lowered);
await writeFile(lowerPath, lower);

await cp(join(here, 'facts-main.rs'), join(overlay, 'native/src/bin/rerail-facts.rs'));
// Reuse compiled dependencies if this checkout already has them. Engine sources stay isolated.
const sharedTarget = join(source, 'target');
const target = process.env.RERAIL_TARGET_DIR ?? await stat(sharedTarget).then(() => sharedTarget, () => join(here, '.cache/target'));
const profile = 'profiling';
const buildEnv = { ...process.env, CARGO_TARGET_DIR: target };
// Homebrew upgrades can leave rustc linked to an older LLVM and Z3. Find working
// libraries for this process only; leave the machine's compiler and links alone.
if (process.platform === 'darwin' && spawnSync('rustc', ['-vV']).status !== 0) {
  const versions = async name => (await readdir(`/opt/homebrew/Cellar/${name}`).catch(() => []))
    .map(version => `/opt/homebrew/Cellar/${name}/${version}/lib`);
  findLibraries: for (const llvm of await versions('llvm')) for (const z3 of await versions('z3')) {
    const libraries = `${llvm}:${z3}`;
    const attempt = spawnSync('rustc', ['-vV'], { env: { ...process.env, DYLD_LIBRARY_PATH: libraries } });
    if (attempt.status !== 0) continue;
    const rustc = execFileSync('which', ['rustc'], { encoding: 'utf8' }).trim();
    const quote = value => `'${value.replaceAll("'", "'\\''")}'`;
    const wrapper = join(here, '.cache/rustc-local');
    await writeFile(wrapper, `#!/bin/sh\nexport DYLD_LIBRARY_PATH=${quote(libraries)}\nexec ${quote(rustc)} "$@"\n`);
    await chmod(wrapper, 0o755);
    buildEnv.RUSTC = wrapper;
    console.log('Using compatible local LLVM libraries for this build.');
    break findLibraries;
  }
}
console.log(`Building standalone fact transport from ${source}`);
const child = spawn('cargo', ['build', '--locked', '--profile', profile, '--bin', 'rerail-facts'], {
  cwd: overlay, stdio: 'inherit', env: buildEnv,
});
const code = await new Promise(resolve => { child.on('exit', resolve); child.on('error', e => { console.error(e); resolve(1); }); });
if (code !== 0) process.exit(code ?? 1);
await cp(join(target, profile, 'rerail-facts'), join(here, '.cache/rerail-facts.next'));
await rename(join(here, '.cache/rerail-facts.next'), join(here, '.cache/rerail-facts'));
await writeFile(join(here, '.cache/engine-source.txt'), `${source}\n${revision}\n`);
await writeFile(join(here, '.cache/build-key'), key);
