/**
 * The licence of every source repository a sample is taken from. A sample carries its repository's SPDX licence,
 * read from the repository's licence file at the sample's commit; the archive's `LICENSES.md` carries those licence
 * texts and the files taken under them. Copyleft sources are never taken: the six source repositories found strong
 * copyleft at their pinned commits, and the one pinned corpus under MPL-2.0.
 */
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";

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
interface SourceLicence {
  spdx: string;
  files: readonly string[];
}

/**
 * Every permissive source repository a sample is taken from, with the licence its licence files state at the pinned
 * commits (checked against the text on every build, `licenceTexts`).
 */
export const SOURCE_LICENCES: Readonly<Record<string, SourceLicence>> = {
  "microsoft/vscode": { spdx: "MIT", files: ["LICENSE.txt"] },
  "excalidraw/excalidraw": { spdx: "MIT", files: ["LICENSE"] },
  "shadcn-ui/ui": { spdx: "MIT", files: ["LICENSE.md"] },
  "vercel/next.js": { spdx: "MIT", files: ["license.md"] },
  "vercel/commerce": { spdx: "MIT", files: ["license.md"] },
  "element-plus/element-plus": { spdx: "MIT", files: ["LICENSE"] },
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
  // The repository is partly public domain and partly BSD-2-Clause (LICENSE.rst); the file taken from it,
  // lib/Crypto/IO/PEM.py, names BSD-2-Clause in its own SPDX licence header.
  "legrandin/pycryptodome": { spdx: "BSD-2-Clause", files: ["LICENSE.rst"] },
};

/**
 * Source repositories whose samples come from the ESLint test set (scripts/samples/convert-eslint-test-set.ts) or the
 * Node test set (the node tool-pack's originals and fixed copies), with the licence the test set recorded for them, as
 * the committed `SAMPLES.md` tables list it. Their licence texts are not
 * read on build; a sample's `source.license` must still equal the entry.
 */
const TEST_SET_LICENCES: Readonly<Record<string, string>> = {
  "aidenybai/million": "MIT",
  "angular/angular-cli": "MIT",
  "appium/appium": "Apache-2.0",
  "automattic/mongoose": "MIT",
  "axios/axios": "MIT",
  "babel/babel": "MIT",
  "balderdashy/sails": "MIT",
  "boardgameio/boardgame.io": "MIT",
  "brianc/node-postgres": "MIT",
  "cheeriojs/cheerio": "MIT",
  "codeceptjs/codeceptjs": "MIT",
  "colinhacks/zod": "MIT",
  "cypress-io/cypress": "MIT",
  "darkreader/darkreader": "MIT",
  "dherault/serverless-offline": "MIT",
  "diegomura/react-pdf": "MIT",
  "discordjs/discord.js": "Apache-2.0",
  "electron/electron": "MIT",
  "enricoros/big-agi": "MIT",
  "eslint/eslint": "MIT",
  "ether/etherpad": "Apache-2.0",
  "expressjs/express": "MIT",
  "facebook/docusaurus": "MIT",
  "feathersjs/feathers": "MIT",
  "gatsbyjs/gatsby": "MIT",
  "getsentry/sentry-javascript": "MIT",
  "gpujs/gpu.js": "MIT",
  "hexojs/hexo": "MIT",
  "ianstormtaylor/slate": "MIT",
  "iobroker/iobroker.js-controller": "MIT",
  "itsriffchan/solar-gwa-calculator": "MIT",
  "jestjs/jest": "MIT",
  "karma-runner/karma": "MIT",
  "katex/katex": "MIT",
  "knex/knex": "MIT",
  "kong/insomnia": "Apache-2.0",
  "lerna/lerna": "MIT",
  "less/less.js": "Apache-2.0",
  "louislam/uptime-kuma": "MIT",
  "magicmirrororg/magicmirror": "MIT",
  "marmelab/react-admin": "MIT",
  "mde/ejs": "Apache-2.0",
  "mermaid-js/mermaid": "MIT",
  "meteor/meteor": "MIT",
  "mobxjs/mobx": "MIT",
  "mochajs/mocha": "MIT",
  "mongodb/node-mongodb-native": "Apache-2.0",
  "nasa-impact/mmgis": "Apache-2.0",
  "nestjs/nest": "MIT",
  "node-fetch/node-fetch": "MIT",
  "node-red/node-red": "Apache-2.0",
  "nodejs/undici": "MIT",
  "notionnext-org/notionnext": "MIT",
  "nuxt/nuxt": "MIT",
  "opentiny/tiny-charts": "MIT",
  "parse-community/parse-server": "Apache-2.0",
  "pinojs/pino": "MIT",
  "pnpm/pnpm": "MIT",
  "postcss/postcss": "MIT",
  "redis/ioredis": "MIT",
  "redwoodjs/graphql": "MIT",
  "reececorkhill/plan-it": "MIT",
  "rollup/rollup": "MIT",
  "scimmyjs/scimmy": "MIT",
  "sequelize/sequelize": "MIT",
  "sidorares/node-mysql2": "MIT",
  "socketio/socket.io": "MIT",
  "storybookjs/storybook": "MIT",
  "tadanobutubutu/screeps": "MIT",
  "tryghost/ghost": "MIT",
  "typeorm/typeorm": "MIT",
  "usebruno/bruno": "MIT",
  "verdaccio/verdaccio": "MIT",
  "vitejs/vite": "MIT",
  "walkover-web-solution/giddh-new-angular4-app": "MIT",
  "web-scrobbler/web-scrobbler": "MIT",
  "webdriverio/webdriverio": "MIT",
  "webpack/webpack": "MIT",
  "wekan/wekan": "MIT",
};

