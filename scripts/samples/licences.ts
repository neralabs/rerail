/**
 * The licence of every source repository a sample is taken from. A sample carries its repository's SPDX licence,
 * read from the repository's licence file at the sample's commit; the archive's `LICENSES.md` carries those licence
 * texts and the files taken under them. Copyleft sources are never taken: the six source repositories found strong
 * copyleft at their pinned commits, and the one pinned corpus under MPL-2.0.
 */
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { basename, dirname, join } from "node:path";

import { byText, sampleId, type Sample } from "./samples.js";

/**
 * Copyleft sources, by `owner/name` (lowercase), with the evidence: no file of theirs becomes a sample. A pinned
 * corpus or task of any other repository whose licence file reads as copyleft stops the build instead.
 */
export const COPYLEFT_SOURCES: Readonly<Record<string, string>> = {
  "mattermost/mattermost": "AGPL-3.0 for server/channels at 613bb61 (LICENSE.txt; Apache-2.0 covers only templates, i18n, public and webapp)",
  "ankitects/anki": "AGPL-3.0-or-later at d52ca66 (LICENSE)",
  "automattic/wp-calypso": "GPL-2.0-or-later at every task commit (LICENSE.md)",
  "grafana/grafana": "AGPL-3.0-only for public/app at every task commit (LICENSE, LICENSING.md)",
  "secdev/scapy": "GPL-2.0-or-later at every task commit (LICENSE and file headers)",
  "simple-login/app": "AGPL-3.0 at every task commit (LICENSE)",
  "mozilla/application-services": "MPL-2.0 at c19ea2c (COPYRIGHT, LICENSE)",
};

export const copyleftReason = (repo: string): string | undefined => COPYLEFT_SOURCES[repo.toLowerCase()];

/** A source repository's licence as read from its licence files: the SPDX expression and the files that state it. */
interface StatedLicence {
  spdx: string;
  files: readonly string[];
}

/**
 * A source repository's licence at the pinned commits: the repository's, and `at` the pinned commits (full sha) whose
 * licence or licence files differ from it, so a sample carries the licence its repository had at its own commit.
 */
interface SourceLicence extends StatedLicence {
  at?: Readonly<Record<string, StatedLicence>>;
}

/** `licence` at `commit`: the commit's own entry, else the repository's. */
const licenceAt = (licence: SourceLicence, commit: string | undefined): StatedLicence => (commit === undefined ? undefined : licence.at?.[commit]) ?? licence;

/**
 * Every source repository a sample is taken from (`owner/name`, lowercase), with the licence files that state its
 * licence at the pinned commits, as each sample's file had them there (read and checked on every publish,
 * `licenceTexts`): permissive, or, for a commit whose file no allowlisted licence covers (`NONE`: nothing states one),
 * the licence that keeps it out.
 */
