---
"@neralabs/rerail-pack-next": patch
---

Next probes: the client-hook probe reports a hook only in a module of the app's server module graph (`file.package.serverGraph`), so a hook module that only a `"use client"` module imports, or that nothing imports, is no longer reported, and one a server page reaches through a `tsconfig.json` alias is.
