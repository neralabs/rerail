# Architecture rules

An architecture rule says where code belongs and which way dependencies run. Examples: "the shared package imports nothing above
it", "hosts don't decide what a mode allows; core does", "route handlers delegate; parse headers in a dependency". This page
covers where that intent is written, the shapes of it that a code rule can check, how to write the message, and how to prove
the rule against the repo's own history.

## Why they are worth it

Rerail measured architecture rules written from two repos' docs and maintainers' review comments. It replayed pull requests
whose first design the maintainers sent back, and ordinary merged pull requests in the same areas.

- **They fix the design when the agent heads for the rejected one.** Across four models, agents without rules shipped the
  rejected design in about 6 runs of 10. With the rules on, it was about 2 in 100. Code that matched the maintainers' design
  rose from about a quarter of runs to more than eight in ten.
- **They stay quiet otherwise.** On 48 ordinary merged pull requests, the rules steered once, correctly.
- **The steer beats the same guidance written into the prompt, for some models.** Given the guideline "keep route handlers
  thin" in its prompt, one model still parsed a header inside the route in 2 of 3 runs, and its contexts were short. With the
  rule on, it got one steer naming the line and where the code goes, and fixed it every time.
- **They cost little.** Cost per run moved by −3% to +5% for the strongest model, and rose by up to about a third for models
  that needed more steers.

The value is concentrated: most edits never touch a boundary. Judge a candidate by how costly the wrong design is when it
happens, not by how often the rule will fire.

## Where the intent is written

Read these in addition to the `standards[]` files from `rerail rules evidence`:

- `ARCHITECTURE.md`, `docs/architecture*`, `docs/adr/` or `docs/decisions/`, and design docs under `docs/`
- package and folder READMEs (`packages/*/README.md`, `src/*/README.md`) and the module docstring at a package's root
- `CODEOWNERS`, which shows which folders have different owners and so where the boundaries are
- the dependency manifests of each package in a monorepo: which package depends on which is the layering, written down

**Maintainers' review comments are the strongest evidence.** A pull request whose first design was sent back and reworked before
merge shows the wrong design, the right one, and the maintainer's words. Find them with the GitHub CLI, read-only:

```sh
gh pr list --state merged --limit 100 --json number,title,files
gh api repos/<owner>/<repo>/pulls/<n>/reviews
gh api repos/<owner>/<repo>/pulls/<n>/comments
```

- **Look for redirects:** "this belongs in core", "move this to", "should live in", "we already have", "don't reach into",
  "every host would need to". A comment on style or naming is not architecture.
- **Filter AI review tools by what they say, not who posted them.** About a third of review text in public repos comes from AI
  review tools, often posted from a maintainer's account. Long, generic, bulleted feedback is a tool; a terse redirect that the
  author then acted on is a person.
- **Check the author acted on it.** The merged code must follow the comment, or it is not the repo's intent.
- **Never comment, review or push.** This is read-only.

**Ask the person** when nothing is written: "Which design do you keep sending back in review?" Most candidates that fail do so
because the intent exists only in someone's head. A maintainer's review comment counts as written.

## Shapes a code rule can check