export const SOURCE_LICENCES: Readonly<Record<string, SourceLicence>> = {
  "microsoft/vscode": { spdx: "MIT", files: ["LICENSE.txt"] },
  "excalidraw/excalidraw": { spdx: "MIT", files: ["LICENSE"] },
  "shadcn-ui/ui": { spdx: "MIT", files: ["LICENSE.md"] },
  "vercel/next.js": {
    spdx: "MIT",
    files: ["license.md"],
    at: {
      "06136c4abdf2d353fbd583413cf594ff8672b74d": { spdx: "MIT", files: ["packages/next/license.md"] },
      "12b802b259bcf9c69ad0b3fbbf9044fa5f9bf1ff": { spdx: "MIT", files: ["packages/next/license.md"] },
      "15e8f35f336718cada887f08960736614a803648": { spdx: "MIT", files: ["license.md", "packages/next/license.md"] },
      "193f030ecf8de54c272a95bcfd7573ccae75cc04": { spdx: "MIT", files: ["license.md", "packages/next/license.md"] },
      "22f8122ad3b1cd3635edc077d6dbf02c59874a03": { spdx: "MIT", files: ["license.md", "packages/next/license.md"] },
      "2e559938ff9d3fae80594795dbba191f39537fb9": { spdx: "MIT", files: ["packages/next/license.md"] },
      "2f19ae8e01e544e21dcd0a26142e59ade22a5ebc": { spdx: "MIT", files: ["packages/next/license.md"] },
      "2f86fbab810d3dd6ceb95bc5dea5b96525fe03b9": { spdx: "MIT", files: ["packages/next/license.md"] },
      "379ba00c76754b09e607c730a89df886967f8e18": { spdx: "MIT", files: ["packages/next/license.md"] },
      "39c4783f14d4a3984846ceb0040fd452b6c8984d": { spdx: "MIT", files: ["packages/next/license.md"] },
      "44f436b91b9201696c59754d4394f91dcdf77999": { spdx: "MIT", files: ["packages/next/license.md"] },
      "4b85c2809831fbdb656a7d84d38a5020c68ef360": { spdx: "MIT", files: ["packages/next/license.md"] },
      "6dda4371effebcb038671273d342c60fcd7472ed": { spdx: "MIT", files: ["packages/next/license.md"] },
      "6ed03f4f7615c89ecca8e036146af8116269fef7": { spdx: "MIT", files: ["packages/next/license.md"] },
      "72003a79a50f14766cf461fdfad5d03e7973badc": { spdx: "MIT", files: ["packages/next/license.md"] },
      "72c26d1442602c9ab000354b7addb63841884a91": { spdx: "MIT", files: ["packages/next/license.md"] },
      "74c6c1b6e25c4545bb31ef39f00c9b86c5664464": { spdx: "MIT", files: ["packages/next/license.md"] },
      "81e667bfaebbf651162e88e6df392c762c68b386": { spdx: "MIT", files: ["packages/next/license.md"] },
      "88f07ed3b917baae57e2fa51cbb0ad4e59398d42": { spdx: "MIT", files: ["packages/next/license.md"] },
      "9b6e563f7fb64d1ec6225306b17acabe741a5098": { spdx: "MIT", files: ["packages/next/license.md"] },
      "9dd974dfca1efd0b451065056f95f92747e2dada": { spdx: "MIT", files: ["packages/next/license.md"] },
      "a430936c7f803767335eab151b294d69524621a2": { spdx: "MIT", files: ["packages/next/license.md"] },
      "a7ca031313b3bd939e5a78266c16b5c956548660": { spdx: "MIT", files: ["packages/next-server/license.md"] },
      "a89c0afe5702f604a142f5d185b0b0018b4eefa7": { spdx: "MIT", files: ["packages/next/license.md"] },
      "b3512c2ea3d672ec07bf5568bbf126786a13241a": { spdx: "MIT", files: ["packages/next/license.md"] },
      "c1fe35e43c250c554cda30ddbfdcbde9829b87d4": { spdx: "MIT", files: ["packages/next/license.md"] },
      "c9d599b69864f4a6c08cada4d0c9733cc630ddc4": { spdx: "MIT", files: ["packages/next-server/license.md"] },
      "cdb48f76410b8a734a133ef03757aa6f3ef1bff8": { spdx: "MIT", files: ["license.md", "packages/next/license.md"] },
      "ce422240e3c9203dbc5e9a3e864a3b1cc30f372f": { spdx: "MIT", files: ["packages/next/license.md"] },
      "da8f4e9229ef139f4307d6a3441b32c053fba3ad": { spdx: "MIT", files: ["packages/next/license.md"] },
      "e6bca73d8763b8312a1cf1a1e36defb175d3fe02": { spdx: "MIT", files: ["packages/next/license.md"] },
      "e9180eae2afd6794018e206cca2c14adc7fbe1d1": { spdx: "MIT", files: ["packages/next/license.md"] },
      "eab3ab87dac708c3ded387f3e0cf42c9ad13a60e": { spdx: "MIT", files: ["packages/next/license.md"] },
      "ec0abf410c8e8b145f87f37143c403a8d38e613f": { spdx: "MIT", files: ["packages/next/license.md"] },
      "f09aa18f8ac286230a507102753c8e1373a45149": { spdx: "MIT", files: ["license.md", "packages/next/license.md"] },
      "f17e61cb1f430177bf1f062706e09ea630268267": { spdx: "MIT", files: ["packages/next/license.md"] },
      "f1a69d7d76cddf40a144864f9d520453a1f1b4e3": { spdx: "MIT", files: ["license.md", "packages/next/license.md"] },
      "f66360c7237b3a52dc0adc8672edf8d20404151e": { spdx: "MIT", files: ["license.md", "packages/next/license.md"] },
      "fdf41d6f6315ebff3a7056d7e7f002b8e78749d4": { spdx: "MIT", files: ["packages/next/license.md"] },
    },
  },
  "vercel/commerce": { spdx: "MIT", files: ["license.md"] },
  "element-plus/element-plus": {
    spdx: "MIT",
    files: ["LICENSE"],
    at: {
      "5b98819a8557b616a988d86df40c2aaa366bc43e": { spdx: "MIT", files: ["package.json"] },
    },
  },
  "vbenjs/vue-vben-admin": { spdx: "MIT", files: ["LICENSE"] },
  "hoppscotch/hoppscotch": { spdx: "MIT", files: ["LICENSE"] },
  "django/django": { spdx: "BSD-3-Clause", files: ["LICENSE"] },
  "postgres/postgres": { spdx: "PostgreSQL", files: ["COPYRIGHT"] },
  "burntsushi/ripgrep": { spdx: "Unlicense OR MIT", files: ["COPYING", "LICENSE-MIT", "UNLICENSE"] },
  "tokio-rs/tokio": { spdx: "MIT", files: ["LICENSE"] },
  "iterative/dvc": { spdx: "Apache-2.0", files: ["LICENSE"] },
  "dask/dask": { spdx: "BSD-3-Clause", files: ["LICENSE.txt"] },
  "conan-io/conan": { spdx: "MIT", files: ["LICENSE.md"] },
  "scikit-learn/scikit-learn": { spdx: "BSD-3-Clause", files: ["COPYING"] },
  "psf/requests": { spdx: "Apache-2.0", files: ["LICENSE"] },
  "modin-project/modin": { spdx: "Apache-2.0", files: ["LICENSE"] },
  "pydantic/pydantic": { spdx: "MIT", files: ["LICENSE"] },
  "laravel/framework": { spdx: "MIT", files: ["LICENSE.md"] },
  "symfony/symfony": { spdx: "MIT", files: ["LICENSE"] },
  "fastify/fastify": { spdx: "MIT", files: ["LICENSE"] },
  "prettier/prettier": { spdx: "MIT", files: ["LICENSE"] },
  "filamentphp/filament": { spdx: "MIT", files: ["LICENSE.md"] },
  "bookstackapp/bookstack": { spdx: "MIT", files: ["LICENSE"] },
  "bagisto/bagisto": { spdx: "MIT", files: ["LICENSE"] },
  // The PHP test set's other sources (scripts/samples/convert-php-test-set.ts).
  "composer/composer": { spdx: "MIT", files: ["LICENSE"] },
  "cakephp/cakephp": { spdx: "MIT", files: ["LICENSE"] },
  "codeigniter4/codeigniter4": { spdx: "MIT", files: ["LICENSE"] },
  "yiisoft/yii2": { spdx: "BSD-3-Clause", files: ["LICENSE.md"] },
  "doctrine/orm": { spdx: "MIT", files: ["LICENSE"] },
  "seldaek/monolog": { spdx: "MIT", files: ["LICENSE"] },
  "guzzle/guzzle": { spdx: "MIT", files: ["LICENSE"] },
  "twigphp/twig": { spdx: "BSD-3-Clause", files: ["LICENSE"] },
  "slimphp/slim": { spdx: "MIT", files: ["LICENSE.md"] },
  // The PHP test set's suppression cases, found by code search for comments naming a ported rule.
  "amphp/cluster": { spdx: "MIT", files: ["LICENSE"] },
  "aphiria/aphiria": { spdx: "MIT", files: ["LICENSE.md"] },
  "buggregator/trap": { spdx: "BSD-3-Clause", files: ["LICENSE.md"] },
  "friendsofcake/cakepdf": { spdx: "MIT", files: ["LICENSE.txt"] },
  "jbzoo/utils": { spdx: "MIT", files: ["LICENSE"] },
  "laminas/laminas-http": { spdx: "BSD-3-Clause", files: ["LICENSE.md"] },
  "laminas/laminas-mail": { spdx: "BSD-3-Clause", files: ["LICENSE.md"] },
  "lucatume/wp-browser": { spdx: "MIT", files: ["LICENSE"] },
  "luzrain/workerman-bundle": { spdx: "MIT", files: ["LICENSE"] },
  "markrogoyski/itertools-php": { spdx: "MIT", files: ["LICENSE"] },
  "phpstreamserver/phpstreamserver": { spdx: "MIT", files: ["LICENSE"] },
  "reliforp/reli-prof": { spdx: "MIT", files: ["LICENSE"] },
  "skillshare/formatphp": { spdx: "MIT", files: ["LICENSE"] },
  "tarlepp/symfony-flex-backend": { spdx: "MIT", files: ["LICENSE"] },
  "wp-cli/config-command": { spdx: "MIT", files: ["LICENSE"] },
  "wp-cli/shell-command": { spdx: "MIT", files: ["LICENSE"] },
  "wp-cli/wp-cli": { spdx: "MIT", files: ["LICENSE"] },
  // The repository is partly public domain and partly BSD-2-Clause (LICENSE.rst); the file taken from it,
  // lib/Crypto/IO/PEM.py, names BSD-2-Clause in its own SPDX licence header.
  "legrandin/pycryptodome": { spdx: "BSD-2-Clause", files: ["LICENSE.rst"] },
  // The Node test set's sources (the node tool-pack's originals and fixed copies): each sample's licence is its file's
  // nearest licence file at its commit (a monorepo package's own, else the root's), or with none the README's licence
  // section or package.json's `license`; `at` names the commits where that differs from the repository's.
  "appium/appium": { spdx: "Apache-2.0", files: ["LICENSE"] },
  "automattic/mongoose": {
    spdx: "MIT",
    files: ["LICENSE"],
    at: {
      "021f84ba2e84ffb7e058789c99bac2e2048b6597": { spdx: "MIT", files: ["LICENSE.md"] },
      "04a985a4924698bf4173c8255c6b637877a437f8": { spdx: "MIT", files: ["LICENSE.md"] },
      "09ba87e55441641cc02ed06f8497554d91c36aa4": { spdx: "MIT", files: ["LICENSE.md"] },
      // 2010: no licence file, no `license` in package.json, nothing in the README.
      "0a37ae8ac1d5e8475d831d36100f66997961ea1e": { spdx: "NONE", files: [] },
      "0a988a0c81dbfdff5de30f1956bf4a5650782070": { spdx: "MIT", files: ["LICENSE.md"] },
      "11725e086dbaea4933557b1a2e548c52f2633e42": { spdx: "MIT", files: ["LICENSE.md"] },
      "151bf700de40105dbe6d0d0d4f3ef17fc321c3d4": { spdx: "MIT", files: ["LICENSE.md"] },
      "1876dbf7cad288ff720305998bd13d0f8625d655": { spdx: "MIT", files: ["README.md"] },
      "1b3308b0e7d3e12fa1474ed08ac487c49fd0668e": { spdx: "MIT", files: ["LICENSE.md"] },
      "21b79d301e4fac6252b6c98c07c294e0a0cd8e34": { spdx: "MIT", files: ["LICENSE.md"] },
      "23a4fde25871a9632abba7a605199eaa0fc0260b": { spdx: "MIT", files: ["LICENSE.md"] },
      "23fa88ecc8f25f9d950bd8ec3d6e9a39d1605d8e": { spdx: "MIT", files: ["LICENSE.md"] },
      "243b7a26d17f29a3c12a4e7910f84bca8648ffa4": { spdx: "MIT", files: ["LICENSE.md"] },
      "257d75cf31921c8995a26951d609e2261bb908aa": { spdx: "MIT", files: ["LICENSE.md"] },
      "2a2fec99a0345c03f5afb3d3f383a694433b1ad5": { spdx: "MIT", files: ["README.md"] },
      "2abdbb1f062f6b12b09d11ee6ff030c7097f4ea8": { spdx: "MIT", files: ["LICENSE.md"] },
      "2b5c4bcc26db352cd7a25c6e4cd241f16ae41611": { spdx: "MIT", files: ["README.md"] },
      "2ba453d2f80537cc39e6732e183dba327e4f1105": { spdx: "MIT", files: ["README.md"] },
      "2bf3cc01f8e0191f4321be011fef6df74cb2ec7e": { spdx: "MIT", files: ["LICENSE.md"] },
      "2cdce60ad384378f7964251b64f9f79308a25f87": { spdx: "MIT", files: ["LICENSE.md"] },
      "2df4667e998beb85eab5de114b67826bce86533c": { spdx: "MIT", files: ["LICENSE.md"] },
      "322b63d6a4c6e76c0115faf469d6482362c4fffe": { spdx: "MIT", files: ["README.md"] },
      "32d30201470e14af5404e6b3e60dd83584873f57": { spdx: "MIT", files: ["LICENSE.md"] },
      "3cfcd4f02aec8a36e60c1777466dfd395401b973": { spdx: "MIT", files: ["README.md"] },
      "3d905bb79e78f6f400a5ffd71661320be087e093": { spdx: "MIT", files: ["LICENSE.md"] },
      "4353a2976d35e210157824b521ee468c2270200f": { spdx: "MIT", files: ["LICENSE.md"] },
      "4443584defd7b1fe475afab217e7f4c9cd63f328": { spdx: "MIT", files: ["LICENSE.md"] },
      "44f194b88bc857f07add94c441f2b02d0625dd02": { spdx: "MIT", files: ["README.md"] },
      "47d771775a2301edcd135ec97afe4a0df6f2ad0e": { spdx: "MIT", files: ["LICENSE.md"] },
      "4edd44f6174f892a20ebb808b63b26c362a97763": { spdx: "MIT", files: ["README.md"] },
      "4f740f2ab75fefb8b25e28fa9e7a0b8bacb15c20": { spdx: "MIT", files: ["LICENSE.md"] },
      "519f3996f31b597d2ae4248ee3b28f943611e678": { spdx: "MIT", files: ["LICENSE.md"] },
      "525a7e9a10922acc0a3bbb9e59d57a356e79e713": { spdx: "MIT", files: ["LICENSE.md"] },
      "5276afdbf5c60c87b7d7e86951089bd0eac3b54f": { spdx: "MIT", files: ["LICENSE.md"] },
      "54e47adf225912b4c909e452248e1d84bb3e8d18": { spdx: "MIT", files: ["LICENSE.md"] },
      "5e40b5b6f1020423012f4dd1934074f9eca62ef7": { spdx: "MIT", files: ["README.md"] },
      "650cfb6ef31b8ef1fea18769b5bb108c077dcde5": { spdx: "MIT", files: ["LICENSE.md"] },
      "692b66f99b02b6db3a223dd11afd38ddf630805d": { spdx: "MIT", files: ["LICENSE.md"] },
      "6b1eb1d513242f7607f9129241c67f2dd5c21b67": { spdx: "MIT", files: ["LICENSE.md"] },
      "6b811a3a4236204b6964dff344b30ea801c1bdd2": { spdx: "MIT", files: ["README.md"] },
      "728e18c6f539041780a9d134287fa569c6fc28c8": { spdx: "MIT", files: ["LICENSE.md"] },
      "780de3fe80ee0bb891e34ff9d854b15b1562d5d6": { spdx: "MIT", files: ["LICENSE.md"] },
      "7833ae3ac6eb7a2ca27968fa14c4ab9c7a79dae8": { spdx: "MIT", files: ["LICENSE.md"] },
      "7c9f6686e0a32ac7018029d80f5e7304e9960e80": { spdx: "MIT", files: ["LICENSE.md"] },
      "7dbe334732f11d59d682179b2e0adf4a03f1148e": { spdx: "MIT", files: ["LICENSE.md"] },
      "7e05890067331973418376da0b250cc073aba682": { spdx: "MIT", files: ["LICENSE.md"] },
      "8527809eee8bf07709188db382e196be33ddf55e": { spdx: "MIT", files: ["LICENSE.md"] },
      "889f18d5fd985e75e8bca91a8608fd78eaa56f1c": { spdx: "MIT", files: ["LICENSE.md"] },
      "8a8b026344c4b0211a84916d1aab187d43f39707": { spdx: "MIT", files: ["README.md"] },
      "8b847c0c32bf638dc16e28c18344c39b98e13744": { spdx: "MIT", files: ["README.md"] },
      "8b92e43e037e12a461664c77293000fc39bbfa35": { spdx: "MIT", files: ["LICENSE.md"] },
      "929cfdd35e5b60a58e5f7dde12d0ebe305b8acc8": { spdx: "MIT", files: ["LICENSE.md"] },
      "936408eff18b60533efffa4e3215422d5692fa35": { spdx: "MIT", files: ["README.md"] },
      "958e386e6301a2d1349e1ea555948a988ac2516e": { spdx: "MIT", files: ["LICENSE.md"] },
      "a126957a5d17e367ee0ba47cd5869cea51b89e34": { spdx: "MIT", files: ["LICENSE.md"] },
      "a37bfc6227012ebef0e28f0fc5d4baa3cfd95808": { spdx: "MIT", files: ["LICENSE.md"] },
      "a8aee7c4b8d0d4e3e402c90445b5d83a90b8c5aa": { spdx: "MIT", files: ["LICENSE.md"] },
      "b34853e1460d27eab6be49759346d8b7ed846b40": { spdx: "MIT", files: ["LICENSE.md"] },
      "b8417457cbc5b682365e2ff391728299adce268f": { spdx: "MIT", files: ["LICENSE.md"] },
      "b8b0b8e317c14922fb58d32ddcf0dba9faed6178": { spdx: "MIT", files: ["LICENSE.md"] },
      // 2010: no licence file, no `license` in package.json, nothing in the README.
      "bd4f0e3171906130e045203e8d175ea0ca7d1ef4": { spdx: "NONE", files: [] },
      "beed59f732cf6d90913a9c95dd2a0e0412dca5d3": { spdx: "MIT", files: ["LICENSE.md"] },
      "bf63264465a5c21c8c9a4fe6a1c810c677c656b8": { spdx: "MIT", files: ["LICENSE.md"] },
      "c0c5c6fb1e462fd4bcf1af245418fab679363bcb": { spdx: "MIT", files: ["README.md"] },
      "c18742ed9d302baeb059b0f320f8894cca602efb": { spdx: "MIT", files: ["LICENSE.md"] },
      "c2314638b4e3ba37e1201d59f340fef52b1e7fc7": { spdx: "MIT", files: ["LICENSE.md"] },
      "ca5207e62df27c591147c050fac5d1b3f766742f": { spdx: "MIT", files: ["LICENSE.md"] },
      "ce20da7ef5ebe69374452f5e5a5ad7a2c8d7d086": { spdx: "MIT", files: ["README.md"] },
      "ce4bce2a01804b8a081b174e43f2b67e76712a6c": { spdx: "MIT", files: ["LICENSE.md"] },
      "d74812a7aa06f2451eadd187ee5950404acb999f": { spdx: "MIT", files: ["LICENSE.md"] },
      "d8ceee68aceae1176c581460f1dc1743538152b0": { spdx: "MIT", files: ["LICENSE.md"] },
      "da46f56c77331d58975561153ec8017feac3af78": { spdx: "MIT", files: ["README.md"] },
      "e356c41a98246b74554ca7a111c13d90a3cbda03": { spdx: "MIT", files: ["LICENSE.md"] },
      "e5397d0535008f61cf42f255eab097385c12b421": { spdx: "MIT", files: ["LICENSE.md"] },
      "e54b0f72e469e7b9ff2080e07a7c5f2b6daa264f": { spdx: "MIT", files: ["LICENSE.md"] },
      "eaa9fddb28993efca7eb4efa1ab07dd05b6b4371": { spdx: "MIT", files: ["README.md"] },
      "ed157bea2cd2590afe50408551c90e7661a4284b": { spdx: "MIT", files: ["LICENSE.md"] },
      "f0f5ffd17eb17f54dfd18d7f06b7b3d60062f384": { spdx: "MIT", files: ["LICENSE.md"] },
      "f743f23a29f3ed718ef02fc1e5a2b42664bef89a": { spdx: "MIT", files: ["LICENSE.md"] },
      // 2010: no licence file, no `license` in package.json, nothing in the README.
      "f797c95356e8987ecb59502faab32151c792c660": { spdx: "NONE", files: [] },
      "f8f151fd9c79fdcd5ab36763b56d356e990b52fb": { spdx: "MIT", files: ["LICENSE.md"] },
      "f91a1ff9d95faa9bad85fdbce3d54ed946da7583": { spdx: "MIT", files: ["LICENSE.md"] },
    },
  },
  "axios/axios": { spdx: "MIT", files: ["LICENSE"] },
  "babel/babel": {
    spdx: "MIT",
    files: ["LICENSE"],
    at: {
      "0915316cf4a66e85af77ff9043d2f9c8a439418b": { spdx: "MIT", files: ["packages/babel-parser/LICENSE"] },
      "0ca5fdaab734b2b755136ca12b79fc71d5f8e012": { spdx: "MIT", files: ["packages/babel-parser/LICENSE"] },
      "2782a549e99d2ef1816332d23d7dfd5190f58a0f": { spdx: "MIT", files: ["packages/babel-parser/LICENSE"] },
      "2e0fcecea5331df460b9f9748857d0183c256708": { spdx: "MIT", files: ["packages/babel-parser/LICENSE"] },
      // The sample is under regenerator/: Facebook's BSD-2-Clause licence with its patent grant (PATENTS).
      "32cb2d8163cb9f0384f71f681d2adb8590461390": { spdx: "BSD-2-Clause", files: ["regenerator/LICENSE", "regenerator/PATENTS"] },
      "36275df8c8734f9ffde4f93f990aada281b34fc8": { spdx: "MIT", files: ["packages/babel-parser/LICENSE"] },
      "4afd2070d3c4c9fab90a916ebc37dda8e3092a5e": { spdx: "MIT", files: ["packages/babel-parser/LICENSE"] },
      "52593e121038ae032d9d2b57a4c4670efd9361b8": { spdx: "MIT", files: ["packages/babel-parser/LICENSE"] },
      "5a1f755469dd00e33f97093567f236ac3244e47e": { spdx: "MIT", files: ["eslint/babel-eslint-plugin/LICENSE"] },
      "5b9468d9bf1ab4f427241673e9f03593da115a69": { spdx: "MIT", files: ["packages/babel-helpers/LICENSE"] },
      "6cd3db7d9c125ec5100093f4ed7e6f6db570c409": { spdx: "MIT", files: ["packages/babel-parser/LICENSE"] },
      "97105cb9f285964065a525c7ab4fd718c57e2a40": { spdx: "MIT", files: ["packages/babel-helpers/LICENSE"] },
      "9b7f37e732208ca48faf9c8c37fdbb180b512874": { spdx: "MIT", files: ["packages/babel-helpers/LICENSE"] },
      "9c351e5220df774498e26a0fcd337921085f065a": { spdx: "MIT", files: ["packages/babel-plugin-transform-regenerator/LICENSE"] },
      "9dcd11575b195e65ed111321bfb33d73e136d7b8": { spdx: "MIT", files: ["packages/babel-helpers/LICENSE"] },
      "a0690e39ea63cdcc3d9282ece739e6677c83ad6e": { spdx: "MIT", files: ["packages/babel-helpers/LICENSE"] },
      "abe208ca5ad41e0a21c4d7663d111d4642dda8a1": { spdx: "MIT", files: ["packages/babel-helpers/LICENSE"] },
      "acc19f92476347c1888381f4c9f0277b8e5a5745": { spdx: "MIT", files: ["packages/babel-parser/LICENSE"] },
      "b4ff3e7d300a7f62e89fe45a8ff235042b46f75c": { spdx: "MIT", files: ["packages/babel-helpers/LICENSE"] },
      "c813803e5c055cde4843bd288bcb36a867b2cab4": { spdx: "MIT", files: ["packages/babel-parser/LICENSE"] },
      "fe32019663f91c2e9aa1fbd4fe96bdc2ae27bf3b": { spdx: "MIT", files: ["packages/babel-helpers/LICENSE"] },
    },
  },
  "balderdashy/sails": {
    spdx: "MIT",
    files: ["LICENSE.md"],
    at: {
      "c39c5e38970058c7ddd6b43e04ba795856c5ab07": { spdx: "MIT", files: ["package.json"] },
    },
  },
  "brianc/node-postgres": {
    spdx: "MIT",
    files: ["LICENSE"],
    at: {
      "4b88c82b1687fb758f65a2c1e7c5213fe3519272": { spdx: "MIT", files: ["README.md"] },
      "94f670590f44f8bb1cbd5356598caae750517037": { spdx: "MIT", files: ["README.md"] },
      "d21cc09556899b8038ec23613a801c19228637ca": { spdx: "MIT", files: ["packages/pg-connection-string/LICENSE"] },
      "ec0d0beff29087a809d85da61faadb44401fcdc1": { spdx: "MIT", files: ["README.md"] },
    },
  },
  "cypress-io/cypress": { spdx: "MIT", files: ["LICENSE"] },
  "dherault/serverless-offline": { spdx: "MIT", files: ["LICENSE"] },
  "discordjs/discord.js": {
    spdx: "Apache-2.0",
    files: ["LICENSE"],
    at: {
      "052ed7fbe7dc0412de7fbf0ac1db44cdc282513f": { spdx: "Apache-2.0", files: ["packages/scripts/LICENSE"] },
      "096177189f5daf00762f27081e5e63da75c76e82": { spdx: "Apache-2.0", files: ["packages/actions/LICENSE"] },
      "25aff7e218af5182b7968a7547e4751a0e050e30": { spdx: "Apache-2.0", files: ["packages/discord.js/LICENSE"] },
      "281f8203d6ad90fa8daee9faaa04fb8dfb8fbeb4": { spdx: "Apache-2.0", files: ["packages/discord.js/LICENSE"] },
      "2c3bf5c8174e90a3b96982e5017a23c9223ddbea": { spdx: "Apache-2.0", files: ["packages/actions/LICENSE"] },
      "5888663392485d35b2538f042c48efea3b93cd54": { spdx: "Apache-2.0", files: ["packages/rest/LICENSE"] },
      "68bb8af58ad8720abb998dcaa297f89d3317ffa8": { spdx: "Apache-2.0", files: ["packages/builders/LICENSE"] },
      "748d7271c45796479a29d8ed3101421de09ef867": { spdx: "Apache-2.0", files: ["packages/ws/LICENSE"] },
      "c9855614c481d6fd4409644b7baee138f0b484e2": { spdx: "MIT", files: ["packages/rpc/LICENSE"] },
      "d923e90ae67e1dce3ea875e05fdcf0a20786021f": { spdx: "Apache-2.0", files: ["packages/create-discord-bot/LICENSE"] },
      "ec3ef7b1bd2b871f20a2d2f4c158c868f46dd87c": { spdx: "Apache-2.0", files: ["packages/create-discord-bot/LICENSE"] },
    },
  },
  "electron/electron": { spdx: "MIT", files: ["LICENSE"] },
  "eslint/eslint": { spdx: "MIT", files: ["LICENSE"] },
  "ether/etherpad": { spdx: "Apache-2.0", files: ["LICENSE"] },
  "expressjs/express": { spdx: "MIT", files: ["LICENSE"] },
  "feathersjs/feathers": { spdx: "MIT", files: ["packages/authentication-oauth/LICENSE"] },
  "gatsbyjs/gatsby": {
    spdx: "MIT",
    files: ["LICENSE"],
    at: {
      // The sample is under a starter whose own LICENSE is 0BSD.
      "16d91f3aa507c0f20af3ab797c90b4fb5011cf0f": { spdx: "0BSD", files: ["starters/gatsby-starter-wordpress-blog/LICENSE"] },
      "20baf7cf40ad8b567c25a547c354a0b1a49d50aa": { spdx: "MIT", files: ["LICENSE", "e2e-tests/development-runtime/LICENSE", "e2e-tests/production-runtime/LICENSE"] },
      "3783ffb57528ca43fbb3ad4487f02ace0a71f155": { spdx: "MIT", files: ["e2e-tests/contentful/LICENSE"] },
      "3954944f70be205ff8202f26288263ba8db80730": { spdx: "MIT", files: ["e2e-tests/development-runtime/LICENSE"] },
      "39e10299259f514c2803391de98c2e2f77f7f2cf": { spdx: "MIT", files: ["e2e-tests/production-runtime/LICENSE"] },
      "4a780fbac717b1df337f156e2ac4b2da6478106b": { spdx: "MIT", files: ["LICENSE", "e2e-tests/production-runtime/LICENSE"] },
      "5c22948094d98f3d4ea90253c80fc78d9f53cc20": { spdx: "MIT", files: ["e2e-tests/development-runtime/LICENSE"] },
      "5d0a0573b630e1bd44ad14a67314d55346c99e2c": { spdx: "MIT", files: ["integration-tests/images/LICENSE"] },
      "66127cc34d7568f3343aebfb7d2e1a71ec5ee150": { spdx: "MIT", files: ["integration-tests/artifacts/LICENSE"] },
      "6c7a0e3ab1a6b7aa1a7ef288b8d61bdf4e5b4622": { spdx: "MIT", files: ["e2e-tests/development-runtime/LICENSE"] },
      "7130cd4b5be9007d851200ebabef9716c92302eb": { spdx: "MIT", files: ["integration-tests/artifacts/LICENSE"] },
      // The sample is under a starter whose own LICENSE is 0BSD.
      "72b35862556e866644e79b63cd410f1920689dc9": { spdx: "0BSD", files: ["starters/gatsby-starter-minimal-ts/LICENSE"] },
      "7dade1e89ca8786bd7d2705e1405e53426250d37": { spdx: "MIT", files: ["www/LICENSE"] },
      "94c2d735ad378bf05836e74cc47f7ca9523cabdc": { spdx: "MIT", files: ["e2e-tests/development-runtime/LICENSE"] },
      "94fe5d39b709d3aec758e81dd0209a4ee88251e4": { spdx: "MIT", files: ["www/LICENSE"] },
      "9a912956a5ea66be7281f6263928b64f20ff1d53": { spdx: "MIT", files: ["e2e-tests/production-runtime/LICENSE"] },
      "9be77e3210a5eff26a25aa0b6dbdeb1b838f61f8": { spdx: "MIT", files: ["www/LICENSE"] },
      "9e39c05f2c48aedfc495421367e67ee4bcbf1c4a": { spdx: "MIT", files: ["LICENSE", "e2e-tests/development-runtime/LICENSE", "e2e-tests/production-runtime/LICENSE"] },
      "9f8ed09228a17fcad6d898024adc55262fa1845f": { spdx: "MIT", files: ["e2e-tests/contentful/LICENSE"] },
      "b8cfe55cba0fa38dfbe7b4514bef5a9886c5693c": { spdx: "MIT", files: ["e2e-tests/development-runtime/LICENSE"] },
      "e3365ab01eebb6c17bbae7fcb7e74040a0c4f260": { spdx: "MIT", files: ["LICENSE", "integration-tests/lmdb-regeneration/LICENSE"] },
      "f1589305d9050d34a75d44d91960b98319960128": { spdx: "MIT", files: ["e2e-tests/development-runtime/LICENSE"] },
    },
  },
  "getsentry/sentry-javascript": { spdx: "MIT", files: ["packages/browser/LICENSE"] },
  "hexojs/hexo": { spdx: "MIT", files: ["LICENSE"] },
  "iobroker/iobroker.js-controller": { spdx: "MIT", files: ["packages/common/LICENSE"] },
  "jestjs/jest": {
    spdx: "MIT",
    files: ["LICENSE"],
    at: {
      "12c13459b35c812db7eee6663bf521032316c127": { spdx: "BSD-3-Clause", files: ["LICENSE", "PATENTS"] },
      "242fb35459b387e360f52394b0e30f42101cc51d": { spdx: "BSD-3-Clause", files: ["LICENSE", "PATENTS"] },
      "27b53ca4fe0e72009bfadd0eb4fd7d6b8f88fea2": { spdx: "BSD-3-Clause", files: ["LICENSE", "PATENTS"] },
      "4f2ce5f3be0a273929f3ef1b8fe05b668285d6c4": { spdx: "BSD-3-Clause", files: ["LICENSE", "PATENTS"] },
      "81d6697617b130cddf1eff3df9c0fec084efbdf7": { spdx: "BSD-3-Clause", files: ["LICENSE", "PATENTS"] },
      "88d6dba783c0b814f85ffd4a54c56b8f7bd4ba6d": { spdx: "BSD-3-Clause", files: ["LICENSE", "PATENTS"] },
      "ea3204789a0005cdb758fc068766b828673668fb": { spdx: "BSD-3-Clause", files: ["LICENSE", "PATENTS"] },
    },
  },
  "karma-runner/karma": { spdx: "MIT", files: ["LICENSE"] },
  "knex/knex": { spdx: "MIT", files: ["LICENSE"] },
  "kong/insomnia": { spdx: "Apache-2.0", files: ["LICENSE"] },
  "lerna/lerna": { spdx: "MIT", files: ["LICENSE"] },
  "less/less.js": { spdx: "Apache-2.0", files: ["LICENSE"] },
  "louislam/uptime-kuma": { spdx: "MIT", files: ["LICENSE"] },
  "magicmirrororg/magicmirror": { spdx: "MIT", files: ["LICENSE.md"] },
  "mde/ejs": {
    spdx: "Apache-2.0",
    files: ["LICENSE"],
    at: {
      // 2010: no licence file, no `license` in package.json, nothing in the README.
      "2f5b75fdfae634f0fec71c9edfd480fedd7dd9c9": { spdx: "NONE", files: [] },
    },
  },
  "meteor/meteor": {
    spdx: "MIT",
    files: ["LICENSE"],
    at: {
      // MIT for Meteor's own code, then the licences of the libraries it bundles, MongoDB's AGPL among them.
      "e4c568b1e1905062c5b3a2cf69784ff6fc8ad404": { spdx: "MIT", files: ["LICENSE.txt"] },
    },
  },
  "mochajs/mocha": {
    spdx: "MIT",
    files: ["LICENSE"],
    at: {
      // The sample at this commit is under docs/, which docs/LICENSE-CC-BY-4.0 licenses.
      "3c191c05d9db1e99aec9b600edac2ce10a6b6d71": { spdx: "CC-BY-4.0", files: ["docs/LICENSE-CC-BY-4.0"] },
    },
  },
  "mongodb/node-mongodb-native": { spdx: "Apache-2.0", files: ["LICENSE.md"] },
  "nestjs/nest": { spdx: "MIT", files: ["LICENSE"] },
  "node-fetch/node-fetch": { spdx: "MIT", files: ["LICENSE.md"] },
  "node-red/node-red": { spdx: "Apache-2.0", files: ["LICENSE"] },
  "nodejs/undici": { spdx: "MIT", files: ["LICENSE"] },
  "nuxt/nuxt": {
    spdx: "MIT",
    files: ["LICENSE"],
    at: {
      "201236195d3c5c0108c764bc56734d418a687e76": { spdx: "MIT", files: ["package.json"] },
      "b49c62b3353843dfcfb57a1e9cea2c320430aa05": { spdx: "MIT", files: ["package.json"] },
    },
  },
  "parse-community/parse-server": { spdx: "Apache-2.0", files: ["LICENSE", "NOTICE"] },
  "pinojs/pino": { spdx: "MIT", files: ["LICENSE"] },
  "pnpm/pnpm": {
    spdx: "MIT",
    files: ["LICENSE"],
    at: {
      // The sample at this commit is under pnpr/npm/pnpr/, which its LICENSE.md puts under PolyForm Shield 1.0.0.
      "85cd31b9e5dcdb3a41c5edcee8b439768076ba39": { spdx: "LicenseRef-PolyForm-Shield-1.0.0", files: ["pnpr/npm/pnpr/LICENSE.md"] },
      "e02e667d91245cbcf80f4b0471767e006b6a5827": { spdx: "MIT", files: ["pnpm/LICENSE"] },
    },
  },
  "rollup/rollup": { spdx: "MIT", files: ["LICENSE-CORE.md"] },
  "sequelize/sequelize": {
    spdx: "MIT",
    files: ["LICENSE"],
    at: {
      // September 2010: no licence file, no `license` in package.json, nothing in the README (LICENSE landed 2011-05-03).
      "155381bd5ddfc8d4b7a6f1cd8385bfca60921ada": { spdx: "NONE", files: [] },
      "3ca94230c5e8c67fb3eca000da724f4d666d433a": { spdx: "NONE", files: [] },
      "4d39a3bd983833c2cc101696ce5bbad4bf7eb35f": { spdx: "NONE", files: [] },
      "c1ac2e7102e3e3cf48ac7c3a9e82c6378a44e2a7": { spdx: "NONE", files: [] },
      "f5c504a60f87e277d8cadd9cea57c47b018a2ae0": { spdx: "NONE", files: [] },
    },
  },
  "sidorares/node-mysql2": { spdx: "MIT", files: ["License"] },
  "socketio/socket.io": {
    spdx: "MIT",
    files: ["LICENSE"],
    at: {
      "000cba31e716a16d71c090f7b3bdd6e19c49b29e": { spdx: "MIT", files: ["packages/engine.io/LICENSE"] },
      "1d4269cee88586f90d491e532ca6f212bd4ab2ae": { spdx: "MIT", files: ["packages/socket.io-cluster-engine/LICENSE"] },
      "3b68658201c771b189b3d235458e2b0ac293dab2": { spdx: "MIT", files: ["packages/engine.io-client/LICENSE"] },
      "594841617d116c830609cd8f61eabcdab9d59cb1": { spdx: "MIT", files: ["packages/socket.io-redis-streams-emitter/LICENSE"] },
      "6868882eabf0b7ad25db3617cdf9c5892d6616d8": { spdx: "MIT", files: ["packages/socket.io-cluster-engine/LICENSE"] },
      "6bb2e7f5fb7c548f0776328b4a19719723e27efd": { spdx: "MIT", files: ["packages/socket.io/LICENSE"] },
      "7a33d9be8e8219c1b0581a77063d451d094c80d9": { spdx: "MIT", files: ["packages/socket.io-cluster-engine/LICENSE"] },
      "8af70195bb8c5bc3efe9685997ab6373fb8b1ca9": { spdx: "MIT", files: ["packages/socket.io/LICENSE"] },
      "8f4b825a763973bf4183b21955beb3b9f995fc33": { spdx: "MIT", files: ["packages/socket.io-cluster-engine/LICENSE"] },
      "91dd763b42cc8439b3bae1bfda120c5f60aeea0a": { spdx: "MIT", files: ["packages/engine.io/LICENSE"] },
      "a80711a8c3b11a3e75a84de567ac4d84820f4714": { spdx: "MIT", files: ["packages/engine.io/LICENSE"] },
      "ae7fb46e08c5ed964b4a1ea8b1703e816511598e": { spdx: "MIT", files: ["packages/engine.io/LICENSE"] },
      "b00124b65acdc0e373c0241c6a8e7f6406f0627b": { spdx: "MIT", files: ["packages/socket.io-clustered-engine/LICENSE"] },
      "d2d753fed4435015c2d83fe62e676b44e07fa3f7": { spdx: "MIT", files: ["packages/socket.io/LICENSE"] },
      "da04267ffc7b0903ca91f2fccb80e56246d13328": { spdx: "MIT", files: ["packages/socket.io/LICENSE"] },
      "dfb5ab3b6a2f00d1ecb226939022ea56e5701124": { spdx: "MIT", files: ["packages/socket.io/LICENSE"] },
      "e08293bc3735de5b824b347383e86e0b8ab9fbd5": { spdx: "MIT", files: ["packages/engine.io/LICENSE"] },
      "e347a3c24e773cf59f589110989fd56703a9057c": { spdx: "MIT", files: ["packages/socket.io/LICENSE"] },
    },
  },
  "storybookjs/storybook": { spdx: "MIT", files: ["LICENSE"] },
  "tryghost/ghost": {
    spdx: "MIT",
    files: ["LICENSE"],
    at: {
      "3365e513d715d4d77a8d6dfb217337f56a28ad31": { spdx: "MIT", files: ["ghost/portal/LICENSE"] },
      "385a49b4b4231b90acfc0bc0eaca85d8065937d0": { spdx: "MIT", files: ["koenig/koenig-lexical/LICENSE"] },
      "43280b7d75409c2c6ff5b5b1434a5577b97f6240": { spdx: "MIT", files: ["koenig/koenig-lexical/LICENSE"] },
      "aae59df7ae1092a4072dc66b49b8f14c8350abc4": { spdx: "MIT", files: ["koenig/kg-simplemde/LICENSE"] },
      "b219e26ea638d560003da782eab27e723b1a3090": { spdx: "MIT", files: ["ghost/members-api/static/auth/package.json"] },
      "cf9cee02085400ea60f1b74511cdcd8a7993cbb7": { spdx: "MIT", files: ["ghost/custom-theme-settings-service/LICENSE"] },
      // A commit of Ghost-Admin's history, holding only ghost/admin/: no licence file, no package.json, no README.
      "ddcb441268a409381373fe48a8d36d7125d2b6ce": { spdx: "NONE", files: [] },
      "e28415abf4363815a133743a546cdd1a2228639a": { spdx: "MIT", files: ["ghost/custom-theme-settings-service/LICENSE"] },
      "f783adbe7f67ffc3c6725c4bc27106eaca78cbc1": { spdx: "MIT", files: ["LICENSE.txt"] },
    },
  },
  "typeorm/typeorm": {
    spdx: "MIT",
    files: ["LICENSE"],
    at: {
      "5a96aa92827fc0803810016ed2e705885861f93c": { spdx: "MIT", files: ["LICENCE"] },
    },
  },
  "verdaccio/verdaccio": {
    spdx: "MIT",
    files: ["LICENSE"],
    at: {
      "0fc30510992197f0f3c8d94e976cac85fcdea5d8": { spdx: "MIT", files: ["packages/plugins/local-storage/LICENSE"] },
      "1905990ceaafb6f08a5e9e649391a52b26737f5a": { spdx: "MIT", files: ["packages/middleware/LICENSE"] },
      "31e18e8bf2a78ed510c0fafc9ca0a7869617ede0": { spdx: "MIT", files: ["packages/plugins/htpasswd/LICENSE"] },
      "7590fb51ff6ee49616dbedbbf48c706f76487eef": { spdx: "MIT", files: ["packages/store/LICENSE"] },
      "8bc332b77ddda7a7b4a1fbbf0c43bef6e8a9ecbc": { spdx: "MIT", files: ["packages/web/LICENSE"] },
      "ed518e805d14c9fd93c9197367296d29fe157e2c": { spdx: "MIT", files: ["packages/store/LICENSE"] },
    },
  },
  // packages/vite/LICENSE.md states Vite's MIT licence, then its bundled dependencies' licences (at 11243de and
  // 80d113b some copyleft, which the build refuses); the root LICENSE states the same MIT licence for those commits.
  "vitejs/vite": {
    spdx: "MIT",
    files: ["LICENSE"],
    at: {
      "1ea38e21c37cf8021940bd4554cc981ccfdcdf25": { spdx: "MIT", files: ["packages/vite/LICENSE.md"] },
      "df869909ae1bb5d7cd0fdd7b705737e9b19e41d4": { spdx: "MIT", files: ["packages/vite/LICENSE.md"] },
    },
  },
  "webdriverio/webdriverio": {
    spdx: "MIT",
    files: ["LICENSE"],
    at: {
      "7f473550e28ecd9bc5710622f975ee4b30fc8e21": { spdx: "MIT", files: ["packages/wdio-appium-service/LICENSE"] },
    },
  },
  "webpack/webpack": { spdx: "MIT", files: ["LICENSE"] },
  // The ESLint and React test sets' sources (scripts/samples/convert-eslint-test-set.ts), recorded as the Node test
  // set's are: each sample's file's nearest licence file at its commit, or with none the README's licence section or
  // package.json's `license`.
  "adobe/react-spectrum": {
    spdx: "Apache-2.0",
    files: ["LICENSE", "NOTICE.txt"],
    at: {
      // Adobe's internal library before the repository was opened: no licence file, no `license` in package.json.
      "1daa2f12b1292d4293f95144a38f9da31ff9b2d9": { spdx: "NONE", files: [] },
      // Adobe's internal library before the repository was opened: no licence file, no `license` in package.json.
      "7ef5fa31d5bcfb08abadeb5b33d09b5a6b9e73ea": { spdx: "NONE", files: [] },
      // v2-license.txt reads "ADOBE CONFIDENTIAL ... All Rights Reserved."; nothing else states a licence.
      "ebd2a7e70cbf71e4e72c79b4a138d58496d7e304": { spdx: "NONE", files: [] },
      // Adobe's internal library before the repository was opened: no licence file, no `license` in package.json.
      "efeeeacdab7cacf85bbba6183a7530e129b38be2": { spdx: "NONE", files: [] },
    },
  },
  "aidenybai/million": { spdx: "MIT", files: ["LICENSE"] },
  "akveo/react-native-ui-kitten": {
    spdx: "MIT",
    files: ["LICENSE.txt"],
    at: {
      "9e38cc23a151b5ac8eae1de9115091ca4d9db8c4": { spdx: "MIT", files: ["LICENSE.txt", "NOTICE"] },
    },
  },
  "angular/angular-cli": { spdx: "MIT", files: ["LICENSE"] },
  "ant-design/ant-design": { spdx: "MIT", files: ["LICENSE"] },
  "ant-design/ant-design-mobile": { spdx: "MIT", files: ["LICENSE.txt"] },
  "apache/superset": {
    spdx: "Apache-2.0",
    files: ["LICENSE.txt", "NOTICE"],
    at: {
      "096454e9cff12c85f66b14fdbf2144079c28db10": { spdx: "Apache-2.0", files: ["superset-frontend/temporary_superset_ui/superset-ui-plugins-deckgl/LICENSE"] },
      "0eafb6c1f70885c7510ca28f7bc84b4f5bfbcc2e": { spdx: "Apache-2.0", files: ["superset-frontend/temporary_superset_ui/superset-ui/plugins/superset-ui-plugins/LICENSE"] },
      "6c9be9d67bc73893055f989915ec4a43e7e848c1": { spdx: "Apache-2.0", files: ["LICENSE.txt"] },
      "7e0e7c89ba8c9384d0db4cf945e0a978910c3b18": { spdx: "Apache-2.0", files: ["LICENSE.txt"] },
      "ac90efaa40fc3ab62b93b4b08dd755b2c2e86bdd": { spdx: "Apache-2.0", files: ["superset-frontend/temporary_superset_ui/superset-ui/LICENSE"] },
      "cac4e07bfeb591719cf6a7122ee2934d27ecca17": { spdx: "Apache-2.0", files: ["superset-frontend/temporary_superset_ui/superset-ui-plugins-deckgl/LICENSE"] },
    },
  },
  "appsmithorg/appsmith": { spdx: "Apache-2.0", files: ["LICENSE"] },
  "apptension/saas-boilerplate": { spdx: "MIT", files: ["LICENSE"] },
  "boardgameio/boardgame.io": { spdx: "MIT", files: ["LICENSE"] },
  "casesandberg/react-color": {
    spdx: "MIT",
    files: ["LICENSE"],
    at: {
      "0d26cb64047638f6592e45ab8344df4179bcf57c": { spdx: "MIT", files: ["package.json"] },
      "75f6356d0546d55f553a1564c04adf5cb5202d57": { spdx: "MIT", files: ["package.json"] },
      "a17bde42bc19f2c787dc4fc6680946850a07fe88": { spdx: "MIT", files: ["package.json"] },
      "a39d12d8daabc129e52fb10ade7bfd0f7d9ba016": { spdx: "MIT", files: ["package.json"] },
    },
  },
  "ccch1mneyyy/dsh-tui": { spdx: "MIT", files: ["LICENSE"] },
  "chakra-ui/chakra-ui": { spdx: "MIT", files: ["LICENSE"] },
  "cheeriojs/cheerio": {
    spdx: "MIT",
    files: ["LICENSE"],
    at: {
      "3e2fed6c0598d330b1fd6059650fe69705a7fa51": { spdx: "MIT", files: ["Readme.md"] },
      "4e66a3b128cc4419ebcb00e6592a4c8d35905516": { spdx: "MIT", files: ["Readme.md"] },
      "525a1dfde6f40ffa289c061e024d7e191e2dea9f": { spdx: "MIT", files: ["Readme.md"] },
      "66c4a47c0dedd22c2204f90682be3209ef366377": { spdx: "MIT", files: ["Readme.md"] },
      "79d4e5e6f2d2310eed09b83f6ccfd9e4961aa4b1": { spdx: "MIT", files: ["Readme.md"] },
      "8b408011155bcdf5228e750cc0685317ce33ca52": { spdx: "MIT", files: ["Readme.md"] },
      "8cc9f7ba87638f658a750048866c180044808250": { spdx: "MIT", files: ["Readme.md"] },
      "9166c478eb82411a165e64a1b474494887dcfdd8": { spdx: "MIT", files: ["Readme.md"] },
      "a1f289680fe8d713ca7f6e41caf1ae61434f666d": { spdx: "MIT", files: ["Readme.md"] },
      "ac092744c1882e91550a7d80425680af252077a4": { spdx: "MIT", files: ["Readme.md"] },
      "c68028ad28f0df4e62c581863c7191ec086df68b": { spdx: "MIT", files: ["Readme.md"] },
      "c7a1d8e14c06378a79109b245d161989c8050e1b": { spdx: "MIT", files: ["Readme.md"] },
      "ca0b1f78922f16f8a84d7c0add79fb4c6888fb59": { spdx: "MIT", files: ["Readme.md"] },
      "cb1f2b3ad35a96f54b2513036143845220ff81ec": { spdx: "MIT", files: ["Readme.md"] },
      "cc27cab3d109b4c63a29b173dd9256aee981abda": { spdx: "MIT", files: ["Readme.md"] },
      "d7939d4d211c51a8ce14823e5a6e2a2e9ce5c3a1": { spdx: "MIT", files: ["Readme.md"] },
      "e12cef1131a165b1f8e19476b1c4d20604e78397": { spdx: "MIT", files: ["Readme.md"] },
      "ffb2c362ed85de48d24d4823ac5c2202a9402c92": { spdx: "MIT", files: ["Readme.md"] },
    },
  },
  "codastic/react-positioning-portal": { spdx: "MIT", files: ["LICENSE.md"] },
  "codeceptjs/codeceptjs": { spdx: "MIT", files: ["LICENSE"] },
  "colinhacks/zod": { spdx: "MIT", files: ["packages/zod/LICENSE"] },
  "darkreader/darkreader": {
    spdx: "MIT",
    files: ["LICENSE"],
    at: {
      // 2014: no licence file, no package.json, nothing in the README.
      "44a520a910c5c16cdec09c9b0694521ee6cd85d2": { spdx: "NONE", files: [] },
      // No licence file; package.json's `license` is ISC (MIT came later).
      "9a36c2a45e1b0e1fd5949d791b723564238dbb67": { spdx: "ISC", files: ["package.json"] },
      // 2014: no licence file, no package.json, nothing in the README.
      "c0346b6510f82bb5ca0aabfe1e7907d3c7da9679": { spdx: "NONE", files: [] },
      // No licence file; package.json's `license` is ISC (MIT came later).
      "ca28c39620d8dd3702d63a362842afd1609a65cc": { spdx: "ISC", files: ["package.json"] },
    },
  },
  "diegomura/react-pdf": {
    spdx: "MIT",
    files: ["LICENSE"],
    at: {
      "62c97745c5cd4fc36234de7a112d1023c49658ab": { spdx: "MIT", files: ["packages/pdfkit/LICENSE"] },
    },
  },
  "egovernments/digit-frontend": { spdx: "MIT", files: ["LICENSE"] },
  "enricoros/big-agi": { spdx: "MIT", files: ["LICENSE"] },
  "facebook/docusaurus": {
    spdx: "MIT",
    files: ["LICENSE"],
    at: {
      "be3d259ac8558d191821482e13807540e1d38bca": { spdx: "MIT", files: ["LICENSE.md"] },
      "fcee060f40668afd1ca4274dcc5c2081f20d4d7a": { spdx: "MIT", files: ["packages/docusaurus-plugin-ideal-image/src/theme/IdealImageLegacy/LICENSE"] },
    },
  },
  "globocom/megadraft": { spdx: "MIT", files: ["LICENSE"] },
  "gpujs/gpu.js": { spdx: "MIT", files: ["LICENSE"] },
  "hackjutsu/lepton": { spdx: "MIT", files: ["LICENSE"] },
  "ianstormtaylor/slate": {
    spdx: "MIT",
    files: ["License.md"],
    at: {
      // No licence file; package.json is private with no `license`; nothing in the Readme.
      "aba40a2aaf10e55548b2aff09e125f42845b13eb": { spdx: "NONE", files: [] },
    },
  },
  "itsriffchan/solar-gwa-calculator": { spdx: "MIT", files: ["LICENSE"] },
  "jameskerr/react-arborist": { spdx: "MIT", files: ["LICENSE"] },
  "jedwatson/react-select": { spdx: "MIT", files: ["LICENSE"] },
  "jitsi/jitsi-meet": { spdx: "Apache-2.0", files: ["LICENSE"] },
  "katex/katex": {
    spdx: "MIT",
    files: ["LICENSE"],
    at: {
      // 2014: no licence file, no `license` in package.json, no README.
      "4122fa2b99d57b9b43a0cec580d5783bcb8589a1": { spdx: "NONE", files: [] },
      // 2014: no licence file, no `license` in package.json, no README.
      "ee58c126fb29155360316905a17e0e742b7c2d83": { spdx: "NONE", files: [] },
    },
  },
  "khan/perseus": { spdx: "MIT", files: ["LICENSE"] },
  "legendapp/legend-list": { spdx: "MIT", files: ["LICENSE"] },
  "mantinedev/mantine": { spdx: "MIT", files: ["LICENSE"] },
  "marmelab/react-admin": {
    spdx: "MIT",
    files: ["LICENSE.md"],
    at: {
      "0cbd671e90d7bfc30866b657349f537cb16ec0e4": { spdx: "MIT", files: ["package.json"] },
      "98647168ba52aae2325ac89967906816ec58d992": { spdx: "MIT", files: ["package.json"] },
      "b00187d697e090cf0a57b3a296acabd75191e372": { spdx: "MIT", files: ["example/package.json"] },
    },
  },
  "mdx-editor/editor": { spdx: "MIT", files: ["LICENSE"] },
  "mermaid-js/mermaid": { spdx: "MIT", files: ["LICENSE"] },
  "mobxjs/mobx": {
    spdx: "MIT",
    files: ["LICENSE"],
    at: {
      "021f34ec81daed9e5b5ed8425b2f3e0fa85dfe5b": { spdx: "MIT", files: ["packages/eslint-plugin-mobx/LICENSE"] },
      "2426373f03f7225096d7b528d5934235dd0301e1": { spdx: "MIT", files: ["packages/eslint-plugin-mobx/LICENSE"] },
      "2587df31a1a967a6b385b7ab2d9f0d42fc94e4b0": { spdx: "MIT", files: ["packages/mobx-react-lite/LICENSE"] },
      "2d92ca775fc1a0f57fa2a3bc4b711fc1fed68cfa": { spdx: "MIT", files: ["packages/mobx-undecorate/LICENSE"] },
      "2dcfec093533bd12bb564580b14ce6037ee1ebac": { spdx: "MIT", files: ["packages/mobx-react-lite/LICENSE"] },
      "44a2cf42dec7635f639ddbfb19202ebc710bac54": { spdx: "MIT", files: ["packages/mobx-react-lite/LICENSE", "packages/mobx-react/LICENSE"] },
      "473cb3f5fc8bf43abdd1c9c7857fe2820d2291fe": { spdx: "MIT", files: ["packages/mobx-react-lite/LICENSE"] },
      "4ef8ff3f84ec8ae893d8c84031664ea388d78091": { spdx: "MIT", files: ["packages/mobx-react-lite/LICENSE"] },
      "638533e592f4fda7663fa351447380f9d0917d14": { spdx: "MIT", files: ["packages/eslint-plugin-mobx/LICENSE"] },
      "6aa5ea379ca77bd6d5e3f2505e5c37227276f17e": { spdx: "MIT", files: ["package.json"] },
      "6ee9fd0dcd1179fcd326a06dc1a3782b24b58d4c": { spdx: "MIT", files: ["package.json"] },
      "751b6d9f406ca35994b3b143716bc5f6352ba3a5": { spdx: "MIT", files: ["package.json"] },
      "87b3e1de58069617a39552d71a4d5c5c134cbbaf": { spdx: "MIT", files: ["packages/mobx-react/LICENSE"] },
      "87e5dfb52a84869d62b6343887a1c8659aee595d": { spdx: "MIT", files: ["packages/mobx-react-lite/LICENSE"] },
      "9ecff3619122d7d8d67b93f636d0519ff79a725d": { spdx: "MIT", files: ["package.json"] },
      "aafda6136afd107c6cbdd73d9bab57e45a2eb9f5": { spdx: "MIT", files: ["packages/eslint-plugin-mobx/LICENSE"] },
      "ade1306cc0c5fb1c0b9f1a5d7855bf4ed766358d": { spdx: "MIT", files: ["packages/mobx-react/LICENSE"] },
      "bbcb12dc754524552181b177a52ffdbe80ecb953": { spdx: "MIT", files: ["packages/mobx-react/LICENSE"] },
      "bd4b70d8ded29673af8161aa42fb88dc4ad4420e": { spdx: "MIT", files: ["packages/mobx-react-lite/LICENSE"] },
    },
  },
  "mui/material-ui": { spdx: "MIT", files: ["LICENSE"] },
  "nasa-impact/mmgis": { spdx: "Apache-2.0", files: ["LICENSE"] },
  "notionnext-org/notionnext": { spdx: "MIT", files: ["LICENSE"] },
  "openrin/rin": { spdx: "MIT", files: ["LICENSE"] },
  "openstackweb/osf-website": { spdx: "MIT", files: ["LICENSE"] },
  "opentiny/tiny-charts": { spdx: "MIT", files: ["LICENSE"] },
  "palantir/blueprint": {
    spdx: "Apache-2.0",
    files: ["packages/core/LICENSE"],
    at: {
      "0ac4b32bea3df51e56558c8c55d55cd9c2ed6c32": { spdx: "Apache-2.0", files: ["packages/datetime/LICENSE"] },
      "0b857f875f4e89a04cf44b191bc1b200a6bcd7ce": { spdx: "Apache-2.0", files: ["packages/docs-theme/LICENSE"] },
      "166d3dbda85dc19d14f6fe6d1be93084a19fe435": { spdx: "Apache-2.0", files: ["LICENSE"] },
      "1b7fa75f2f8ac370c0e6231ec7af54388a1dcb00": { spdx: "Apache-2.0", files: ["LICENSE"] },
      "3e77b386fd5d150f70d15c3617491a056f028c79": { spdx: "Apache-2.0", files: ["packages/table/LICENSE"] },
      "42a0c1a68038836cb8df08c935e2ce7c7a278e6a": { spdx: "Apache-2.0", files: ["packages/datetime2/LICENSE"] },
      "5a868f6fe1c842ee06c9dca48d6a41e8106bf728": { spdx: "Apache-2.0", files: ["LICENSE"] },
      "5adc1326a053f71c772593133a965068cd38ff1b": { spdx: "Apache-2.0", files: ["packages/popover2/LICENSE"] },
      "5d1b1f500954d0953dfc7012f652646d474541dc": { spdx: "Apache-2.0", files: ["packages/datetime/LICENSE"] },
      "5f40f68063e0f1436bcf8c6bc7690235308ee6fa": { spdx: "Apache-2.0", files: ["packages/datetime2/LICENSE"] },
      "90b51cfdf49d815cda4d0a37c8bba0b6f35ed9cb": { spdx: "Apache-2.0", files: ["packages/datetime2/LICENSE"] },
      "9b802f62931aac400ad5d6ec5489fe52e4215567": { spdx: "Apache-2.0", files: ["packages/datetime/LICENSE"] },
      "aa7c73670a867f3244f4001d83436244f4a0afc1": { spdx: "Apache-2.0", files: ["packages/docs-theme/LICENSE"] },
      "bdeff6e3bd09b1ce8d5974fda37d8a4da836f5b2": { spdx: "Apache-2.0", files: ["packages/datetime2/LICENSE"] },
      "cf03517a6a4e8a3981917d4193ece0fa7c1208d5": { spdx: "Apache-2.0", files: ["packages/docs-theme/LICENSE"] },
      "e67c6fe313627977107ac9036817a88cb9532c0d": { spdx: "Apache-2.0", files: ["LICENSE"] },
    },
  },
  "pascalorg/editor": { spdx: "MIT", files: ["packages/editor/LICENSE"] },
  "pellenybe/crypto-mcp-server---by-corax-colab": { spdx: "MIT", files: ["LICENSE"] },
  "polarsource/polar": {
    spdx: "Apache-2.0",
    files: ["LICENSE"],
    at: {
      "0563de373501da7e4bd90217f6b03586c8e3529c": { spdx: "Apache-2.0", files: ["clients/apps/app/LICENSE"] },
      "553995b34076e08069247370bde0805e01808f42": { spdx: "Apache-2.0", files: ["clients/apps/app/LICENSE"] },
    },
  },
  "postcss/postcss": { spdx: "MIT", files: ["LICENSE"] },
  "preactjs/signals": { spdx: "MIT", files: ["LICENSE"] },
  "pubkey/rxdb": { spdx: "Apache-2.0", files: ["LICENSE.txt"] },
  "radix-ui/primitives": { spdx: "MIT", files: ["LICENSE"] },
  "react-bootstrap/react-bootstrap": { spdx: "MIT", files: ["LICENSE"] },
  "react-dates/react-dates": { spdx: "MIT", files: ["LICENSE"] },
  "react-dnd/react-dnd": {
    spdx: "MIT",
    files: ["LICENSE"],
    at: {
      // Facebook's BSD licence, with no patent grant at this commit.
      "130e2423d1cb6be5694dd95897f829207e51dbc0": { spdx: "BSD-3-Clause", files: ["site/LICENSE"] },
      "4b4337571d18cb2b54a5003b7fb688987c8cafaf": { spdx: "MIT", files: ["packages/core/react-dnd/LICENSE"] },
      // Facebook's BSD licence, with no patent grant at this commit.
      "659bef204c6b0d185d0396a313beaeb44b5f0752": { spdx: "BSD-3-Clause", files: ["LICENSE"] },
      // Facebook's BSD licence, with no patent grant at this commit.
      "694772d80f3a8b014c56820d3312f18b5fc49442": { spdx: "BSD-3-Clause", files: ["site/LICENSE"] },
      "6b1b0501f7cc62acbaa75bdd00d50668fb886307": { spdx: "MIT", files: ["packages/react-dnd/LICENSE"] },
      // Facebook's BSD licence, with no patent grant at this commit.
      "eb9effa595663a2577d62dedc43478ec0f5c0215": { spdx: "BSD-3-Clause", files: ["LICENSE", "packages/documentation/site/LICENSE"] },
      "f67aa53703dad584baeadad964053c9b50413eb9": { spdx: "MIT", files: ["packages/react-dnd/LICENSE"] },
      "f78b6b91f85dc8a4b2aa1bf696c1f837c855a347": { spdx: "MIT", files: ["packages/react-dnd/LICENSE"] },
    },
  },
  "reactjs/react-modal": { spdx: "MIT", files: ["LICENSE"] },
  "redis/ioredis": { spdx: "MIT", files: ["LICENSE"] },
  "redwoodjs/graphql": { spdx: "MIT", files: ["LICENSE"] },
  "reececorkhill/plan-it": { spdx: "MIT", files: ["LICENSE"] },
  "remix-run/react-router": { spdx: "MIT", files: ["LICENSE.md"] },
  "rwm6857/gracechords": { spdx: "Apache-2.0", files: ["LICENSE", "NOTICE"] },
  "scimmyjs/scimmy": { spdx: "MIT", files: ["LICENSE.md"] },
  "streamlit/streamlit": {
    spdx: "Apache-2.0",
    files: ["LICENSE"],
    at: {
      // 2018, before Streamlit was open source: LICENSE reads only "Copyright 2018 Streamlit Inc. All rights reserved."
      "cc1fe5800950d592c186ac484e4921013a710d8a": { spdx: "NONE", files: [] },
    },
  },
  "styled-components/styled-components": {
    spdx: "MIT",
    files: ["packages/styled-components/LICENSE"],
    at: {
      "44e565bf5d775da06f1356373384b80f7cc1c17f": { spdx: "MIT", files: ["LICENSE"] },
      "5431a8b15044439dc62b4de5e1be62cd549f5362": { spdx: "MIT", files: ["LICENSE", "packages/benchmarks/LICENSE.md"] },
      "944f85018db9745af7ec260558dd5faafae4f811": { spdx: "MIT", files: ["packages/benchmarks/LICENSE.md"] },
      "e2db45dc0c6bd4a051209776cbb6449abab28e0b": { spdx: "MIT", files: ["LICENSE"] },
    },
  },
  "supabase/supabase": { spdx: "Apache-2.0", files: ["LICENSE"] },
  "tadanobutubutu/screeps": { spdx: "MIT", files: ["LICENSE"] },
  "tinacms/tinacms": { spdx: "Apache-2.0", files: ["LICENSE"] },
  "usebruno/bruno": {
    spdx: "MIT",
    files: ["license.md"],
    at: {
      "0b3f5100e73572c17ecdde26f73ad078e52a4184": { spdx: "MIT", files: ["packages/bruno-cli/license.md"] },
      "1622ee0c9dd2e2d56b27852c2366dccc0c382cf6": { spdx: "MIT", files: ["packages/bruno-cli/license.md", "packages/bruno-js/license.md"] },
      "23e1a987129c173569aba75365a182f440a15c7b": { spdx: "MIT", files: ["packages/bruno-cli/license.md"] },
      "304973dc1215c278dd667571c8d59493d0bfc9d4": { spdx: "MIT", files: ["packages/bruno-sqlite/LICENSE.md"] },
      "31027cb2e07b8ec6c592b5c84cbe7d7e25b1b5a6": { spdx: "MIT", files: ["packages/bruno-common/license.md"] },
      "3169e6cdf4d1cfa55e922a9ffa6b6534a2e7a4b4": { spdx: "MIT", files: ["license.md", "packages/bruno-lang/license.md"] },
      "3c576487c974c3c48d07bed73e134313c2779cbc": { spdx: "MIT", files: ["packages/bruno-js/license.md"] },
      "4badee903affef2f5250ac18883ed3f4f267d453": { spdx: "MIT", files: ["packages/bruno-filestore/LICENSE.md"] },
      "59dfa97ea785d7b51120b1bcc001562840437242": { spdx: "MIT", files: ["packages/bruno-converters/license.md"] },
      "a4d0665ccf0f434332e7a6e5ff4f8eac316ff386": { spdx: "MIT", files: ["packages/bruno-converters/license.md"] },
      "b1911d80e9150f3b0e1654a6828ca283919ce400": { spdx: "MIT", files: ["packages/bruno-lang/license.md"] },
      "bc90e46b288446ebe0c8d67b28470d2efc0a8c48": { spdx: "MIT", files: ["packages/bruno-converters/license.md"] },
      "cedf8381380dc547f37405e83eb88e4db98507de": { spdx: "MIT", files: ["packages/bruno-lang/license.md"] },
    },
  },
  "walkover-web-solution/giddh-new-angular4-app": { spdx: "MIT", files: ["LICENSE"] },
  "web-scrobbler/web-scrobbler": { spdx: "MIT", files: ["LICENSE.md"] },
  "wekan/wekan": {
    spdx: "MIT",
    files: ["LICENSE"],
    at: {
      "00768b4392109dec62a4dcf44dbceb295990d785": { spdx: "MIT", files: ["packages/wekan-ldap/LICENSE"] },
      "40265144af880893398f4befa3f5796b76609aba": { spdx: "MIT", files: ["packages/wekan-tap-i18n/LICENSE"] },
      "7dfd942ff67717164bf9b3c18f058d6a6cf2f4a6": { spdx: "MIT", files: ["packages/wekan-ldap/LICENSE"] },
      "b933a69cd4953543cece844865ab7dc72eb4a6c1": { spdx: "MIT", files: ["LICENSE", "packages/wekan-ldap/LICENSE"] },
      // The sample is under packages/wekan-oidc/, whose LICENSE.txt is Apache-2.0.
      "ccb5c28b7689d4428db6448c71987b1891eaf19f": { spdx: "Apache-2.0", files: ["packages/wekan-oidc/LICENSE.txt"] },
      "ffdeed099e72c25e57fa36e69fe2d1a218a4a145": { spdx: "MIT", files: ["packages/wekan-fontawesome/fontawesome-free/LICENSE.txt"] },
    },
  },
  "widergy/react-native-credit-card-input-tmp": { spdx: "MIT", files: ["LICENSE"] },
  "xyflow/xyflow": { spdx: "MIT", files: ["LICENSE"] },
  "yikart/aitoearn": { spdx: "MIT", files: ["LICENSE.txt"] },
};

