---
"@neralabs/rerail-pack-next": minor
---

Needs Rerail 0.2.3 or newer: its App Router rules written in .rerail give dimensions, and server-action-without-auth resolves imports through the tsconfig's `paths`, which older Rerail does not read.

Adds rules for redirects swallowed by catch blocks and revalidation paths that name no page. Auth-check and misplaced-middleware rules ship off after unseen-app measurement. Unproven rules remain candidates, outside the published pack.
