---
"@neralabs/rerail-pack-django": patch
---

Requires Rerail 0.2.3 or newer. The rules read models, managers, hooks and call targets by name, reducing cross-file work. Model and task-base names now resolve through re-exports. Cross-file diagnostic details identify the originating write or query without carrying tree nodes between files.
