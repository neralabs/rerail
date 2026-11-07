// Builds packs/next/candidates/probes, a runnable pack of the Next fact model's probes: probes/conformance.json,
// probes/rules/next-probe-<probe>.json, probes/labels/fixtures/** and probes/labels/fixtures.expected.json. The fixtures
// are one checkout: a small Next.js project (both routers) at the workspace root, two nested Next apps and a Vite app
// whose app/ folder is no Next router, with modules a server page reaches directly, through a tsconfig alias, or only
// through a "use client" module; every label below is chosen by hand from Next.js 16's documented semantics, located
// by a unique substring of its fixture. Run offline by a maintainer, never in CI; the output is deterministic.
//
// Usage (from the repository root): node packs/next/candidates/build-probes.mjs
// Then: node --import tsx conformance/languages/cli.ts --pack-dir packs/next/candidates/probes --fixtures
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "probes");
const SCOPE = { include: ["**/*.ts", "**/*.mts", "**/*.cts", "**/*.tsx", "**/*.js", "**/*.jsx", "**/*.mjs", "**/*.cjs"], exclude: [] };

// ---- fixtures: a checkout with Next.js projects ---------------------------------------------------------------------
const FILES = {
  "package.json": `{ "name": "probe", "private": true, "dependencies": { "next": "16.0.0", "react": "19.2.0", "react-dom": "19.2.0" } }
`,
  "tsconfig.json": `{ "extends": "./tsconfig-base.json" }
`,
  "tsconfig-base.json": `{
  // create-next-app's import alias, from a base configuration
  "compilerOptions": { "paths": { "@/*": ["./*"] }, /* aliases */ },
}
`,
  // zechub: a hook module that only a "use client" module imports runs on the client, as Next bundles it.
  "app/zechub/page.tsx": `import Widget from "./widget";

export default function Page() {
  return <Widget />;
}
`,
  "app/zechub/widget.tsx": `"use client";
import { useCounter } from "../adapters/react";

export default function Widget() {
  return <p>{useCounter()}</p>;
}
`,
  "app/adapters/react.ts": `import { useState } from "react";

export function useCounter() {
  const [count] = useState(0);
  return count;
}
`,
  // A hook module a server page imports through the tsconfig alias is in the server graph.
  "app/aliased/page.tsx": `import { useToggle } from "@/app/shared/use-toggle";

export default function Page() {
  return <p>{String(useToggle())}</p>;
}
`,
  "app/shared/use-toggle.ts": `import { useReducer } from "react";

export function useToggle() {
  const [on] = useReducer((value: boolean) => !value, false);
  return on;
}
`,
  // No module imports this one: Next never bundles it.
  "app/unused/use-later.ts": `import { useRef } from "react";

export function useLater() {
  return useRef(null);
}
`,
  "app/layout.tsx": `import type { Metadata, Viewport } from "next";

export const metadata: Metadata = { title: "Probe" };
export const viewport: Viewport = { themeColor: "black" };

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
`,
  "app/page.tsx": `import { useState } from "react";

export const dynamic = "force-dynamic";
export const revalidate = 60;

export default function Page() {
  const [count] = useState(0);
  return <p>{count}</p>;
}
`,
  "app/dashboard/page.tsx": `"use client";
import { useEffect } from "react";
import { cookies } from "next/headers";

export const metadata = { title: "Dashboard" };

export default function Dashboard() {
  useEffect(() => {}, []);
  return <main />;
}
`,
  "app/api/users/route.ts": `export const runtime = "deno";

export async function GET() {
  return Response.json([]);
}

export const POST = async (request: Request) => new Response(await request.text(), { status: 201 });

function helper() {}
export { helper };
`,
  "app/(shop)/[id]/loading.js": `export const revalidate = "60";

export default function Loading() {
  return null;
}
`,
  "app/actions.ts": `"use server";

export async function save(data: FormData) {
  console.log(data);
}

export function reset() {}

export const VERSION = 1;

export const archive = async (id: string) => id;
`,
  "app/components/form.tsx": `export default function Form() {
  async function submit(formData: FormData) {
    "use server";
    console.log(formData);
  }
  return <form action={submit} />;
}
`,
  "components/client-button.tsx": `// a comment before the directive prologue
"use strict";
"use client";

export function Button() {
  return <button type="button" />;
}
`,
  "lib/late-directive.ts": `const value = 1;
"use client";
export default value;
`,
  "middleware.ts": `import { NextResponse } from "next/server";

export function middleware() {
  return NextResponse.next();
}
`,
  "pages/_app.tsx": `import type { AppProps } from "next/app";

export default function App({ Component, pageProps }: AppProps) {
  return <Component {...pageProps} />;
}
`,
  "pages/_document.tsx": `import { Html, Head, Main, NextScript } from "next/document";

export default function Document() {
  return (
    <Html>
      <Head />
      <body>
        <Main />
        <NextScript />
      </body>
    </Html>
  );
}
`,
  "pages/api/hello.ts": `import type { NextApiRequest, NextApiResponse } from "next";

export default function handler(_request: NextApiRequest, response: NextApiResponse) {
  response.status(200).json({ name: "probe" });
}
`,
  "pages/index.tsx": `export default function Home() {
  return <h1>Home</h1>;
}
`,
  // A Next app in a subfolder: its router paths are relative to its own package.json.
  "apps/web/package.json": `{ "name": "web", "private": true, "dependencies": { "next": "16.0.0", "react": "19.2.0", "react-dom": "19.2.0" } }
`,
  "apps/web/app/page.tsx": `import { useState } from "react";

export default function WebPage() {
  const [open] = useState(false);
  return <p>{String(open)}</p>;
}
`,
  "apps/docs/package.json": `{ "name": "docs", "private": true, "dependencies": { "next": "16.0.0", "react": "19.2.0", "react-dom": "19.2.0" } }
`,
  "apps/docs/src/app/page.tsx": `import { useState } from "react";

export default function DocsPage() {
  const [query] = useState("");
  return <input defaultValue={query} />;
}
`,
  // A Vite app beside the Next ones: its app/ folder is no app router, whatever the root package.json lists.
  "tools/vite-app/package.json": `{ "name": "vite-app", "private": true, "dependencies": { "react": "19.2.0", "react-dom": "19.2.0" }, "devDependencies": { "vite": "7.1.0" } }
`,
  "tools/vite-app/app/App.tsx": `import { useState } from "react";

export function App() {
  const [count, setCount] = useState(0);
  return <button type="button" onClick={() => setCount(count + 1)}>{count}</button>;
}
`,
};

