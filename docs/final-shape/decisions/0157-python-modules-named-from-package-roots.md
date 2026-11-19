# 0157: A corpus run names Python modules from their package roots

Status: accepted (2026-11-13; bug 3 of the Django practice rules report, fixed with bug 4)

## Intro

A Python import names a module by its place in a package, not by its path in the repository. In a
`src` layout the file `src/shop/base.py` is the module `shop.base`, and other files import it as
`from shop.base import Base`. Python finds it there because `src/` is on its path.

A corpus run (`run-rr`, the engine's `corpus::run`) named every document from its path below the
scanned root instead (`syntax::module`): `src/shop/base.py` was `src.shop.base`. So the `doc` and
`pkg` facts disagreed with every import of the run's own files, and everything that joins an import
to a definition of the run failed: `subclass`, `mro_method`, `resolve_export`, the import graph.
The report found it in pretix (`src/pretix`), InvenTree (`src/backend/InvenTree`), plane
(`apps/api/plane`) and netbox (`netbox/netbox`).

Now a corpus run names a Python file from its package root. That is the highest folder reached by
climbing from the file's folder while each folder holds an `__init__.py`. It is how Ruff names
modules (`detect_package_root`), and it is the name the hosts already send
(`model::python_module_path`):

```
src/shop/__init__.py    shop            (was src.shop)
src/shop/base.py        shop.base       (was src.shop.base)
netbox/netbox/urls.py   netbox.urls     (was netbox.netbox.urls; the outer netbox/ has no __init__.py)
src/manage.py           src.manage      (in no package: its path's name, as before)
a/b/m.py                a.b.m           (a/ and a/b/ are packages up to the root: unchanged)
```

## What changes

- `corpus::package_root_modules(root, paths)` gives each Python file whose package-root name differs
  from its path's name. It reads `__init__.py` on disk below the root, one check per folder.
- Phase 1 (`sources.rr`, decision 0104) states each file under that name (`observer__source_file`), so
  `py_own` and the import fixpoint see the run's own modules as their importers name them.
- The run's documents carry it (`Document::module`, the field the hosts' modules already fill), so
  `doc(d, path, module)` and `pkg(d, p)` use it.
- The per-file memo (decision 0052) is keyed by the file's content, path and plan. The module is no
  longer a function of the path: an `__init__.py` added or removed elsewhere renames it. So the
  renamed set is folded into the memo's plan digest (`memo::with_modules`). A tree whose modules all
  match their paths keeps the plan it had.

## Departures and open points

- The memo key is coarse: any change to the renamed set misses the memo for every file of the run,
  where only the renamed files' entries changed. Adding or removing an `__init__.py` is rare, so this
  is the simple correct form. A per-file key (the module beside the path in the entry's key) is the
  refinement if it ever shows in a profile.
- A configured source root (a `pyproject.toml`'s `src`, Ruff's `src` setting) is not read. A
  namespace package (a folder without `__init__.py`, PEP 420) is not a package here, as in Ruff's
  default.
- A file whose names keep a dot (`json.d/__init__.py`) keeps its path's name.

## Prior work consulted

- ast-index-spike indexes: nothing on module naming (the spike named files by path).
- 0052 (the per-file memo, keyed by path because a document key is a function of it), 0104 (phase 1's
  stated modules).
- `model::python_module_path`: the hosts' naming (Ruff's `detect_package_root`), which this matches.
