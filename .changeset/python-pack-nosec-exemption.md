---
"@neralabs/rerail-pack-python": patch
---

Python pack: `conformance.json` exempts the five Bandit rules, which do not honour `# nosec` yet, from the new check that a ported rule honours its tool's suppression comments.
