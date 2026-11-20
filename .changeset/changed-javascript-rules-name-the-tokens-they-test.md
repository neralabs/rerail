---
"@neralabs/rerail-pack-javascript": patch
---

This version needs Rerail 0.2.0 or newer, where rules that test punctuation such as `,`, `=>` or `[` run again; 0.2.0 skips the previous version's. They report what they did, except that `eslint-no-with` now spans the whole `with` statement, not only its keyword.
