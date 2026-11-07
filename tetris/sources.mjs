import { readFile } from 'node:fs/promises';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
const exec = promisify(execFile);

// Read-only source catalog, restricted to committed .rr files at the built revision.
export function sourceCatalog(gameSource) {
  let catalog;
  const contents = new Map([['game.rr', Promise.resolve(gameSource)]]);
  async function list() {
    if (!catalog) catalog = (async () => {
      const [root, revision] = (await readFile(new URL('./.cache/engine-source.txt', import.meta.url), 'utf8')).trim().split('\n');
      const { stdout } = await exec('git', ['ls-tree', '-r', '--name-only', revision, '--',
        'docs/rule-syntax/lib', 'native/src/code_rule_dsl/logic/programs', 'native/src/code_rule_dsl/typing/programs'], { cwd: root, maxBuffer: 2 * 1024 * 1024 });
      const files = [{ path: 'game.rr', group: 'Game program', label: 'game.rr', executed: true },
        ...stdout.trim().split('\n').filter(path => path.endsWith('.rr')).map(path => ({
          path, group: path.startsWith('docs/') ? 'Engine library' : 'Engine programs',
          label: path.replace(/^docs\/rule-syntax\/lib\//, '').replace(/^native\/src\/code_rule_dsl\//, ''), executed: false,
        }))];
      return { root, revision, files };
    })();
    const { revision, files } = await catalog;
    return { revision, files };
  }
  async function source(path) {
    const { files } = await list();
    const file = files.find(file => file.path === path);
    if (!file) return null;
    if (!contents.has(path)) contents.set(path, (async () => {
      const { root, revision } = await catalog;
      const { stdout } = await exec('git', ['show', `${revision}:${path}`], { cwd: root, maxBuffer: 4 * 1024 * 1024 });
      return stdout;
    })());
    return { ...file, text: await contents.get(path) };
  }
  return { list, source };
}
