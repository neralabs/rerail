# Contributing to Rerail

Thank you for helping. A few rules keep the project open source and its licensing clean.

## Licence of contributions

Rerail is licensed under the Apache License 2.0. By submitting a contribution you agree that it is licensed under
the same terms, as described in section 5 of the licence. There is no contributor licence agreement and no
sign-off to add.

## Third-party material

Do not paste code, data or rules from another project without recording it:

1. Add an `UPSTREAM.md` entry beside the material: source project, version or revision, files, and whether each is
   verbatim, derived or an excerpt.
2. Add the upstream licence text (to `LICENSES/` or beside the material under `licenses/<project>/`).
3. Annotate the path in `REUSE.toml`.

`reuse lint` must pass. Only permissively licensed material is accepted; nothing distributed by Rerail may carry
a copyleft licence.

## Dependencies

New dependencies must be permissively licensed if they end up in anything Rerail distributes. Copyleft is
acceptable only in development-only dependencies.

## Pull requests

Open a pull request against `main`. Changes are squash-merged after review.