| Shape | The intent, in the repo's words | How to match | Scope |
|---|---|---|---|
| Dependency direction | "shared sits at the bottom: it imports nothing above it" | `regex` on import lines naming a higher layer, as in [code-rules.md](code-rules.md#an-import-boundary-regex) | the lower layer |
| Layer purity | "the agent loop is stateless: no filesystem, process or environment" | `ast` pattern for the forbidden API (`fs.$F($$$A)`, `process.env`) | the pure layer |
| A construct in the wrong place | "routes delegate; parse headers in a dependency" | `ast` with `inside` the construct that must stay thin (a decorated route, a component) | the files that hold it |
| A cross-cutting change in a shared seam | "add headers or tracing with an interceptor, not by changing the shared transport" | `regex` on what such a change writes into the shared file | the shared file only |
| A policy decided in the wrong layer | "hosts don't turn the mode into a tool restriction; core enforces it" | `regex` on the expression that makes the decision (`blocked = mode === "plan"`) | the layers that must not decide |
| One home per thing | "one connector package per provider, beside its definition" | `regex` on that thing's identifiers or imports outside its home | everything but its home |

**What a code rule cannot check:** "core should own this" in general, "don't duplicate behaviour", "this is the wrong
abstraction". These need judgement, and code rules cannot call a model. Narrow them to one of the shapes above, for the
construct the review comment named, or say "I can't check this reliably" and add nothing.

## Writing the rule

1. **Write the rule for the design, not for one pull request.** Use the folders, base classes and helpers the repo uses
   everywhere, not the names one pull request happened to use. A rule written from one pull request's identifiers missed the
   same mistake in the next one.
2. **Match what the wrong design writes.** Rerail reports what an edit adds; a match already in the file before the edit is not
   reported. A rule about where a file lives must therefore match something the file's lines hold, not only its path.
3. **Cover every way to write the wrong design.** Agents route around a narrow matcher. A rule that caught a parameter typed
   `Request` missed one typed `Request[State]`. List the spellings: the raw object, the helper that reads from it, the import of
   it, and add each one as a caught case.
4. **The message names where the code goes and how.** Name the construct to use, where it goes, and the helper or seam that
   already exists: "Read this header in a FastAPI dependency and inject the resolved value, instead of declaring and parsing it
   in the route." A principle ("keep routes thin") is what the prompt already said; the line and the destination are what the
   steer adds.
5. **Give the reason in a clause** when it isn't obvious: "…enforce mode limits in core, where the session's tools are built,
   so hosts that bring their own tools get them too." The agent fixes a boundary more readily when it knows what crossing it
   breaks.
6. **Exclude what may cross the boundary:** tests, examples, scripts, migrations, generated code, and the layer that owns the
   thing.

## Proving it against history

Architecture rules have a test that other rules don't: the repo's history holds both the wrong design and the right one. Run it
after the cases pass, as part of the gates in [rule-quality.md](rule-quality.md#gates).

1. **For each pull request the rule came from, count matches at three commits:** the base, the first draft the maintainers
   sent back, and the merge. Check each commit out into a temporary worktree outside the repository and run the draft there:

   ```sh
   git worktree add --detach <tmp>/<sha> <sha>
   rerail rules test <tmp>/draft.json --repo <tmp>/<sha> --json
   git worktree remove <tmp>/<sha>
   ```

   The rule fits when the first draft has more matches than the base, and the merge has no more than the base. A rule that
   misses the first draft does not catch what the maintainer asked for. A rule that matches the merge would steer against the
   design the maintainer accepted.
2. **Check it stays quiet on ordinary work.** Take 10 to 20 recent merged pull requests that touch the rule's scope, and run
   the draft at each one's merge commit and its parent. A rule whose matches rise in a pull request the maintainers merged
   without comment is wrong for that code: narrow it and add that code as a passed case.
3. **With no rejected first draft** (the intent is only in the docs), write the wrong design as caught cases in the repo's real
   files and code style, and say so in the read-back.

Put the result above the read-back card, in one line: `Checked against history: caught the first draft of #8335; quiet on its
merge and on 15 recent merged PRs.`

## Steer or Log

- **Steer** when the history check fits and the repo pass matches only places that are real violations.
- **Log** when the repo pass matches code the maintainers merged and kept, or when the matcher cannot tell an allowed case from
  the wrong design. Show the person two of those matches and let them choose.

## Example: a header parsed in the route

The repo's routes delegate to handler modules, and maintainers asked for request parsing to move into FastAPI dependencies.

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

The second caught case is the spelling an agent used to get past a first draft of this rule, which matched `^Request$` only.
The near miss is the private helper in the router file that takes `Request` but is not a route. The dependency in
`app/dependencies.py` passes because it is outside the scope, which is where the steer sends the code.