// ---- probes ---------------------------------------------------------------------------------------------------------
const USE_CLIENT_STATEMENT = { kind: "expression_statement", has: { kind: "string", value: { eq: "use client" } } };
/** A directive-prologue statement: every earlier sibling is a string expression statement or a comment. */
const IN_PROLOGUE = { inside: { kind: "program" }, not: { follows: { stopBy: "end", not: { any: [{ kind: "expression_statement", has: { kind: "string" } }, { kind: "comment" }] } } } };
const directiveFile = text => ({ inside: { kind: "program", stopBy: "end", has: { kind: "expression_statement", has: { kind: "string", value: { eq: text } }, ...IN_PROLOGUE } } });
/** A file of a package that depends on next, whose path relative to that package's folder matches `regex`. */
const inNextPackage = regex => ({ package: { dependsOn: ["next"], path: { regex } } });
const APP_ROUTE_FILE = "^(?:src/)?app/(?:.*/)?(?:page|layout|template|loading|error|global-error|not-found|default|route)\\.(?:js|jsx|ts|tsx)$";
const exportedName = (name, rules = {}) => ({
  kind: "export_statement",
  all: [
    {
      any: [
        { has: { any: [{ kind: "function_declaration" }, { kind: "generator_function_declaration" }], has: { field: "name", regex: name } } },
        { has: { any: [{ kind: "lexical_declaration" }, { kind: "variable_declaration" }], has: { kind: "variable_declarator", has: { field: "name", kind: "identifier", regex: name } } } },
      ],
    },
    ...(rules.all ?? []),
  ],
});

