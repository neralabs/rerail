// @ts-check
import { argv, env, exit } from "node:process"
import { execa } from "execa"

if (!env.NETLIFY_AUTH_TOKEN) {
  console.error(``)
  console.error(`============================================================`)
  console.error(``)
  console.error(`  SKIPPING: NETLIFY_AUTH_TOKEN is not set.`)
  console.error(``)
  console.error(`  This is expected for pull requests from forks.`)
  console.error(`  The adapter e2e tests deploy to Netlify and cannot`)
  console.error(`  run without credentials.`)
  console.error(``)
  console.error(`  Maintainers: to run these tests, push this branch`)
  console.error(`  to the main repository and re-run CI.`)
  console.error(``)
  console.error(`============================================================`)
  console.error(``)
  exit(0)
}

// only set NETLIFY_SITE_ID from E2E_ADAPTERS_NETLIFY_SITE_ID if it's set
if (env.E2E_ADAPTERS_NETLIFY_SITE_ID) {
  env.NETLIFY_SITE_ID = env.E2E_ADAPTERS_NETLIFY_SITE_ID
}

env.ADAPTER = "netlify"

const deployTitle = `${
  env.CIRCLE_SHA1 || "N/A commit"
} - trailingSlash:${env.TRAILING_SLASH || `always`} / pathPrefix:${
  env.PATH_PREFIX || `-`
}`

const npmScriptToRun = argv[2] || "test:netlify"

// ensure clean build
await execa(`npm`, [`run`, `clean`], {
  stdio: `inherit`,
})

const deployAlias = "gatsby-e2e-tests"

// NO_COLOR disables ANSI escape codes so the URL/deploy-id parsing below is reliable.
const deployResults = await execa(
  "npx",
  [
    "ntl",
    "deploy",
    "--alias",
    deployAlias,
    "--message",
    deployTitle,
    env.EXTRA_NTL_CLI_ARGS ?? "--cwd=.",
  ],
  {
    reject: false,
    env: {
      ...env,
      NO_COLOR: "1"
    },
  }
)

if (deployResults.stdout) {
  console.log(deployResults.stdout)
}

if (deployResults.stderr) {
  console.error(deployResults.stderr)
}

if (deployResults.exitCode !== 0) {
  exit(deployResults.exitCode)
}

// /deploys/<deploy_id>
const deployIdMatch = deployResults.stdout.match(/\/deploys\/([a-f0-9]+)/)

if (!deployIdMatch) {
  console.error(`Could not extract deploy URL or deploy ID from Netlify CLI output`)
  exit(1)
}

const deployId = deployIdMatch[1]
const deployUrl = `https://${deployId}--gatsby-adapters-e2e-default.netlify.app` + (env.PATH_PREFIX ?? ``)
env.DEPLOY_URL = deployUrl
console.log(`Deployed to ${deployUrl}`)

try {
  await execa(`npm`, [`run`, npmScriptToRun], {
    stdio: `inherit`,
  })
} finally {
  if (!env.GATSBY_TEST_SKIP_CLEANUP) {
    console.log(`Deleting project with deploy_id ${deployId}`)

    const deleteResponse = await execa("npx", [
      "ntl",
      "api",
      "deleteDeploy",
      "--data",
      `{ "deploy_id": "${deployId}" }`,
    ])

    if (deleteResponse.exitCode !== 0) {
      throw new Error(
        `Failed to delete project ${deleteResponse.stdout} ${deleteResponse.stderr} (${deleteResponse.exitCode})`
      )
    }

    console.log(`Successfully deleted project with deploy_id ${deployId}`)
    }
}