/**
 * The recorded SPDX licence of `repo` (any case) at `commit` (a full sha; the repository's own licence when omitted or
 * when the commit has no entry of its own), or undefined when none is recorded. Sample publish (`publishedSamples`)
 * refuses a repository with none and a declared `source.license` that is not this value at the sample's commit.
 */
export function recordedLicence(repo: string, commit?: string): string | undefined {
  const source = SOURCE_LICENCES[repo.toLowerCase()];
  return source === undefined ? undefined : licenceAt(source, commit).spdx;
}

/** Every licence recorded for `repo` (any case) at any of its commits, sorted; empty when none is recorded. */
export function recordedLicences(repo: string): string[] {
  const source = SOURCE_LICENCES[repo.toLowerCase()];
  if (source === undefined) return [];
  return [...new Set([source.spdx, ...Object.values(source.at ?? {}).map((licence) => licence.spdx)])].sort(byText);
}

/** What each SPDX id's licence text must say, so a table entry that no longer matches its file stops the build. */
const LICENCE_MARKERS: Readonly<Record<string, readonly RegExp[]>> = {
  MIT: [/Permission is hereby granted, free of charge/u],
  "BSD-2-Clause": [/Redistribution and use in source and binary forms/u],
  // The non-endorsement clause, however it wraps: "Neither the name ...", or older texts' "may not be used to endorse
  // or\n promote" (Twig's before 2020).
  "BSD-3-Clause": [/Redistribution and use in source and binary forms/u, /Neither\s+the\s+name|endorse\s+or\s+promote/u],
  "Apache-2.0": [/Apache License,?\s+Version 2\.0/u],
  PostgreSQL: [/Permission to use, copy, modify, and distribute this software and its\s+documentation for any purpose, without fee/u],
  Unlicense: [/free and unencumbered software released into the public domain/u],
  // "and/or ... is hereby granted." with a full stop: ISC's grant goes on ", provided that ...".
  "0BSD": [/Permission to use, copy, modify, and\/or distribute this software for any\s+purpose with or without fee is hereby granted\./u],
  // Recorded so a commit's licence is the true one; the licence allowlist refuses both at publish.
  "CC-BY-4.0": [/Attribution 4\.0 International/u],
  "LicenseRef-PolyForm-Shield-1.0.0": [/PolyForm Shield License 1\.0\.0/u],
};
const COPYLEFT_TEXT = /GNU (?:AFFERO |LESSER )?GENERAL PUBLIC LICENSE|Mozilla Public License/iu;

