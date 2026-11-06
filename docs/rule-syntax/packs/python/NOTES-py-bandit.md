# py-bandit: 49 Bandit rules, hand-written

Every rule follows Bandit 1.9.4, the pinned tool. I installed it in a scratch venv (`pip install bandit==1.9.4`)
and ran it on every counterexample below. The rules share two libraries:

| file | lines | what |
|---|---|---|
| `lib/python/security.rr` | 234 | Bandit's view of a file: a call's qualified name, its arguments and their literals, the imports seen so far, the blacklist test (B001), catch-all handlers, the process-starting functions |
| `lib/python/security-blacklist.rr` | 120 | generated: the B3xx call lists and the B4xx import lists, 110 rows |
| `packs/python/gen-bandit-blacklist.py`, `check-bandit-blacklist.py` | | regenerate the table from the JSON; check it against the JSON (round trip) and, with `--bandit DIR`, against Bandit's `blacklists/*.py` |

`check-bandit-blacklist.py --bandit …/site-packages/bandit` reports `round trip: 20 call lists, 13 import lists,
110 rows, 0 differences` and `bandit: 33 lists; 0 differ; import order matches`.

The 29 blacklist rules are one line each: `B301(c) :- blacklisted(c, "B301").` The other 20 are 14 to 52 lines.
The total is 1,007 lines, comments and directives included.

## Scores

`rrlab score python <id> --all`. Bandit's fixtures are flat files, not one directory per rule, so a rule's
"own fixtures" set is empty. `--all` scores the rule's code on all 133 fixture files. Every rule is perfect at
(path, line) and at (path, line, column).

| rule | lines | TP / FP / FN | deviations from the JSON |
|---|---|---|---|
| `bandit-b101-assert-used` | 14 | 3 / 0 / 0 | - |
| `bandit-b102-exec-used` | 18 | 3 / 0 / 0 | D1 |
| `bandit-b103-set-bad-file-permissions` | 29 | 11 / 0 / 0 | D3 |
| `bandit-b110-try-except-pass` | 17 | 4 / 0 / 0 | - |
| `bandit-b112-try-except-continue` | 18 | 4 / 0 / 0 | - |
| `bandit-b113-request-without-timeout` | 31 | 28 / 0 / 0 | D2 |
| `bandit-b201-flask-debug-true` | 20 | 1 / 0 / 0 | D4 |
| `bandit-b301-pickle` | 17 | 15 / 0 / 0 | D1 |
| `bandit-b302-marshal` | 16 | 4 / 0 / 0 | D1 |
| `bandit-b303-md5` | 19 | 13 / 0 / 0 | D1 |
| `bandit-b304-ciphers` | 18 | 16 / 0 / 0 | D1 |
| `bandit-b305-cipher-modes` | 17 | 3 / 0 / 0 | D1 |
| `bandit-b306-mktemp-q` | 18 | 6 / 0 / 0 | D1 |
| `bandit-b307-eval` | 17 | 5 / 0 / 0 | D1 |
| `bandit-b308-mark-safe` | 17 | 45 / 0 / 0 | D1 |
| `bandit-b310-urllib-urlopen` | 17 | 10 / 0 / 0 | D1 |
| `bandit-b311-random` | 18 | 14 / 0 / 0 | D1 |
| `bandit-b312-telnetlib` | 15 | 3 / 0 / 0 | D1 |
| `bandit-b313-xml-bad-celementtree` | 15 | 6 / 0 / 0 | D1 |
| `bandit-b314-xml-bad-elementtree` | 16 | 6 / 0 / 0 | D1 |
| `bandit-b315-xml-bad-expatreader` | 15 | 3 / 0 / 0 | D1 |
| `bandit-b316-xml-bad-expatbuilder` | 15 | 4 / 0 / 0 | D1 |
| `bandit-b317-xml-bad-sax` | 16 | 9 / 0 / 0 | D1, D5 |
| `bandit-b318-xml-bad-minidom` | 15 | 4 / 0 / 0 | D1 |
| `bandit-b319-xml-bad-pulldom` | 15 | 4 / 0 / 0 | D1 |
| `bandit-b321-ftplib` | 16 | 3 / 0 / 0 | D1 |
| `bandit-b323-unverified-context` | 16 | 3 / 0 / 0 | D1 |
| `bandit-b401-import-telnetlib` | 18 | 9 / 0 / 0 | D6 |
| `bandit-b402-import-ftplib` | 17 | 10 / 0 / 0 | D6 |
| `bandit-b405-import-xml-etree` | 18 | 16 / 0 / 0 | D6 |
| `bandit-b406-import-xml-sax` | 18 | 17 / 0 / 0 | D6, D7 |
| `bandit-b407-import-xml-expat` | 18 | 9 / 0 / 0 | D6 |
| `bandit-b408-import-xml-minidom` | 18 | 10 / 0 / 0 | D6 |
| `bandit-b409-import-xml-pulldom` | 18 | 10 / 0 / 0 | D6 |
| `bandit-b411-import-xmlrpclib` | 18 | 3 / 0 / 0 | D6 |
| `bandit-b413-import-pycrypto` | 19 | 24 / 0 / 0 | D6 |
| `bandit-b415-import-pyghmi` | 18 | 3 / 0 / 0 | D6 |
| `bandit-b502-ssl-with-bad-version` | 32 | 22 / 0 / 0 | D8 |
| `bandit-b505-weak-cryptographic-key` | 48 | 16 / 0 / 0 | D9 |
| `bandit-b507-ssh-no-host-key-verification` | 26 | 10 / 0 / 0 | - |
| `bandit-b508-snmp-insecure-version` | 20 | 1 / 0 / 0 | D10 |
| `bandit-b509-snmp-weak-cryptography` | 19 | 2 / 0 / 0 | D11 |
| `bandit-b606-start-process-with-no-shell` | 18 | 22 / 0 / 0 | - |
| `bandit-b609-linux-commands-wildcard-injection` | 40 | 4 / 0 / 0 | D12 |
| `bandit-b610-django-extra-used` | 43 | 13 / 0 / 0 | D13 |
| `bandit-b612-logging-config-insecure-listen` | 17 | 1 / 0 / 0 | D14 |
| `bandit-b614-pytorch-load` | 20 | 3 / 0 / 0 | D15 |
| `bandit-b615-huggingface-unsafe-download` | 52 | 17 / 0 / 0 | - |
| `bandit-b702-use-of-mako-templates` | 17 | 3 / 0 / 0 | D16 |

