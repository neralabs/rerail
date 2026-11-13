# 0175: The engine reads JSON rules, through a translator that is part of it

Status: decided (2026-11-13, the user: "I want the engine to be able to load the JSON rule files directly for
backwards compatibility, so the translator needs to become part of the product").

## Intro

Main's rules are JSON documents (`rerail.policy/v1`). The engine's rules are `.rr` files. Until now a Python
translator in the docs (`docs/rule-syntax/site/translate.py`, `main_terms.py`, `engine_dialect.py`) turned the JSON
into `.rr` files ahead of time, and the engine read those (`docs/rule-syntax/packs/python-main`). That keeps an
old pack working only after someone translates it, and the product may not depend on Python.

## Decision

1. **The engine reads a JSON rule file directly.** Where it takes rule files (`logic`, `run-rr`, `check-rr`, a
   rules directory), a `.json` file whose `format` is `rerail.policy/v1` is translated when read, and the `.rr` text
   it gives goes through the reader and lowering like any `.rr` file. There is one reader and one lowering: JSON
   is never a second rule language inside the engine (consistent with "rules are `.rr` only": the translation is
   `.rr` text).
2. **The translator is Rust, in the engine** (`native/src/code_rule_dsl/logic/json_rules/`): a port of the Python
   translator and the parts of `simplify.py`, `souffle.py` and `textglob.py` it uses. No CPython.
3. **The Python is the reference until the port matches it byte for byte** on all 537 of main's Python rules
   (`packs/python-main/source/rules`). A test holds that: the port's text for each rule equals the committed
   generated file. Then the Python retires, and `regen_main_pack.sh` calls the engine
   (`rerail-engine translate-json`).
4. **The translator's fixes are made once, in Rust**, after the port. Until then a fix is made in the Python and
   ported, and the byte-parity test says when the two differ.
5. **Overrides are not part of loading.** The engine translates the JSON it is given. A rule the translator
   cannot write yet (the 9 in `packs/python-main/overrides`) is a translator bug to fix, not a file the engine
   looks for.

## Consequences

- An old pack runs on the engine without a translation step; `packs/python-main` becomes the translator's
  golden output and the run check, not a required input.
- The translator's speed matters: it runs on every load of a JSON rule. Its output is a function of the JSON text
  alone, so a run that reads the same JSON twice may translate it twice; no cache is added for it (the rule
  files are small, and a cache would be a new store).
