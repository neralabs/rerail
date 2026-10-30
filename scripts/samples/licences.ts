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
 * Every source repository a sample is taken from whose licence files the builders read, with the licence they state at
 * the pinned commits (checked against the text on every build, `licenceTexts`): permissive, or, for a commit whose
 * file no allowlisted licence covers (`NONE`: nothing states one), the licence that keeps it out.
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
      "4edd44f6174f892a20ebb808b63b26c362a97763": { spdx: "MIT", files: ["README.md"] },
    },
  },
  "axios/axios": { spdx: "MIT", files: ["LICENSE"] },
  "babel/babel": { spdx: "MIT", files: ["LICENSE"] },
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
  "gatsbyjs/gatsby": { spdx: "MIT", files: ["LICENSE"] },
  "getsentry/sentry-javascript": { spdx: "MIT", files: ["packages/browser/LICENSE"] },
  "hexojs/hexo": { spdx: "MIT", files: ["LICENSE"] },
  "iobroker/iobroker.js-controller": { spdx: "MIT", files: ["packages/common/LICENSE"] },
  "jestjs/jest": {
    spdx: "MIT",
    files: ["LICENSE"],
    at: {
      "12c13459b35c812db7eee6663bf521032316c127": { spdx: "BSD-3-Clause", files: ["LICENSE", "PATENTS"] },
      "242fb35459b387e360f52394b0e30f42101cc51d": { spdx: "BSD-3-Clause", files: ["LICENSE", "PATENTS"] },
      "4f2ce5f3be0a273929f3ef1b8fe05b668285d6c4": { spdx: "BSD-3-Clause", files: ["LICENSE", "PATENTS"] },
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
  "tryghost/ghost": { spdx: "MIT", files: ["LICENSE"] },
  "typeorm/typeorm": { spdx: "MIT", files: ["LICENSE"] },
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
};

/**
 * Source repositories whose samples come from the ESLint test set or the React test set
 * (scripts/samples/convert-eslint-test-set.ts), with the licence the test set recorded for them (the React test set's
 * each checked against GitHub's detected licence when added), as the committed `SAMPLES.md` tables list it. Their
 * licence texts are not read on build; a sample's `source.license` must still equal the entry.
 */
const TEST_SET_LICENCES: Readonly<Record<string, string>> = {
  "adobe/react-spectrum": "Apache-2.0",
  "aidenybai/million": "MIT",
  "akveo/react-native-ui-kitten": "MIT",
  "angular/angular-cli": "MIT",
  "ant-design/ant-design": "MIT",
  "ant-design/ant-design-mobile": "MIT",
  "apache/superset": "Apache-2.0",
  "appsmithorg/appsmith": "Apache-2.0",
  "apptension/saas-boilerplate": "MIT",
  "boardgameio/boardgame.io": "MIT",
  "casesandberg/react-color": "MIT",
  "ccch1mneyyy/dsh-tui": "MIT",
  "chakra-ui/chakra-ui": "MIT",
  "cheeriojs/cheerio": "MIT",
  "codastic/react-positioning-portal": "MIT",
  "codeceptjs/codeceptjs": "MIT",
  "colinhacks/zod": "MIT",
  "darkreader/darkreader": "MIT",
  "diegomura/react-pdf": "MIT",
  "egovernments/digit-frontend": "MIT",
  "enricoros/big-agi": "MIT",
  "facebook/docusaurus": "MIT",
  "globocom/megadraft": "MIT",
  "gpujs/gpu.js": "MIT",
  "hackjutsu/lepton": "MIT",
  "ianstormtaylor/slate": "MIT",
  "itsriffchan/solar-gwa-calculator": "MIT",
  "jameskerr/react-arborist": "MIT",
  "jedwatson/react-select": "MIT",
  "jitsi/jitsi-meet": "Apache-2.0",
  "katex/katex": "MIT",
  "khan/perseus": "MIT",
  "legendapp/legend-list": "MIT",
  "mantinedev/mantine": "MIT",
  "marmelab/react-admin": "MIT",
  "mdx-editor/editor": "MIT",
  "mermaid-js/mermaid": "MIT",
  "mobxjs/mobx": "MIT",
  "mui/material-ui": "MIT",
  "nasa-impact/mmgis": "Apache-2.0",
  "notionnext-org/notionnext": "MIT",
  "openrin/rin": "MIT",
  "openstackweb/osf-website": "MIT",
  "opentiny/tiny-charts": "MIT",
  "palantir/blueprint": "Apache-2.0",
  "pascalorg/editor": "MIT",
  "pellenybe/crypto-mcp-server---by-corax-colab": "MIT",
  "polarsource/polar": "Apache-2.0",
  "postcss/postcss": "MIT",
  "preactjs/signals": "MIT",
  "pubkey/rxdb": "Apache-2.0",
  "radix-ui/primitives": "MIT",
  "react-bootstrap/react-bootstrap": "MIT",
  "react-dates/react-dates": "MIT",
  "react-dnd/react-dnd": "MIT",
  "reactjs/react-modal": "MIT",
  "redis/ioredis": "MIT",
  "redwoodjs/graphql": "MIT",
  "reececorkhill/plan-it": "MIT",
  "remix-run/react-router": "MIT",
  "rwm6857/gracechords": "Apache-2.0",
  "scimmyjs/scimmy": "MIT",
  "streamlit/streamlit": "Apache-2.0",
  "styled-components/styled-components": "MIT",
  "supabase/supabase": "Apache-2.0",
  "tadanobutubutu/screeps": "MIT",
  "tinacms/tinacms": "Apache-2.0",
  "usebruno/bruno": "MIT",
  "walkover-web-solution/giddh-new-angular4-app": "MIT",
  "web-scrobbler/web-scrobbler": "MIT",
  "wekan/wekan": "MIT",
  "widergy/react-native-credit-card-input-tmp": "MIT",
  "xyflow/xyflow": "MIT",
  "yikart/aitoearn": "MIT",
};

/**
 * Every source repository a sample may be taken from (`owner/name`, lowercase) and its SPDX licence: SOURCE_LICENCES
 * (the repository's, before any commit of its own in `at`) and the test sets' sources. The approver summary lists
 * repositories by it; publish judges each sample by recordedLicence at its commit.
 */
export const RECORDED_LICENCES: Readonly<Record<string, string>> = {
  ...Object.fromEntries(Object.entries(SOURCE_LICENCES).map(([repo, licence]) => [repo, licence.spdx])),
  ...TEST_SET_LICENCES,
};

/**
 * The recorded SPDX licence of `repo` (any case) at `commit` (a full sha; the repository's own licence when omitted or
 * when the commit has no entry of its own), or undefined when none is recorded. Sample publish (`publishedSamples`)
 * refuses a repository with none and a declared `source.license` that is not this value at the sample's commit.
 */
export function recordedLicence(repo: string, commit?: string): string | undefined {
  const source = SOURCE_LICENCES[repo.toLowerCase()];
  return source === undefined ? TEST_SET_LICENCES[repo.toLowerCase()] : licenceAt(source, commit).spdx;
}

/** Every licence recorded for `repo` (any case) at any of its commits, sorted; empty when none is recorded. */
export function recordedLicences(repo: string): string[] {
  const source = SOURCE_LICENCES[repo.toLowerCase()];
  if (source === undefined) return TEST_SET_LICENCES[repo.toLowerCase()] === undefined ? [] : [TEST_SET_LICENCES[repo.toLowerCase()]!];
  return [...new Set([source.spdx, ...Object.values(source.at ?? {}).map((licence) => licence.spdx)])].sort(byText);
}

/** What each SPDX id's licence text must say, so a table entry that no longer matches its file stops the build. */
const LICENCE_MARKERS: Readonly<Record<string, readonly RegExp[]>> = {
  MIT: [/Permission is hereby granted, free of charge/u],
  "BSD-2-Clause": [/Redistribution and use in source and binary forms/u],
  "BSD-3-Clause": [/Redistribution and use in source and binary forms/u, /Neither the name|endorse or promote/u],
  "Apache-2.0": [/Apache License,?\s+Version 2\.0/u],
  PostgreSQL: [/Permission to use, copy, modify, and distribute this software and its\s+documentation for any purpose, without fee/u],
  Unlicense: [/free and unencumbered software released into the public domain/u],
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
  if (source === undefined) throw new Error(`${repo}: no licence is recorded for it in scripts/samples/licences.ts; read its licence at ${commit} and add it`);
  const licence = licenceAt(source, commit);
  const texts = licence.files.map((file) => {
    const cached = join(cacheRoot, "licences", repo.toLowerCase(), commit, file);
    if (!existsSync(cached)) {
      const text = execFileSync("gh", ["api", "-H", "Accept: application/vnd.github.raw", `repos/${repo}/contents/${file}?ref=${commit}`], { encoding: "utf8", maxBuffer: 16 * 1024 * 1024 });
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