## How Bandit reads a file (lib/python/security.rr)

Bandit does not resolve names. Its tests read one view of the file (`core/context.py`, `node_visitor.py`,
`utils.get_call_name`), and the library reproduces that view:

- **Qualified name of a call.** This is the callee's dotted text. A name is mapped through the import aliases
  seen so far, so `import pickle as p` makes `p.loads` read as `pickle.loads`, and `from m import x` makes `x` read
  as `m.x`.
  - `import a.b` adds no alias, and an unimported `pickle.loads` still reads `pickle.loads`.
  - The alias table is file-wide and grows as Bandit walks the file. An import inside a function counts for every
    later line, a call above the import does not see it, and no local binding shadows an alias.
  - Relative dots are dropped (`from .m import x` maps x to `m.x`). `from . import x as y` maps y to `x`.
  - A callee that is not a name chain reads as `""`, or as `.attr` after a call (`foo().bar` reads `.bar`).
- **Arguments.** Bandit reads an argument's literal: an attribute's attr, a name's id, True/False/None as
  strings, numbers, strings. A call, an f-string or `-1` has no literal.
- **"Imported like".** An entry of the `imports` set seen so far contains the string.
- **The blacklist.**
  - A call has one name: its qualified name, or the module of `__import__("m")` / `importlib.import_module("m")`.
    It is tested against the call lists and then the import lists, by equality, as Bandit's Call blacklist holds
    both: `telnetlib()` is B401, `importlib.import_module("pickle.loads")` is B301.
  - An import matches by `startswith`, so `import telnetlib2` is B401. It gives one finding per statement, from
    the first list in Bandit's order that matches.
- **"Before".** Bandit's walk order, which is CPython's ast order: core's `earlier` and `visit_index`. A def's
  decorators come after its body, so `@p.loads(x)` over `def f(): import pickle as p` is B301.
