---
"@neralabs/rerail": patch
---

Code rules: a file-level gate written through a utility now lets the anchor index skip a file. A utility whose rule is `inside` the grammar's root node to the end (Python's `module`, TypeScript's `program`), such as an import gate `inside module has dotted_name ^airflow`, keeps its relations through `matches`, as do the utilities it reaches; the gate is evaluated once against the file's one root node, so a file it rejects selects no root. Any other relation utility is still reduced to its root kinds. The Python pack's AIR302, AIR311 and AIR312 documents had to write the gate inline in every clause to get this.