/** One licence file of a repository at a commit. */
export interface LicenceText {
  file: string;
  text: string;
}

/**
 * Whether `texts` state the licence term `term`: a package.json whose `license` is `term` (an npm package's own
 * statement, for a commit with no licence file), or every marker of `term` in the other files.
 */
function states(texts: readonly LicenceText[], term: string): boolean {
  // SPDX NONE: nothing at the commit states a licence, so nothing is read; the licence allowlist refuses it at publish.
  if (term === "NONE") return texts.length === 0;
  const declared = texts.filter(({ file }) => basename(file) === "package.json").map(({ text }) => packageLicence(text));
  if (declared.includes(term)) return true;
  const markers = LICENCE_MARKERS[term];
  const all = texts.filter(({ file }) => basename(file) !== "package.json").map(({ text }) => text).join("\n");
  return markers !== undefined && all !== "" && markers.every((marker) => marker.test(all));
}

function packageLicence(text: string): unknown {
  try {
    const parsed: unknown = JSON.parse(text);
    return typeof parsed === "object" && parsed !== null && "license" in parsed ? parsed.license : undefined;
  } catch {
    return undefined;
  }
}

/**
 * A repository's licence files at a commit, read once from GitHub and then from the builder's cache (keyed by
 * repository, commit and file), and checked: they state the table's licence at that commit and no copyleft one.
 */
