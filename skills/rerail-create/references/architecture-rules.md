# Architecture rules

An architecture rule says where code belongs or which way dependencies run: "shared imports nothing above it", "hosts don't
decide what a mode allows; core does", "routes delegate; parse headers in a dependency". Score a candidate by the rework a
reviewer would ask for when it is broken, not by how often it will fire.

## Find the intent

Read these as well as the `standards[]` files:

- `ARCHITECTURE.md`, `docs/architecture*`, `docs/adr/`, `docs/decisions/` and other design docs
- package and folder READMEs, and the docstring or comment at a package's root
- `CODEOWNERS`: folders with different owners are boundaries
- each package's dependency manifest in a monorepo: what depends on what is the layering

Maintainers' review comments are the best evidence: a pull request whose design was sent back and reworked before merge gives
the wrong design, the right one, and the reason. Read them with the GitHub CLI, read-only:

```sh
gh pr list --state merged --limit 100 --json number,title,files
gh api repos/<owner>/<repo>/pulls/<n>/reviews
gh api repos/<owner>/<repo>/pulls/<n>/comments
```

- Look for redirects: "this belongs in", "move this to", "should live in", "we already have", "don't reach into".
- Skip AI review tools, which often post from maintainers' accounts. Long, generic, bulleted feedback is a tool; a short
  redirect the author acted on is a person.
- Keep it only if the merged code follows the comment.
- Never comment, review or push.

If nothing is written, ask: "Which design do you keep sending back in review?"

## Shapes a code rule can check

