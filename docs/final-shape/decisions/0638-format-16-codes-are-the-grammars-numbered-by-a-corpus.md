# 0638. Format 16's codes are the grammar's, numbered by a corpus's counts

Status: decided (2026-11-23), by the user: "you may ORDER the codes based on a corpus, so long as it's large (we have
a 502-repo corpus and the ast-index-spike test corpus on this machine). You may just not not limit what is
REPRESENTABLE; what is representable MUST come from the grammar."

It amends decision 0626 (format 16 alone; every language's table from its grammar). What a table holds is unchanged:
every code the grammar allows and no other, so a file of any shape is written in format 16. Only the numbering
changes.

## Why

A node's code is one byte. A table of more than 256 codes keeps the numbers past the cutoff (255) under one prefix
byte, with the low part in a side list that a read searches. In the grammar's order, `(flags, kind, field, classes)`,
the codes past the cutoff were most of the nodes. Over the spike corpus (below), they were 64.6% of Python's nodes,
59.8% of JavaScript's, 62.7% of C's and 63.5% of C++'s. So most code reads went through the side list. That was
the gap between a kind or field probe of the tape and the old per-node arrays.

Numbered by frequency, the same tables put 0.5% of Python's nodes past the cutoff, 0.31% of JavaScript's, 0.13% of
C's and 0.89% of C++'s. The format, the readers and the CLIF reads stay as they were. Nothing per node is added.

## What

- `rerail-engine code-counts <root>... --out <counts.tsv>` parses every file under the roots, as the product does,
  and counts each node's code by its tag and the shipped table's number.
- `tape16 table <tags…> --counts <counts.tsv>…` (the lab, `native/lab/tape16/src/table.rs`) reads each count as the
  shipped table's entry. It numbers the grammar's codes by count, highest first. A code no counted node has keeps
  the grammar's order, after them. Where a table is shipped, the run refuses a grammar whose set of codes differs
  from it: a changed grammar is written by a run without counts first, then numbered.
- A counts file names the table revision its numbers are of (a `revision` line per tag), and the lab refuses
  counts of another revision than the shipped table's.
- The tables' revision is 5. A tape of an older revision is refused, not misread, and is made again.
- The counts, summed:
  - 520 repositories' bare mirrors (the sampo corpus): 578,677 files, 971 million nodes. Each repository's HEAD
    tree is read from its object store through gix, with nothing checked out (`code-counts <dir of mirrors>
    --per-repo 1000`: at most 1,000 files of each language per repository, spread evenly over the tree);
  - this tree's rule files (`docs/rule-syntax`), for the rule language, JSON and Markdown.
- Not the ast-index-spike sample corpus (revisions 3 and 4). It is a few large projects per language, many of them
  compilers and runtimes with their test suites, and the user does not want the tables specialized "for just
  compiling the compiler fast". Each repository weighs about the same: no project's size outweighs the rest.
- Sampo has few Scala (86 files) and Swift (1,821) sources, so those two tables' order is the least settled. A
  larger corpus of them changes only their order.

## How each corpus fares

The nodes past the cutoff, by the spike's counts alone (revision 3) and by both (revision 4):

| language | spike: 3 | spike: 4 | sampo: 3 | sampo: 4 |
|---|---|---|---|---|
| Python | 0.50% | 0.60% | 0.89% | 0.63% |
| TypeScript | 1.24% | 2.14% | 31.75% | 0.81% |
| JavaScript | 0.84% | 1.08% | 1.24% | 0.96% |
| Rust | 1.30% | 1.57% | 1.82% | 1.40% |
| C# | 1.85% | 2.30% | 4.44% | 2.26% |
| C++ | 1.17% | 1.67% | 4.00% | 2.35% |
| PHP | 0.25% | 0.30% | 2.23% | 0.51% |
| Scala | 1.08% | 1.20% | 27.03% | 9.61% |

One corpus alone fits itself: the spike's numbering left a third of the sampo corpus's TypeScript past the cutoff.

Held out: numbered by half of sampo's repositories (every other batch, at 1,000 files per language per repository)
and read on the other half, the nodes past the cutoff are 0.57% for Python, 2.49% TypeScript, 1.23% JavaScript,
0.59% C, 2.70% C++, 2.84% C#, 1.28% Rust, 0.07% Go. Adding the spike's counts to that half changes them by at most
0.3 points either way. So the code people write predicts itself, and the spike corpus adds nothing to it: revision
5 is numbered by sampo alone.

## Measured

Revision 3 (the spike's counts), on the data layer's 334-file history, step 00 profiled, two runs each (M3):

| probe | before (ns) | after (ns) | per-node arrays (ns) |
|---|---|---|---|
| kind / membership on (c0, c1) | 198 | 181 | 143 |
| field / lookup on (p, f) | 318 | 278 | 255 |
| kind / anti-join on (c0, c1) | 158 | 145 | 408 |

The session's steps took 3.54 s before and 3.25 s after. The findings are identical. Revisions 4 and 5 measure
within the M3's noise of it, with identical findings. The nodes of this history's own files (342 Python files, in
none of the corpora) past the cutoff: 62.92% in the grammar's order, 1.22% by revision 3, 0.91% by 4, 0.80% by 5.
