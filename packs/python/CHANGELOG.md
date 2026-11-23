# @neralabs/rerail-pack-python

Every notable change to this rule pack, latest first. Each release says how many rules the pack ships, then lists its changes, rule by rule, under Added, Changed, Deprecated, Removed, Fixed and Security, as in [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/); releases from before the pack's notes took that format list them without those headings. There are two differences: a heading reads `## <version> (<date>)` rather than `## [<version>] - <date>`, and changes waiting for the next release are [changesets](../../.changeset/README.md), so there is no `## Unreleased`.

Versions are numbered as in [Semantic Versioning](https://semver.org/spec/v2.0.0.html), which promises nothing about compatibility below 1.0.0. Below 1.0.0 every release is a patch, and a patch can add, change or remove rules, so read the notes before you update.

## 0.1.11 (2026-11-20)

538 rules, 317 of them on by default.

### Added

- `bandit-b105-hardcoded-password-string` (starts in Steer): Don't assign or compare a string literal to a variable, attribute, or key named like a password, token, or secret; read credentials from the environment or a secret store.

### Changed

- This version needs Rerail 0.2.1 or newer (0.2.0 was never published), where rules that test punctuation or keywords the grammar implies run again; 0.2.1 skips the previous version's. They report what they did, except that `ruff-ret505` to `ruff-ret508` now span the whole `else` or `elif` clause, not only its keyword.

### Fixed

- `bandit-b201-flask-debug-true` now reports `.run(debug=True)` on an app from a factory, such as `app = create_app()`, as Bandit does. After an import of a module whose name contains `flask`, any `.run` call whose `debug` keyword is `True` or `"True"` is reported. Apps made by `Flask(...)` are reported as before.
- `bandit-b401-import-telnetlib` now reports telnetlib in three more kinds of import statement, as Bandit does: as one name of a multi-name import (`import os, telnetlib`), through a relative import (`from . import telnetlib`), and as any module whose name starts with telnetlib (`import telnetlib3 as telnetlib`).
- `bandit-b402-import-ftplib` now reports ftplib in three more kinds of import statement, as Bandit does: as one name of a multi-name import (`import os, ftplib`), through a relative import (`from . import ftplib`), and as any module whose name starts with ftplib.
- `bandit-b405-import-xml-etree` now reports xml.etree.ElementTree and xml.etree.cElementTree in three more kinds of import statement, as Bandit does: as one name of a multi-name import (`import os, xml.etree.ElementTree` or `from xml.etree import ElementInclude, ElementTree`), through a relative import, and as any longer dotted name that starts with either module's.
- `bandit-b406-import-xml-sax` now reports xml.sax in three more kinds of import statement, as Bandit does: as one name of a multi-name import (`import os, xml.sax`), through a relative import, and as a longer dotted name (`import xml.sax.handler`). `from xml.sax import saxutils, handler` is reported too; a from-import of only saxutils still is not.
- `bandit-b407-import-xml-expat` now reports xml.dom.expatbuilder in three more kinds of import statement, as Bandit does: as one name of a multi-name import (`import os, xml.dom.expatbuilder` or `from xml.dom import Node, expatbuilder`), through a relative import, and as any longer dotted name that starts with xml.dom.expatbuilder.
- `bandit-b408-import-xml-minidom` now reports xml.dom.minidom in three more kinds of import statement, as Bandit does: as one name of a multi-name import (`import sys, xml.dom.minidom` or `from xml.dom import Node, minidom`), through a relative import, and as any longer dotted name that starts with xml.dom.minidom.
- `bandit-b409-import-xml-pulldom` now reports xml.dom.pulldom in three more kinds of import statement, as Bandit does: as one name of a multi-name import (`import sys, xml.dom.pulldom` or `from xml.dom import Node, pulldom`), through a relative import, and as any longer dotted name that starts with xml.dom.pulldom.
- `bandit-b411-import-xmlrpclib` now reports xmlrpc in three more kinds of import statement, as Bandit does: as one name of a multi-name import (`import os, xmlrpc.client`), through a relative import (`from . import xmlrpc`), and as any module whose name starts with xmlrpc, such as Python 2's `xmlrpclib`.
- `bandit-b413-import-pycrypto` now reports PyCrypto's modules in three more kinds of import statement, as Bandit does: as one name of a multi-name import (`import os, Crypto.Cipher` or `from Crypto import Random, Hash`), through a relative import, and as a submodule or longer dotted name of one of them (`import Crypto.Cipher.AES`, `from Crypto.Util.number import bytes_to_long`).
- `bandit-b415-import-pyghmi` now reports pyghmi in three more kinds of import statement, as Bandit does: as one name of a multi-name import (`import os, pyghmi`), through a relative import (`from . import pyghmi`), and as any module whose name starts with pyghmi.
- `ruff-sim113-enumerate-for-loop` no longer reports a counter that is read after its loop outside the block holding the counter and the loop, such as in an `if` after the enclosing `if` or `for`, as Ruff does not.

## 0.1.10 (2026-11-17)

537 rules, 316 of them on by default.

### Changed

- This version needs the Rerail released with it, or a newer one: update Rerail when you update the pack (`rerail update`). An older Rerail leaves the whole pack out.
- The pack's summary line and its README now name the tool versions it matches.
- `ruff-b009-get-attr-with-constant` and `ruff-b010-set-attr-with-constant` are now held off with their reason, so set-up never turns them on to match a Ruff config that runs B009 or B010.

## 0.1.9 (2026-11-12)

537 rules, 316 of them on by default.

### Changed

- `ruff-b009-get-attr-with-constant` now starts off (was in Steer).
- `ruff-b010-set-attr-with-constant` now starts off (was in Steer).

### Fixed

- `ruff-b009-get-attr-with-constant` no longer tells agents to swap every constant-name `getattr()` for a plain attribute read, which type checkers reject when the object's type does not declare the attribute. It asks for the read only where the type declares the attribute, and to keep `getattr()` otherwise rather than add a `# type: ignore`.
- `ruff-b010-set-attr-with-constant` no longer tells agents to swap every constant-name `setattr()` for a plain assignment, which in type-checked code led them to add `# type: ignore`. It asks for the assignment only where the object's type declares the attribute, and to keep `setattr()` otherwise.
- `ruff-up026-deprecated-mock-import` now checks long files in full. In a file of a few hundred lines or more it ran out of its work budget and missed `mock.mock` uses.

## 0.1.8 (2026-11-06)

537 rules, 318 of them on by default.

### Changed

- The pack's npm page lists keywords: Rerail, its language and the tools it ports, so a search on npm finds it. Its rules are unchanged.

## 0.1.7 (2026-11-04)

537 rules, 318 of them on by default.

### Fixed

- `bandit-b101-assert-used` no longer reports test code. It skips `tests/` directories and `test_*.py`, `*_test.py` and `conftest.py` files at any depth, where pytest's `assert` is the idiom. Asserts in every other file, including a `test/` package and a Django app's `tests.py`, are still reported.

## 0.1.6 (2026-11-04)

537 rules, 318 of them on by default.

- 401 new rules (11 from Bandit and 390 from Ruff): 206 start in Steer and 195 off.
- `ruff-f822-undefined-export` now reports an undefined name in an `__all__` entry written in parentheses (`[((("a")))]`) and in a bare tuple value (`__all__ = "a", "b"`, `__all__ += "c", "d"`), as Ruff does.

## 0.1.5

### Patch Changes

- 7392aea: README and licence notes: the pack's test data now lives outside the package; no rule changes.
- fa6fe5e: Preserve comparison polarity in E711/F632 steers and add concise behavior-preserving guidance for f-string braces, zip truncation, exception ordering, ordered comparisons, deployment environment keys and async HTTP/process waits. Keep existing rule scopes and enforcement modes.
- d0dd309: README: internal ticket references removed; no rule changes.
- 211e8b5: The note on the port readiness recording in each `licenses/*/UPSTREAM.md` names the upstream licence file kept beside it (`LICENSE`) by its own name.

## 0.1.4

### Patch Changes

- e06db22: Python pack: messages whose rewrite only changed the voice are back to their original wording; in an eval with a coding agent, the original and rewritten messages steered it equally well. Messages where the original steered the agent better in that eval are back to their original wording too.
- e06db22: Python pack: every rule description now says in one plain sentence what the rule wants; the exact matching detail moved to the pack README.
- e06db22: Python pack: every violation message now says in one sentence what to change in the flagged code, without tool names or rule codes.

## 0.1.3

### Patch Changes

- d1e67d7: Python pack: the port readiness gates decide what ships. `ruff-b030-except-with-non-exception-classes` passes every gate and the pack's conformance check and ships, on; `bandit-b406-import-xml-sax`, `bandit-b610-django-extra-used` and `ruff-f722-forward-annotation-syntax-error` move to `candidates/` after failing the gates (B406 differs from Bandit on a Django file, B610 names words from Bandit's tests, F722 repeats too much structure); 136 rules ship.