| Shape | Example | Match | Scope |
|---|---|---|---|
| Dependency direction | "shared imports nothing above it" | `regex` on imports of a higher layer, as in [code-rules.md](code-rules.md#an-import-boundary-regex) | the lower layer |
| Layer purity | "the agent loop has no filesystem, process or environment access" | `ast` pattern for the forbidden API | the pure layer |
| A construct in the wrong place | "routes delegate; parse headers in a dependency" | `ast` with `inside` the construct that must stay thin | files that hold it |
| A cross-cutting change in a shared seam | "add tracing with an interceptor, not by editing the shared transport" | `regex` on what such a change writes | the shared file only |
| A policy decided in the wrong layer | "hosts don't turn the mode into a tool restriction" | `regex` on the deciding expression | layers that must not decide |
| One home per thing | "one connector package per provider" | `regex` on its identifiers or imports outside its home | everything but its home |

"Core should own this", "don't duplicate behaviour" and "wrong abstraction" need judgement. Narrow them to one of these shapes
for the construct the review named, or say "I can't check this reliably" and add nothing.

## Write it

1. **Write it for the design, not one pull request.** Use the folders, base classes and helpers the repo uses everywhere, not
   the names one change happened to use.
2. **Match what the wrong design writes.** Rerail reports what an edit adds, not what was in the file before. A rule about where
   code lives must match the code's lines, not only the file's path.
3. **Cover every spelling of the wrong design.** The raw object, the helper that reads from it, a subscripted or aliased type,
   the import. Make each one a caught case.
4. **The message says where the code goes and how:** "Read this header in a FastAPI dependency and inject the resolved value,
   instead of declaring and parsing it in the route." Restating the principle ("keep routes thin") is not enough.
5. **Add the reason in a clause** when it isn't obvious: "…enforce mode limits in core, where the session's tools are built, so
   every host gets them."
6. **Exclude what may cross the boundary:** tests, examples, scripts, migrations, generated code, and the owning layer.

## Check it against history

Run this after the cases pass.

1. **For each pull request the rule came from**, count matches at its base, the first draft the maintainers sent back, and the
   merge:

   ```sh
   git worktree add --detach <tmp>/<sha> <sha>
   rerail rules test <tmp>/draft.json --repo <tmp>/<sha> --json
   git worktree remove <tmp>/<sha>
   ```

   It passes when the first draft has more matches than the base and the merge has no more than the base. Missing the first
   draft means it doesn't catch what was asked for; matching the merge means it steers against the accepted design.
2. **Check it stays quiet.** Run it at the merge commit and parent of 10 to 20 recent merged pull requests that touch its scope.
   Matches that rise in one of them are false positives: narrow the rule and add that code as a passed case.
3. **With no rejected draft** (intent from docs only), write the wrong design as caught cases in the repo's real files and style,
   and say so in the read-back.

Put the result above the read-back card: `Checked against history: caught the first draft of #123; quiet on its merge and on 15
recent merged PRs.`

## Steer or Log

- **Steer** when the history check passes and the repo pass matches only real violations.
- **Log** when it matches code the maintainers merged and kept, or can't tell an allowed case from the wrong design. Show the
  person two matches and let them choose.

## Example: a header parsed in the route

```json draft
{
  "kind": "code",
  "rule": {
    "format": "rerail.policy/v1",
    "id": "local:parse-headers-in-dependencies",
    "revision": 1,
    "mode": "steer",
    "description": "Read request headers in FastAPI dependencies and inject the value, not inside route handlers; routes only delegate.",
    "scope": { "include": ["app/routers/**/*.py"], "exclude": ["**/*_test.py", "**/test_*.py"] },
    "clauses": [
      {
        "id": "request-in-route",
        "forbid": {
          "ast": {
            "language": "python",
            "rule": {
              "kind": "typed_parameter",
              "has": { "kind": "type", "regex": "^Request\\b" },
              "inside": {
                "kind": "decorated_definition",
                "has": { "kind": "decorator", "regex": "^@router\\.(get|post|put|patch|delete|api_route)\\b" },
                "stopBy": "end"
              }
            }
          }
        },
        "violation": { "code": "REQUEST_IN_ROUTE", "anchor": "witness.span", "message": "Resolve what you need from the request (a header, state) in a FastAPI dependency and inject that value, instead of taking Request in the route." }
      },
      {
        "id": "header-in-route",
        "forbid": {
          "ast": {
            "language": "python",
            "rule": {
              "kind": "call",
              "regex": "^(Header|Cookie)\\s*\\(",
              "inside": {
                "kind": "parameters",
                "inside": {
                  "kind": "decorated_definition",
                  "has": { "kind": "decorator", "regex": "^@router\\.(get|post|put|patch|delete|api_route)\\b" },
                  "stopBy": "end"
                },
                "stopBy": "end"
              }
            }
          }
        },
        "violation": { "code": "HEADER_IN_ROUTE", "anchor": "witness.span", "message": "Read this header in a FastAPI dependency and inject the resolved value, instead of declaring and parsing it in the route." }
      }
    ]
  },
  "cases": {
    "caught": [
      { "path": "app/routers/runs.py", "content": "from fastapi import APIRouter, Request\n\nrouter = APIRouter()\n\n@router.post(\"/runs\")\ndef start_run(body: dict, request: Request):\n    source = request.headers.get(\"x-run-source\")\n    return handlers.start_run(body, source)\n" },
      { "path": "app/routers/runs.py", "content": "from fastapi import APIRouter, Request\n\nrouter = APIRouter()\n\n@router.get(\"/runs\")\ndef list_runs(request: Request[State]):\n    return handlers.list_runs(request.state.db)\n" },
      { "path": "app/routers/runs.py", "content": "from fastapi import APIRouter, Header\n\nrouter = APIRouter()\n\n@router.post(\"/runs\")\ndef start_run(body: dict, source: str = Header(\"unknown\")):\n    return handlers.start_run(body, source)\n" }
    ],
    "passed": [
      { "path": "app/routers/runs.py", "content": "from typing import Annotated\nfrom fastapi import APIRouter, Depends\nfrom app.dependencies import get_run_source\n\nrouter = APIRouter()\n\n@router.post(\"/runs\")\ndef start_run(body: dict, source: Annotated[str, Depends(get_run_source)]):\n    return handlers.start_run(body, source)\n" },
      { "path": "app/routers/runs.py", "content": "from fastapi import APIRouter, Request\n\nrouter = APIRouter()\n\ndef _client_ip(request: Request) -> str:\n    return request.client.host\n" },
      { "path": "app/dependencies.py", "content": "from fastapi import Header\n\ndef get_run_source(source: str = Header(\"unknown\")) -> str:\n    return source\n" }
    ]
  }
}
```

The near miss is the private helper that takes `Request` but isn't a route. `app/dependencies.py` is outside the scope; it's
where the steer sends the code.