export function licenceTexts(cacheRoot: string, repo: string, commit: string): { spdx: string; texts: LicenceText[] } {
  const source = SOURCE_LICENCES[repo.toLowerCase()];
  if (source === undefined) throw new Error(`${repo}: scripts/samples/licences.ts records no licence files for it (SOURCE_LICENCES); read its licence files at ${commit} and add them`);
  const licence = licenceAt(source, commit);
  const texts = licence.files.map((file) => {
    const cached = join(cacheRoot, "licences", repo.toLowerCase(), commit, file);
    if (!existsSync(cached)) {
      let text: string;
      try {
        text = execFileSync("gh", ["api", "-H", "Accept: application/vnd.github.raw", `repos/${repo}/contents/${file}?ref=${commit}`], { encoding: "utf8", maxBuffer: 16 * 1024 * 1024, stdio: ["ignore", "pipe", "pipe"] });
      } catch (error) {
        throw new Error(`${repo} at ${commit}: ${file}, which scripts/samples/licences.ts records for this commit, could not be read from GitHub (${ghFailure(error)})`);
      }
      mkdirSync(dirname(cached), { recursive: true });
      writeFileSync(cached, text);
    }
    return { file, text: readFileSync(cached, "utf8") };
  });
  if (COPYLEFT_TEXT.test(texts.map((entry) => entry.text).join("\n"))) throw new Error(`${repo} at ${commit}: its licence files name a copyleft licence`);
  for (const term of licence.spdx.split(/\s+(?:OR|AND)\s+/u)) {
    if (!states(texts, term)) throw new Error(`${repo} at ${commit}: ${licence.files.join(", ")} do not read as ${term}`);
  }
  return { spdx: licence.spdx, texts };
}