const PROBES = [
  {
    name: "app-route-file",
    description: "File role: an app-router special file (page, layout, template, loading, error, global-error, not-found, default, route) under app/ or src/app/ of a package that depends on next.",
    ast: { file: inNextPackage(APP_ROUTE_FILE), rule: { kind: "program" } },
    anchor: "file.start",
    labels: [["app/layout.tsx"], ["app/page.tsx"], ["app/dashboard/page.tsx"], ["app/api/users/route.ts"], ["app/(shop)/[id]/loading.js"], ["app/zechub/page.tsx"], ["app/aliased/page.tsx"], ["apps/web/app/page.tsx"], ["apps/docs/src/app/page.tsx"]],
  },
  {
    name: "pages-special-file",
    description: "File role: a pages-router special file (_app, _document, _error, 404, 500) or an API route under pages/api/, in a package that depends on next.",
    ast: { file: inNextPackage("^(?:src/)?pages/(?:(?:_app|_document|_error|404|500)\\.(?:js|jsx|ts|tsx)$|api/)"), rule: { kind: "program" } },
    anchor: "file.start",
    labels: [["pages/_app.tsx"], ["pages/_document.tsx"], ["pages/api/hello.ts"]],
  },
  {
    name: "middleware-file",
    description: "File role: the middleware (Next 16 also reads proxy.ts) beside app/ or pages/ of a package that depends on next.",
    ast: { file: inNextPackage("^(?:src/)?(?:middleware|proxy)\\.(?:js|ts)$"), rule: { kind: "program" } },
    anchor: "file.start",
    labels: [["middleware.ts"]],
  },
  {
    name: "use-client-directive",
    description: "Directive: a \"use client\" statement in the file's directive prologue (a later one is no directive).",
    ast: { rule: { all: [USE_CLIENT_STATEMENT, IN_PROLOGUE] } },
    labels: [["app/dashboard/page.tsx", '"use client"'], ["app/zechub/widget.tsx", '"use client"'], ["components/client-button.tsx", '"use client"']],
  },
  {
    name: "inline-server-action",
    description: "Directive: a \"use server\" directive opening a function body (an inline Server Action).",
    ast: {
      rule: {
        kind: "expression_statement",
        all: [
          { has: { kind: "string", value: { eq: "use server" } } },
          { inside: { kind: "statement_block", inside: { any: [{ kind: "function_declaration" }, { kind: "function_expression" }, { kind: "arrow_function" }, { kind: "method_definition" }] } } },
          // `follows` walks unnamed siblings too: the block's `{` precedes its first statement.
          { not: { follows: { stopBy: "end", not: { any: [{ kind: "expression_statement", has: { kind: "string" } }, { kind: "comment" }, { regex: "^\\{$" }] } } } },
        ],
      },
    },
    labels: [["app/components/form.tsx", '"use server"']],
  },
  {
    name: "route-handler",
    description: "Route handler: an exported HTTP-method function or const in an app-router route file.",
    ast: {
      file: inNextPackage("^(?:src/)?app/(?:.*/)?route\\.(?:js|ts)$"),
      rule: {
        kind: "identifier",
        regex: "^(?:GET|HEAD|POST|PUT|DELETE|PATCH|OPTIONS)$",
        any: [
          { inside: { kind: "function_declaration", inside: { kind: "export_statement" } } },
          { inside: { kind: "variable_declarator", inside: { kind: "lexical_declaration", inside: { kind: "export_statement" } } } },
        ],
      },
    },
    labels: [["app/api/users/route.ts", "GET"], ["app/api/users/route.ts", "POST"]],
  },
  {
    name: "metadata-in-client-component",
    description: "Metadata export + directive: metadata, generateMetadata, viewport or generateViewport exported from a \"use client\" file (Next refuses it).",
    ast: { rule: { all: [exportedName("^(?:metadata|generateMetadata|viewport|generateViewport)$"), { inside: { kind: "program" } }, directiveFile("use client")] } },
    labels: [["app/dashboard/page.tsx", "export const metadata"]],
  },
  {
    name: "invalid-segment-config",
    description: "Route segment config: dynamic not one of auto|force-dynamic|error|force-static, runtime not nodejs|edge, or revalidate not a number or false, in an app-router route file.",
    ast: {
      file: inNextPackage(APP_ROUTE_FILE),
      rule: {
        inside: { kind: "variable_declarator", inside: { kind: "lexical_declaration", inside: { kind: "export_statement", inside: { kind: "program" } } } },
        any: [
          { inside: { kind: "variable_declarator", has: { field: "name", regex: "^dynamic$" } }, value: { type: "string" }, not: { value: { regex: "^(?:auto|force-dynamic|error|force-static)$" } } },
          { inside: { kind: "variable_declarator", has: { field: "name", regex: "^runtime$" } }, value: { type: "string" }, not: { value: { regex: "^(?:nodejs|edge)$" } } },
          { inside: { kind: "variable_declarator", has: { field: "name", regex: "^revalidate$" } }, value: { constant: true }, not: { any: [{ value: { type: "number" } }, { value: { eq: false } }] } },
        ],
      },
    },
    labels: [["app/api/users/route.ts", '"deno"'], ["app/(shop)/[id]/loading.js", '"60"']],
  },
  {
    name: "server-import-in-client",
    description: "next/* import + directive: a \"use client\" file importing server-only, next/headers or next/server.",
    ast: { rule: { all: [{ kind: "import_statement", has: { field: "source", value: { regex: "^(?:server-only|next/headers|next/server)$" } } }, directiveFile("use client")] } },
    labels: [["app/dashboard/page.tsx", "import { cookies }"]],
  },
  {
    name: "client-hook-in-server-component",
    description: "React (inherited) + file role + module graph + directive: a call of a client-only hook imported from react, in a file under app/ or src/app/ of a package that depends on next which is in the app's server module graph (reached from a server page, layout or route by imports that pass no \"use client\" module), without a \"use client\" directive.",
    ast: {
      file: { package: { dependsOn: ["next"], path: { regex: "^(?:src/)?app/" }, serverGraph: true } },
      rule: {
        kind: "call_expression",
        all: [
          {
            has: {
              field: "function",
              kind: "identifier",
              resolves: {
                binding: {
                  site: {
                    kind: "import_specifier",
                    has: { field: "name", regex: "^(?:useState|useReducer|useEffect|useLayoutEffect|useInsertionEffect|useRef|useImperativeHandle|useSyncExternalStore|useTransition|useDeferredValue|useOptimistic|useActionState|useContext)$" },
                    inside: { kind: "import_statement", stopBy: "end", has: { field: "source", value: { eq: "react" } } },
                  },
                },
              },
            },
          },
          { not: directiveFile("use client") },
        ],
      },
    },
    labels: [["app/page.tsx", "useState(0)"], ["app/shared/use-toggle.ts", "useReducer((value"], ["apps/web/app/page.tsx", "useState(false)"], ["apps/docs/src/app/page.tsx", "useState(\"\")"]],
  },
  {
    name: "use-server-sync-export",
    description: "Directive + exports: in a \"use server\" file, an exported name that is not an async function (Next allows only async functions there).",
    ast: {
      rule: {
        kind: "identifier",
        all: [
          {
            any: [
              { inside: { kind: "function_declaration", not: { regex: "^async\\s" }, inside: { kind: "export_statement" } } },
              { inside: { kind: "variable_declarator", not: { has: { field: "value", any: [{ kind: "arrow_function" }, { kind: "function_expression" }], regex: "^async\\s" } }, inside: { kind: "lexical_declaration", inside: { kind: "export_statement" } } } },
            ],
          },
          { inside: { stopBy: "end", kind: "program", has: { kind: "expression_statement", has: { kind: "string", value: { eq: "use server" } }, ...IN_PROLOGUE } } },
        ],
      },
    },
    labels: [["app/actions.ts", "reset"], ["app/actions.ts", "VERSION"]],
  },
];

