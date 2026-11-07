import { mkdir, readFile, writeFile, cp, symlink, stat } from 'node:fs/promises';
import { execFileSync, spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { dirname, resolve, join } from 'node:path';
import { bundleSingleFile } from './bundle.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const game = resolve(here, '..');
const source = resolve(process.env.RERAIL_WASM_SOURCE ?? join(game, '../../More-ergonomic-rule-syntax'));
const revision = execFileSync('git', ['rev-parse', process.env.RERAIL_WASM_REVISION ?? 'b8657e81b'], { cwd: source, encoding: 'utf8' }).trim();
const overlay = join(game, '.cache/wasm-engine');
const dist = join(here, 'dist');
await mkdir(overlay, { recursive: true });
await mkdir(dist, { recursive: true });
const hash = createHash('sha256').update(revision).update('persistent-facts-reactor-v3-rlib-lang-python');
for (const file of ['facts-api.rs', 'facts-plan.rs', 'facts-source.rs']) hash.update(await readFile(join(game, file)));
hash.update(await readFile(join(here, 'facts-wasm.rs')));
const buildKey = hash.digest('hex');
const cached = join(game, '.cache/rerail-facts.wasm');
if (await readFile(join(game, '.cache/wasm-build-key'), 'utf8').catch(() => '') !== buildKey || !await stat(cached).catch(() => null)) {
  // Only committed sources enter the isolated build; never edit the other session's worktree.
  const archive = execFileSync('git', ['archive', revision, 'native', 'Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml'], { cwd: source, maxBuffer: 120 * 1024 * 1024 });
  execFileSync('tar', ['-xf', '-', '-C', overlay], { input: archive });
  const manifestPath = join(overlay, 'native/Cargo.toml');
  await writeFile(manifestPath, (await readFile(manifestPath, 'utf8')).replace('crate-type = ["cdylib", "rlib"]', 'crate-type = ["rlib"]'));
  for (const folder of ['languages', 'docs', 'fixtures', 'model', 'packs', 'src']) {
    try { await symlink(join(source, folder), join(overlay, folder)); }
    catch (error) { if (error.code !== 'EEXIST') throw error; }
  }
  for (const [path, addition] of [
    ['tools.rs', 'facts-api.rs'], ['logic/plan.rs', 'facts-plan.rs'], ['logic/rr.rs', 'facts-source.rs'],
  ]) {
    const target = join(overlay, 'native/src/code_rule_dsl', path);
    await writeFile(target, (await readFile(target, 'utf8')) + '\n' + (await readFile(join(game, addition), 'utf8')));
  }
  async function patch(path, before, after) {
    const target = join(overlay, 'native/src/code_rule_dsl', path);
    const text = await readFile(target, 'utf8');
    if (text.split(before).length !== 2) throw new Error(`Engine hook changed: ${path}`);
    await writeFile(target, text.replace(before, after));
  }
  await patch('logic/plan.rs', '                let n = run.insert_rows(h, &sql)?;',
    '                let n = facts_run_rule(db, prog, rule_no(rule), rule, &sql, || run.insert_rows(h, &sql))?;');
  const clause = '        l.clause(&heads, &body).map_err(|e| match e {';
  await patch('logic/rr/lower.rs', clause, '        let first_rule = l.prog.rules.len();\n' + clause);
  await patch('logic/rr/lower.rs', '    }\n    l.three_valued();',
    '        super::record_rule_sources(&l.prog.rules[first_rule..], line);\n    }\n    l.three_valued();');
  await cp(join(here, 'facts-wasm.rs'), join(overlay, 'native/src/bin/falling-facts-wasm.rs'));
  const target = join(game, '.cache/wasm-target');
  // Use the WASI environment provided by this engine revision, with reactor startup
  // instead of command startup. The same instance and rule cache serve every event.
  const build = 'source native/wasm/env.sh\n' +
    'export CARGO_TARGET_WASM32_WASIP1_RUSTFLAGS="${CARGO_TARGET_WASM32_WASIP1_RUSTFLAGS/crt1-command.o/crt1-reactor.o} -C link-arg=--no-entry -C link-arg=--export=_initialize -C link-arg=--export=facts_allocate -C link-arg=--export=facts_evaluate"\n' +
    'exec "$HOME/.cargo/bin/cargo" build --locked --profile wasm-release --bin falling-facts-wasm --target wasm32-wasip1 --no-default-features --features headless,lang-python';
  console.log(`Building browser Rerail from ${revision.slice(0, 9)} in an isolated snapshot…`);
  const child = spawn('/bin/bash', ['-c', build], { cwd: overlay, stdio: 'inherit', env: { ...process.env, CARGO_TARGET_DIR: target } });
  const code = await new Promise(resolve => { child.on('exit', resolve); child.on('error', error => { console.error(error); resolve(1); }); });
  if (code !== 0) process.exit(code ?? 1);
  await cp(join(target, 'wasm32-wasip1/wasm-release/falling-facts-wasm.wasm'), cached);
  await writeFile(join(game, '.cache/wasm-build-key'), buildKey);
}
await cp(cached, join(dist, 'rerail-facts.wasm'));
for (const file of ['index.html', 'style.css', 'browser.mjs', 'evaluation.mjs', 'rr-source.mjs', 'source-browser.mjs', 'waves.mjs', 'game-session.mjs']) {
  await cp(join(game, file), join(dist, file));
}
for (const file of ['transport.mjs', 'worker.mjs', 'wasm-engine.mjs', 'offline-worker.js']) await cp(join(here, file), join(dist, file));
const shimFiles = execFileSync('git', ['ls-tree', '-r', '--name-only', revision, '--', 'docs/rule-syntax/site/explorer/shim'], { cwd: source, encoding: 'utf8' }).trim().split('\n');
await mkdir(join(dist, 'shim'), { recursive: true });
for (const path of shimFiles) await writeFile(join(dist, 'shim', path.split('/').at(-1)), execFileSync('git', ['show', `${revision}:${path}`], { cwd: source }));
const paths = execFileSync('git', ['ls-tree', '-r', '--name-only', revision, '--', 'docs/rule-syntax/lib', 'native/src/code_rule_dsl/logic/programs', 'native/src/code_rule_dsl/typing/programs'], { cwd: source, encoding: 'utf8' }).trim().split('\n').filter(path => path.endsWith('.rr'));
const files = [{ path: 'game.rr', group: 'Game program', label: 'game.rr', executed: true }, ...paths.map(path => ({
  path, group: path.startsWith('docs/') ? 'Engine library' : 'Engine programs',
  label: path.replace(/^docs\/rule-syntax\/lib\//, '').replace(/^native\/src\/code_rule_dsl\//, ''), executed: false,
}))];
const contents = { 'game.rr': await readFile(join(game, 'game.rr'), 'utf8') };
for (const path of paths) contents[path] = execFileSync('git', ['show', `${revision}:${path}`], { cwd: source, encoding: 'utf8', maxBuffer: 4 * 1024 * 1024 });
await writeFile(join(dist, 'sources.json'), JSON.stringify({ revision, files, contents }));
const notices = ['LICENSE', 'THIRD_PARTY_NOTICES'].map(path =>
  `${path}\n${execFileSync('git', ['show', `${revision}:${path}`], { cwd: source, encoding: 'utf8' })}`).join('\n\n');
await writeFile(join(dist, 'NOTICES.txt'), notices + '\n\nBrowser WASI shim\n' + await readFile(join(dist, 'shim/LICENSE-MIT'), 'utf8') + '\n' + await readFile(join(dist, 'shim/LICENSE-APACHE'), 'utf8'));
let html = await readFile(join(dist, 'index.html'), 'utf8');
html = html.replace('<body>', '<body>\n<div id="engine-loading" role="status" style="padding:14px;text-align:center">Loading browser Rerail…</div>');
html = html.replaceAll('href="/style.css"', 'href="./style.css"').replaceAll('href="/"', 'href="./"').replaceAll('src="/browser.mjs"', 'src="./browser.mjs"');
await writeFile(join(dist, 'index.html'), html);
const assets = ['index.html', 'style.css', 'browser.mjs', 'evaluation.mjs', 'rr-source.mjs', 'source-browser.mjs', 'waves.mjs', 'game-session.mjs', 'transport.mjs', 'worker.mjs', 'wasm-engine.mjs', 'rerail-facts.wasm', 'sources.json', 'NOTICES.txt', ...shimFiles.map(path => `shim/${path.split('/').at(-1)}`)];
const assetHash = createHash('sha256');
for (const asset of [...assets, 'offline-worker.js']) assetHash.update(await readFile(join(dist, asset)));
await writeFile(join(dist, 'manifest.json'), JSON.stringify({ revision, version: assetHash.digest('hex'), assets }));
console.log(`Standalone game: ${dist} (${(await stat(cached)).size} bytes of WASM; ${files.length} source files).`);
await bundleSingleFile(here, dist);