/**
 * Every source repository a sample may be taken from (`owner/name`, lowercase) and its SPDX licence: SOURCE_LICENCES
 * and the ESLint test set's sources. Sample publish (`publishedSamples`) refuses a repository missing here and a
 * declared `source.license` that is not this value; the approver summary lists repositories by it.
 */
export const RECORDED_LICENCES: Readonly<Record<string, string>> = {
  ...Object.fromEntries(Object.entries(SOURCE_LICENCES).map(([repo, licence]) => [repo, licence.spdx])),
  ...TEST_SET_LICENCES,
};

/** The recorded SPDX licence of `repo` (any case), or undefined when none is recorded. */
export const recordedLicence = (repo: string): string | undefined => RECORDED_LICENCES[repo.toLowerCase()];

/** What each SPDX id's licence text must say, so a table entry that no longer matches its file stops the build. */
const LICENCE_MARKERS: Readonly<Record<string, readonly RegExp[]>> = {
  MIT: [/Permission is hereby granted, free of charge/u],
  "BSD-2-Clause": [/Redistribution and use in source and binary forms/u],
  "BSD-3-Clause": [/Redistribution and use in source and binary forms/u, /Neither the name|endorse or promote/u],
  "Apache-2.0": [/Apache License,?\s+Version 2\.0/u],
  PostgreSQL: [/Permission to use, copy, modify, and distribute this software and its\s+documentation for any purpose, without fee/u],
  Unlicense: [/free and unencumbered software released into the public domain/u],
};
const COPYLEFT_TEXT = /GNU (?:AFFERO |LESSER )?GENERAL PUBLIC LICENSE|Mozilla Public License/iu;

/** One licence file of a repository at a commit. */
export interface LicenceText {
  file: string;
  text: string;
}

/**
 * A repository's licence files at a commit, read once from GitHub and then from the builder's cache (keyed by
 * repository, commit and file), and checked: they state the table's licence and no copyleft one.
 */
export function licenceTexts(cacheRoot: string, repo: string, commit: string): { spdx: string; texts: LicenceText[] } {
  const licence = SOURCE_LICENCES[repo.toLowerCase()];
  if (licence === undefined) throw new Error(`${repo}: no licence is recorded for it in scripts/samples/licences.ts; read its licence at ${commit} and add it`);
  const texts = licence.files.map((file) => {
    const cached = join(cacheRoot, "licences", repo.toLowerCase(), commit, file);
    if (!existsSync(cached)) {
      const text = execFileSync("gh", ["api", "-H", "Accept: application/vnd.github.raw", `repos/${repo}/contents/${file}?ref=${commit}`], { encoding: "utf8", maxBuffer: 16 * 1024 * 1024 });
      mkdirSync(dirname(cached), { recursive: true });
      writeFileSync(cached, text);
    }
    return { file, text: readFileSync(cached, "utf8") };
  });
  const all = texts.map((entry) => entry.text).join("\n");
  if (COPYLEFT_TEXT.test(all)) throw new Error(`${repo} at ${commit}: its licence files name a copyleft licence`);
  for (const term of licence.spdx.split(/\s+(?:OR|AND)\s+/u)) {
    const markers = LICENCE_MARKERS[term];
    if (markers === undefined || !markers.every((marker) => marker.test(all))) throw new Error(`${repo} at ${commit}: ${licence.files.join(", ")} do not read as ${term}`);
  }
  return { spdx: licence.spdx, texts };
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
 * What the archive's LICENSES.md holds, for SAMPLES.md: licence texts for every repository, for none (ESLint test-set
 * sets: the licence ids the test set recorded, which publishedSamples holds to RECORDED_LICENCES), or for some.
 */
export function licensesClaim(repos: number, withText: readonly string[]): string {
  if (withText.length === repos) return "The archive's `LICENSES.md` lists every file with its repository's licence text.";
  const ids = "the licence id the ESLint test set recorded, which is the one `scripts/samples/licences.ts` records for the repository";
  if (withText.length === 0) return `The archive's \`LICENSES.md\` lists every file with its repository's licence: ${ids}. It holds no licence texts.`;
  return `The archive's \`LICENSES.md\` lists every file with its repository's licence. It holds the licence text of ${withText.join(", ")} only; for every other repository it holds ${ids}, and no licence text.`;
}

/**
 * The archive's LICENSES.md: per source repository its licence, the files taken from it (as the archive holds them),
 * and the licence texts read at their commits. ESLint test-set sources carry the licence the test set recorded.
 */
export function licensesMarkdown(samples: readonly Sample[], texts: (repo: string, commit: string) => LicenceText[] | undefined): string {
  const byRepo = new Map<string, Sample[]>();
  for (const sample of samples) byRepo.set(sample.source.repo, [...(byRepo.get(sample.source.repo) ?? []), sample]);
  const allText = reposWithText(samples, texts).length === byRepo.size;
  const lines = [
    "# Sources",
    "",
    `Every sample in this archive is a copy of one file of an open-source repository, at the commit its \`.expected.json\` names (\`source.commit\`; for a SWE task's file, that commit with the task's reference solution applied). Each repository below is listed with its licence, the files taken from it, and ${allText ? "its licence text as read at those commits" : "its licence text as read at those commits where one was read (otherwise the licence the ESLint test set recorded)"}. The files remain under their repositories' licences.`,
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
    if (!anyText) lines.push("The licence is the one the ESLint test set recorded for each file (`source.license`).", "");
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