/** What a failed `gh` call said: the last line of its stderr (`gh: Not Found (HTTP 404)`), else the error's message. */
function ghFailure(error: unknown): string {
  const stderr = typeof error === "object" && error !== null && "stderr" in error ? error.stderr : undefined;
  if (typeof stderr === "string" && stderr.trim() !== "") return stderr.trim().split("\n").at(-1)!;
  return error instanceof Error ? error.message : String(error);
}

const fence = (text: string): string => {
  const longest = Math.max(2, ...[...text.matchAll(/`+/gu)].map((match) => match[0].length));
  return "`".repeat(longest + 1);
};

/** The repositories of `samples` that `texts` has a licence text for at any of their commits, sorted. */
function reposWithText(samples: readonly Sample[], texts: (repo: string, commit: string) => LicenceText[] | undefined): string[] {
  const repos = [...new Set(samples.map((sample) => sample.source.repo))].sort(byText);
  return repos.filter((repo) => samples.some((sample) => sample.source.repo === repo && (texts(repo, sample.source.commit)?.length ?? 0) > 0));
}

/**
 * What the archive's LICENSES.md holds, for SAMPLES.md: licence texts for every repository, for none (a set converted
 * from a test set with only the licence id the test set recorded per file, as the ESLint test set was), or for some.
 */
export function licensesClaim(repos: number, withText: readonly string[]): string {
  if (withText.length === repos) return "The archive's `LICENSES.md` lists every file with its repository's licence text.";
  const ids = "the licence id its test set recorded, which publish holds to the one `scripts/samples/licences.ts` records for the repository at the file's commit";
  if (withText.length === 0) return `The archive's \`LICENSES.md\` lists every file with its repository's licence: ${ids}. It holds no licence texts.`;
  return `The archive's \`LICENSES.md\` lists every file with its repository's licence. It holds the licence text of ${withText.join(", ")} only; for every other repository it holds ${ids}, and no licence text.`;
}

/**
 * The archive's LICENSES.md: per source repository its licence, the files taken from it (as the archive holds them),
 * and the licence texts read at their commits; a repository with none carries the licence its test set recorded.
 */
export function licensesMarkdown(samples: readonly Sample[], texts: (repo: string, commit: string) => LicenceText[] | undefined): string {
  const byRepo = new Map<string, Sample[]>();
  for (const sample of samples) byRepo.set(sample.source.repo, [...(byRepo.get(sample.source.repo) ?? []), sample]);
  const allText = reposWithText(samples, texts).length === byRepo.size;
  const lines = [
    "# Sources",
    "",
    `Every sample in this archive is a copy of one file of an open-source repository, at the commit its \`.expected.json\` names (\`source.commit\`; for a SWE task's file, that commit with the task's reference solution applied). Each repository below is listed with its licence, the files taken from it, and ${allText ? "its licence text as read at those commits" : "its licence text as read at those commits where one was read (otherwise the licence its test set recorded)"}. The files remain under their repositories' licences.`,
    "",
  ];
  for (const repo of [...byRepo.keys()].sort(byText)) {
    const taken = byRepo.get(repo)!;
    const licences = [...new Set(taken.map((sample) => sample.source.license))].sort(byText);
    const commits = [...new Set(taken.map((sample) => sample.source.commit))].sort(byText);
    lines.push(`## ${repo}`, "", `License: ${licences.join(", ")}. ${taken.length} file(s) from ${commits.length} commit(s).`, "");
    for (const sample of [...taken].sort((a, b) => byText(a.source.path, b.source.path) || byText(a.source.sha256, b.source.sha256))) {
      lines.push(`- \`${sampleId(sample.short, sample.source.path, sample.source.sha256)}/${sample.source.path}\` (${sample.source.commit.slice(0, 12)})`);
    }
    lines.push("");
    const seen = new Set<string>();
    let anyText = false;
    for (const commit of commits) {
      for (const { file, text } of texts(repo, commit) ?? []) {
        anyText = true;
        if (seen.has(`${file}\0${text}`)) continue;
        seen.add(`${file}\0${text}`);
        const commitsWithText = commits.filter((other) => texts(repo, other)?.some((entry) => entry.file === file && entry.text === text));
        lines.push(`### ${file} (at ${commitsWithText.map((other) => other.slice(0, 12)).join(", ")})`, "", fence(text), text.trimEnd(), fence(text), "");
      }
    }
    if (!anyText) lines.push("The licence is the one its test set recorded for each file (`source.license`).", "");
  }
  return `${lines.join("\n").trimEnd()}\n`;
}

/**
 * The tool-pack's SAMPLES.md, committed beside samples.lock: which repositories the sample set takes files from,
 * under which licence and at which commits, and the archive, with what its LICENSES.md holds (`texts`, as for
 * licensesMarkdown).
 */
export function samplesMarkdown(
  toolPack: string,
  lock: { sha256: string; files: number; bytes: number },
  samples: readonly Sample[],
  texts: (repo: string, commit: string) => LicenceText[] | undefined,
): string {
  const byRepo = new Map<string, Sample[]>();
  for (const sample of samples) byRepo.set(sample.source.repo, [...(byRepo.get(sample.source.repo) ?? []), sample]);
  const lines = [
    `# Real-code samples: ${toolPack}`,
    "",
    `The sample set \`samples.lock\` pins (archive \`rerail/archives/samples/${toolPack}/${lock.sha256}.tar.zst\`, ${lock.files} files, ${lock.bytes} bytes) holds ${samples.length} sample(s): files copied from the repositories below, each under its repository's licence. ${licensesClaim(byRepo.size, reposWithText(samples, texts))} None of it is published with the pack.`,
    "",
    "| Repository | Licence | Commits | Files |",
    "|---|---|---|---:|",
  ];
  for (const repo of [...byRepo.keys()].sort(byText)) {
    const taken = byRepo.get(repo)!;
    const commits = [...new Set(taken.map((sample) => sample.source.commit.slice(0, 12)))].sort(byText);
    const shown = commits.length > 3 ? `${commits.slice(0, 3).join(", ")} and ${commits.length - 3} more` : commits.join(", ");
    lines.push(`| ${repo} | ${[...new Set(taken.map((sample) => sample.source.license))].sort(byText).join(", ")} | ${shown} | ${taken.length} |`);
  }
  return `${lines.join("\n")}\n`;
}