// ---- write ----------------------------------------------------------------------------------------------------------
rmSync(join(ROOT, "rules"), { recursive: true, force: true });
rmSync(join(ROOT, "labels"), { recursive: true, force: true });
const fixtures = join(ROOT, "labels", "fixtures");
for (const [path, text] of Object.entries(FILES)) {
  mkdirSync(dirname(join(fixtures, path)), { recursive: true });
  writeFileSync(join(fixtures, path), text);
}
const position = (path, needle) => {
  const text = FILES[path];
  if (needle === undefined) return { line: 1, column: 1 };
  const index = text.indexOf(needle);
  if (index < 0 || text.indexOf(needle, index + 1) >= 0) throw new Error(`${path}: ${needle} is not unique`);
  const before = text.slice(0, index).split("\n");
  return { line: before.length, column: [...before.at(-1)].length + 1 };
};
const findings = [];
mkdirSync(join(ROOT, "rules"), { recursive: true });
for (const probe of PROBES) {
  const code = `next/${probe.name}`;
  writeFileSync(
    join(ROOT, "rules", `next-probe-${probe.name}.json`),
    `${JSON.stringify(
      {
        format: "rerail.policy/v1",
        id: `next-probe-${probe.name}`,
        revision: 1,
        mode: "log",
        description: `Next pack probe. ${probe.description}`,
        scope: SCOPE,
        clauses: [{ id: "probe", forbid: { ast: { language: "typescript", ...probe.ast } }, violation: { anchor: probe.anchor ?? "witness.span", code, message: probe.description } }],
      },
      null,
      2,
    )}\n`,
  );
  for (const [path, needle] of probe.labels) findings.push({ rule: code, path, ...position(path, needle) });
}
findings.sort((a, b) => (a.rule < b.rule ? -1 : a.rule > b.rule ? 1 : 0) || (a.path < b.path ? -1 : a.path > b.path ? 1 : 0) || a.line - b.line || a.column - b.column);
writeFileSync(
  join(ROOT, "labels", "fixtures.expected.json"),
  `${JSON.stringify({ tool: "labels", version: "1", source: { kind: "fixtures", origin: "hand-labelled", rules: PROBES.map(probe => `next/${probe.name}`), files: [Object.keys(FILES).sort()] }, findings }, null, 1)}\n`,
);
const capabilities = probe => ["syntax", ...(JSON.stringify(probe.ast).includes('"resolves"') ? ["scope"] : []), ...(JSON.stringify(probe.ast).includes('"value"') ? ["value"] : [])];
writeFileSync(
  join(ROOT, "conformance.json"),
  `${JSON.stringify(
    {
      pack: "next-probes",
      family: "ecma",
      capabilities: ["syntax", "scope", "value"],
      corpora: [],
      tools: [{ name: "labels", version: "1" }],
      oracles: ".",
      rules: PROBES.map(probe => ({ id: `next-probe-${probe.name}`, upstream: { tool: "labels", rule: `next/${probe.name}` }, policy: `rules/next-probe-${probe.name}.json`, capabilities: capabilities(probe), match: "line+column" })),
    },
    null,
    2,
  )}\n`,
);
console.log(`probes: ${PROBES.length}, files: ${Object.keys(FILES).length}, labels: ${findings.length}`);
