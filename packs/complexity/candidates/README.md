# Rerail rule pack: Cyclomatic complexity, candidates

The port in `rules/` here is still being evaluated: scored against lizard's expected findings and reported, never
gated and never shipped. This file is not published; the pack's [README](../README.md) covers the rules it ships.

## Rule details

What each rule matches beyond its one-line description: exact matching, edge cases, upstream settings and known divergences.

### `complexity-cyclomatic-java`

The pack's rule for Java (`.java`), with the shipped rules' limit, message and counting: a method fails above 6, the
finding spans the whole method and names it (a constructor takes its class's name), and a `switch` counts once however
many cases it has. It reads syntax only, so it needs none of the capabilities Java does not have yet.

**Waits on** its corpus (`upstream/lizard/fixtures/java/`) and lizard's scores on it being published as pack test
data, then on the conformance check's verdict on that data and a reviewer's.

**What counts.** Per method with a body, constructor and record compact constructor, in classes, interfaces (`default`
and `static` methods), enums, records, and local and anonymous classes:

- `if` (`else if` included), `for` and the enhanced `for`, `while`, `do...while`, `catch`, each `switch` statement or
  expression once however many `case`s it has (a `switch` with only `default` counts 0), `&&`, `||` and the `?:`
  conditional;
- not `else`, `finally`, `try` (with resources or not), `assert`, `synchronized`, `instanceof`, a `case`'s `when`
  keyword, or the `?` of a generic wildcard (`List<?>`, `<? extends T>`).

A method of a local or anonymous class is scored on its own. A lambda is not a function here: its conditions count
toward the method around it, as lizard counts them; a lambda in a field's initializer is in no method and is not
scored. Neither is an initializer block (`static { }`, `{ }`), which is no method, or a method without a body
(`abstract`, or an interface's).

Tree-sitter node kinds, from tree-sitter-java 0.23.5's `node-types.json` (through ast-grep-language 0.51.0):

| Function boundary | Decision |
|---|---|
| `method_declaration` with a `body`, `constructor_declaration`, `compact_constructor_declaration` | `if_statement`, `for_statement`, `enhanced_for_statement`, `while_statement`, `do_statement`, `catch_clause`, `switch_expression` (a statement or an expression) with a `case` label in its `switch_block`, `ternary_expression`, `binary_expression` whose `operator` is `&&` or `\|\|` |

A `switch` statement and a `switch` expression are both `switch_expression` nodes; one counts when its block holds a
`case` label, in colon (`case 1:`) or arrow (`case 1 ->`) form.

**Scope.** A Java directory is a package name, so the rule leaves out no directory by a bare name: `build`, `target`,
`test` and `out` are packages of real code bases (`com/example/build/`, a hexagonal `port/out/`). Test sources are
out by Maven's and Gradle's source-set layout (`**/src/test/**`, `**/src/*Test/**` such as `src/integrationTest/`,
`**/src/testFixtures/**`) and by class name (`**/*Test.java`, `**/*Tests.java`, `**/*TestCase.java`, `**/*IT.java`,
`**/*ITCase.java`); generated sources by `**/generated/**`, `**/generated-sources/**`,
`**/generated-test-sources/**` and `**/generated-src/**`, which is where Maven (`target/generated-sources/`) and
Gradle (`build/generated/`) write them.

**Deviations from lizard 1.24.0.** Each in bold has a corpus case, listed with lizard's number and Rerail's in
`test/engine/code-rule/complexity-rules.test.ts` (`DEVIATIONS`).

- **A record's compact constructor is scored**, as the same constructor written in full is. lizard's Java reader skips
  it and lists no function for it.
- **An initializer block is no function.** lizard reads a call followed by `{` inside one (`isEmpty()) {` in a
  `static { }` block) as a method of that name.
- **A wildcard bounded by a wildcard type counts 0** (`List<? extends Comparable<?>>` in a body). lizard reads its
  first `?` as a conditional; it reads a lone `<?>` and `<? extends T>` correctly.
- **A method with an annotation on a line of its own starts on that line**, where its declaration starts. lizard lists
  it on its name's line, with the same number. Where the method is over the limit the two findings are one line
  apart, which `divergences.json` declares (`java/Nesting.java`, lines 46 and 47).
- **A default-only switch counts 0 around a cased one**, as beside one in the TypeScript and JavaScript corpus
  (the pack README's "Deviations").
- lizard's Java reader lists no function, or only some, in some files (none in a class declared under `@Entity` and
  `@Table(name = "owners")`), and reads some annotations with arguments, annotation type elements
  (`String[] names() default {};`) and enum constants with a body as functions. The corpus has none of these.

**Checked on real code.** Compared with lizard 1.24.0 function by function (2026-11-12, at the commits named), pairing
each lizard function with the Rerail function that starts within two lines of it:

| Repository | Paired | Same number | The rest |
|---|--:|--:|---|
| gson `65df6e5` `gson/src/main/java` | 704 | 704 | |
| commons-lang `5d2715b` `src/main/java` | 4,031 | 4,031 | |
| spring-petclinic `500158f` `src/main/java` | 43 | 43 | |
| jackson-databind `0736200` `src/main/java` | 8,347 | 8,343 | 4 methods with a wildcard bounded by a wildcard type in the body |
| kafka `151dea7` `clients/src/main/java` | 11,588 | 11,588 | |

Every one of the 2,315 files parses without a syntax error. Outside the pairs, lizard lists 21 functions: annotations
with arguments, annotation type elements and enum constants with a body, which are not methods, and methods under
three or more annotation lines, which the rule starts more than two lines earlier. The rule scores 1,373 methods
outside the pairs: those under several annotation lines, and methods lizard does not list (spring-petclinic's annotated
entity and controller classes among them, where lizard lists 44 functions and the rule 88). Those were sampled, not
inspected one by one.

**Corpus.** `upstream/lizard/fixtures/java/` (written by Rerail, `upstream/lizard/UPSTREAM.md`): every counted
decision, a method at exactly 6 and ones over it, switch statements and expressions in colon and arrow form, records,
lambdas, local and anonymous classes, enum constants with a body, text that looks like a decision and is not one
(wildcards, strings, text blocks, comments, annotations), each deviation above in bold, and two test sources lizard
scores over the limit, one out of scope by its path and one by its name.
`test/engine/code-rule/complexity-rules.test.ts` holds the rule to the same parity as the shipped rules, loading it
from here.