- **Literals.** Ellipsis is a literal (`timeout=...` is a timeout). A complex number is a literal of its own type,
  which no size test compares (`1024j` is no key size). An int compares as decimal text and is never converted,
  so a 25-digit mode or key size cannot overflow.
- **Lines.** Some tests (B201, B502, B609) move a finding to the line of a keyword's value. `report_at` reports at
  the call when the two share a line, and at the value otherwise. That gives Bandit's line, and its column in the
  usual case.

## Deviations from the JSON (the rules follow Bandit)

Each counterexample was run through Bandit 1.9.4.

- **D1. Names are read as Bandit reads them, not resolved.**
  - `pickle.loads(x)` with no import is B301 in Bandit. The JSON's `resolves.import` needs the import.
  - `import pickle as p` then `def f(p): p.loads(x)` is B301: Bandit ignores the parameter.
  - The JSON's B102 and B307 already modelled the alias table for exec and eval, and they agree.
- **D2. B113.** `requests.get(u, timeout=pick())` is reported: a call has no literal, so Bandit reads it as "no
  timeout". The JSON accepts any timeout keyword.
- **D3. B103.**
  - Bandit takes exactly two positional arguments and reads the second, so `os.chmod(p, mode=0o777)` is not
    reported. The JSON matched `mode=`.
  - The name test is "contains chmod" on the last segment of the qualified name (`my_chmod_helper(p, 0o020)` is
    reported), where the JSON tested the callee's text.
- **D4. B201.**
  - Bandit does not check that the receiver is a Flask app: once flask is imported, `server.run(debug=True)` is
    reported.
  - The finding is on the line of `True`. The JSON reports the call's line.
- **D5. B317.** The JSON matched only expression statements. `p = xml.sax.make_parser()` is B317 in Bandit.
- **D6. Imports, all B4xx rules.**
  - The match is a prefix match (`import telnetlib2`, `import xmlrpclib`).
  - `from telnetlib import *` is reported.
  - `importlib.import_module("telnetlib")` and `__import__("ftplib")` are reported.
  - There is one finding per statement: `import pickle, telnetlib` is B401 only.
  - The JSON had none of these, except for listed spellings.
- **D7. B406.** Bandit reports `from xml.sax import saxutils` ("xml.sax.saxutils" starts with "xml.sax"). The
  JSON exempted saxutils, and so did its description. I rewrote the description toward Bandit: "Avoid importing
  xml.sax or anything under it, saxutils included; …".
- **D8. B502.**
  - Bandit reads the last name of a `method=` or `ssl_version=` keyword on any call, with no import:
    `anything(method=PROTOCOL_SSLv3)` is reported.
  - `ssl.wrap_socket` is judged by `ssl_version` only, and `pyOpenSSL.SSL.Context` by `method` only.
  - The line is the keyword value's.
  - The JSON required the value to resolve to an imported `ssl.*` or `OpenSSL.*` name.
- **D9. B505.**
  - The size is `kw or args[i] or default`, by truthiness: `rsa.generate_private_key(e, 512, key_size=0)` is
    reported.
  - Floats count (`dsa.generate_private_key(512.0)`).
  - An EC curve may be a bare name (`curve=SECT163K1`).
- **D10. B508.** Bandit reads only the `mpModel=` keyword: `CommunityData("public", "x", 1)` is not reported. The
  JSON bound mpModel positionally too.
- **D11. B509.** Bandit counts positional arguments only: `UsmUserData("u", authKey="a", privKey="p")` is
  reported. The JSON counted keywords as arguments.
- **D12. B609.**
  - Bandit's lists are used, so `subprocess.run`, `check_call`, `check_output` with shell=True and
    `subprocess.getoutput` are covered: `subprocess.run("tar *", shell=True)` is reported.
  - The wildcard is `*` only: `"tar c ?"` is not reported.
  - A list argument is read through its items' reprs, nested lists, tuples, sets and dicts included:
    `os.system(["tar", ["*"]])` and `os.system(["tar", b"*"])` are reported. A dict's items show their values
    only from Python 3.14, where ast nodes got a readable repr.
  - The line is shell='s.
