# Bladestan 0.11.7

Upstream: [TomasVotruba/bladestan](https://github.com/TomasVotruba/bladestan/tree/ceb0823cd4b4f400b64a008cace0d91e786a2859) at tag `0.11.7` (commit `ceb0823cd4b4f400b64a008cace0d91e786a2859`). License: MIT; [`LICENSE`](LICENSE) is the upstream license file, copied verbatim at that tag.

The files listed below are the templates of Bladestan's rule test (`tests/Rules/BladeRuleTest.php`, its fixtures under `tests/Rules/Fixture/` render them) and remain under Bladestan's license. Every other file in this pack is Rerail's. The rule documents in `rules/` and `candidates/rules/` are Rerail's re-implementations of the validators and lints of stillat/blade-parser, Bladestan and tlint, published under the pack's licence expression, `Apache-2.0 AND MIT`, as `REUSE.toml` annotates them. The rest is under the Apache License 2.0 (the pack's `LICENSE`): the generators `upstream/common.py` and `upstream/bladestan/` (`generate.py`, `RecordTest.php`), and the expected findings they write by running Bladestan.

Paths are relative to the pack directory, `packs/blade/`.

| File | Upstream path | Copy |
|---|---|---|
| `upstream/bladestan/fixtures/skeleton/resources/namespace/Test/namespacedView.blade.php` | `tests/skeleton/resources/namespace/Test/namespacedView.blade.php` | verbatim |
| `upstream/bladestan/fixtures/skeleton/resources/views/bar.blade.php` | `tests/skeleton/resources/views/bar.blade.php` | verbatim |
| `upstream/bladestan/fixtures/skeleton/resources/views/compile-error.blade.php` | `tests/skeleton/resources/views/compile-error.blade.php` | verbatim |
| `upstream/bladestan/fixtures/skeleton/resources/views/component.blade.php` | `tests/skeleton/resources/views/component.blade.php` | verbatim |
| `upstream/bladestan/fixtures/skeleton/resources/views/components/component.blade.php` | `tests/skeleton/resources/views/components/component.blade.php` | verbatim |
| `upstream/bladestan/fixtures/skeleton/resources/views/empty.blade.php` | `tests/skeleton/resources/views/empty.blade.php` | verbatim |
| `upstream/bladestan/fixtures/skeleton/resources/views/file_with_include.blade.php` | `tests/skeleton/resources/views/file_with_include.blade.php` | verbatim |
| `upstream/bladestan/fixtures/skeleton/resources/views/file_with_recursive_include.blade.php` | `tests/skeleton/resources/views/file_with_recursive_include.blade.php` | verbatim |
| `upstream/bladestan/fixtures/skeleton/resources/views/foo.blade.php` | `tests/skeleton/resources/views/foo.blade.php` | verbatim |
| `upstream/bladestan/fixtures/skeleton/resources/views/foreach-include.blade.php` | `tests/skeleton/resources/views/foreach-include.blade.php` | verbatim |
| `upstream/bladestan/fixtures/skeleton/resources/views/include_with_parameters.blade.php` | `tests/skeleton/resources/views/include_with_parameters.blade.php` | verbatim |
| `upstream/bladestan/fixtures/skeleton/resources/views/included_view.blade.php` | `tests/skeleton/resources/views/included_view.blade.php` | verbatim |
| `upstream/bladestan/fixtures/skeleton/resources/views/missing-component.blade.php` | `tests/skeleton/resources/views/missing-component.blade.php` | verbatim |
| `upstream/bladestan/fixtures/skeleton/resources/views/missing-include.blade.php` | `tests/skeleton/resources/views/missing-include.blade.php` | verbatim |
| `upstream/bladestan/fixtures/skeleton/resources/views/nested-foreach.blade.php` | `tests/skeleton/resources/views/nested-foreach.blade.php` | verbatim |
| `upstream/bladestan/fixtures/skeleton/resources/views/once.blade.php` | `tests/skeleton/resources/views/once.blade.php` | verbatim |
| `upstream/bladestan/fixtures/skeleton/resources/views/partials/filename.blade.php` | `tests/skeleton/resources/views/partials/filename.blade.php` | verbatim |
| `upstream/bladestan/fixtures/skeleton/resources/views/partials/filename1.blade.php` | `tests/skeleton/resources/views/partials/filename1.blade.php` | verbatim |
| `upstream/bladestan/fixtures/skeleton/resources/views/partials/filename2.blade.php` | `tests/skeleton/resources/views/partials/filename2.blade.php` | verbatim |
| `upstream/bladestan/fixtures/skeleton/resources/views/partials/has_use.blade.php` | `tests/skeleton/resources/views/partials/has_use.blade.php` | verbatim |
| `upstream/bladestan/fixtures/skeleton/resources/views/php_directive_with_comment.blade.php` | `tests/skeleton/resources/views/php_directive_with_comment.blade.php` | verbatim |
| `upstream/bladestan/fixtures/skeleton/resources/views/simple_variable.blade.php` | `tests/skeleton/resources/views/simple_variable.blade.php` | verbatim |
| `upstream/bladestan/fixtures/skeleton/resources/views/static_content.blade.php` | `tests/skeleton/resources/views/static_content.blade.php` | verbatim |
| `upstream/bladestan/fixtures/skeleton/resources/views/value-from-array.blade.php` | `tests/skeleton/resources/views/value-from-array.blade.php` | verbatim |
| `upstream/bladestan/fixtures/skeleton/resources/views/view-render/render_form.blade.php` | `tests/skeleton/resources/views/view-render/render_form.blade.php` | verbatim |
| `upstream/bladestan/fixtures/skeleton/resources/views/view-render/some_int.blade.php` | `tests/skeleton/resources/views/view-render/some_int.blade.php` | verbatim |