## 0.1.2

### Patch Changes

- da9e12b: First release through the release workflow with npm trusted publishing. The rules are unchanged.

## 0.1.1

### Patch Changes

- a0200c4: The package names the repository and directory it is published from, `neralabs/rerail` at `packs/<id>`.
- 0179e73: Python pack: `ruff-b030-except-with-non-exception-classes` no longer reports every `except T as name` handler; an `as` binding is transparent, so like Ruff's B030 the rule reports only handler types that are not exception class expressions (such as `except 1 as e`).
- 48d8728: bandit-b604 (candidate) reads only a `shell=` keyword, as Bandit's has_shell does: a call's second positional argument is no longer taken as `shell` (pytest.mark.parametrize, getattr and re.sub stop being flagged), and `shell=<expression>` is now reported unless it is a falsy constant, `[]` or `{}`.
- b8cc317: Three Python rules (two on by default, B406 and B610; F722 is off by default) no longer steer agents to rewrite idiomatic code: `bandit-b406-import-xml-sax` skips imports of only `xml.sax.saxutils` (escaping and writing helpers, no parser), `bandit-b610-django-extra-used` reports `.extra(...)` only on a receiver that looks like a Django QuerySet or Manager, and `ruff-f722-forward-annotation-syntax-error` skips jaxtyping shape strings such as `Float[Array, "b n"]`.
- 5a9d5b6: Python pack: 138 Ruff 0.16.9 and Bandit 1.9.4 rules, each scored against the tool's recorded findings on its own tests, 25 of them off by default; 96 more ports, among them 5 rules the pack carried before, wait in candidates.
- 740cf08: One rule id style in every pack, before any pack is published: lowercase letters, digits and dashes, the same string as the rule's document file name and its port's id in `conformance.json`. JavaScript's `eslint:<rule>` and `unicorn:<rule>` ids become `eslint-<rule>` and `unicorn-<rule>`, with their documents renamed to match; TypeScript's `typescript-eslint:<rule>` become `typescript-eslint-<rule>`; Python's `python-conformance:<tool>-<rule>` become `<tool>-<rule>`; React's `react:no-array-index-key` becomes `react-no-array-index-key`; Next's probe documents are named for their ids; the complexity rules become `complexity-cyclomatic-<language>`. TypeScript's `no-explicit-any` candidate is converted from the older policy format to `rerail.policy/v1` and moves to `candidates/rules/`. The JavaScript and TypeScript packs say how many of their rules start on.