- **D13. B610.**
  - Any call named `extra` is checked. The JSON wanted a QuerySet-looking receiver; Bandit has no such test.
  - A select that is no dict is insecure: `anything.extra(None, ["a = b"])` is reported.
- **D14. B612.** Only the keyword counts: `logging.config.listen(9999, check)` is reported. The JSON bound
  `verify` positionally.
- **D15. B614.** Bandit requires `import torch` itself: `from torch import load; load(f)` is not reported.
- **D16. B702.** Any qualified name with a `mako` segment that ends in `Template`: `import mako; mako.Template(t)`
  is reported. The JSON wanted exactly `mako.template.Template`.

## Limits (no fixture shows them)

- **Nosec on a moved finding (B201, B502, B609).** Bandit reads `# nosec` on the reported line and on the call's
  lines. When the keyword's value is on a later line than the call, the rule reports at the value, so only the
  value's lines are read:

  ```python
  app.run(  # nosec          Bandit: suppressed (the call's lines hold the nosec)
      debug=True,            the rule: B201 here
  )
  ```

  The language has no way to take the line from one node and suppression from another. See change request 4.
- **Blacklist precedence and `-t`.** One import statement gives one finding, from the first list in Bandit's
  order: `import telnetlib, ftplib` is B401 only. That holds for a full Bandit run, which is what the recording
  is. `bandit -t B402` loads only B402's list and reports B402 there.
- **Truthiness.** In B505 and B615, a list, tuple, set or dict literal counts as truthy, even when empty.
- **Options.** Bandit's `assert_used.skips` and `check_typed_exception` are not ported; the JSON has none.
- **Same-name tests.** B602 to B605 and B607 share `process_fn` but are not in this batch.

## Harness changes (tools/rrlab)

- **`suppress nosec` in rrlab.py.** rrlab honoured only eslint and noqa comments, so every nosec fixture scored
  as an FP. I added Bandit's rules as `native/src/code_rule_dsl/suppress/nosec.rs` ports them:
  - the test list, as ids or names, with test names read from nosec.rs;
  - a blanket `# nosec`;
  - the reported line together with the first `# nosec` among the node's lines.

  The change is the `nosec_*` functions and four lines in `Run.suppress` and `Run.findings` (the finding keeps
  its `at` node).

## Harness gaps seen in review

- **Walk order.** rrlab's `visit_index` is start order, not CPython's ast order (its README says so). The
  decorator cases are right in the engine but missed in rrlab:
  - `@p.loads(x)` over `def f(): import pickle as p` is B301;
  - `@app.run(debug=True)` over `def g(): import flask` is B201.

## Library and core change requests

1. **rrlab: `split_dot(q, _, x)`** lowers to `_ = @rr_dot_init(q)`, which Soufflé rejects ("Underscore in binary
   relation"). `call_leaf` uses a regex capture instead, `match("(?:.*\\.)?(?<x>[^.]*)", q)`. The fix is to drop
   the assignment of an unused `_` column.
2. **Bitwise operators.** Soufflé's `band`/`bor`/`bxor` are not accepted by `site/check_rr.py` ("unbound band")
   or by rrlab's `lower.py`. B103 tests bits by arithmetic, `(m / 2) % 2 = 1`. I propose accepting the bitwise
   words in both, since core.rr says the language is Soufflé.
3. **`ast.rr`'s `str_const`** depends on `str_part`, which rrlab leaves empty. The library reads `value` and
   `strprefix` directly (`literal`, `string_kind`). No change is needed if the engine provides `str_part`.
4. **Report line and suppression node from different places.** Bandit reports some findings on a keyword
   value's line and reads `# nosec` over the call's lines. I propose an optional suppression column:

   ```
   .violation B201(at: node, call: node) code "B201" "…"
       suppress nosec "B201" over {call}      // the comment is read on at's line and on call's lines
   ```

   The finding's key stays (policy, code, path, line of at). Only the lines searched for the suppression comment
   change: at's line plus the lines spanned by `call` (Bandit's `linerange`). It is used by B201, B502 and B609,
   with `call` the call node, and it changes no finding key.
