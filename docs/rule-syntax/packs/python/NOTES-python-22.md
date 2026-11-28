# python-22: Ruff PTH201–PTH205

All five rules follow the JSON original and Ruff. Ruff checks calls inside parsed string type definitions, while the prior translations enumerated only the file tree. `python/python-22.rerail` roots qualified-name lookup, argument inspection and string-literal decoding in `python/python-01.rerail`'s `typedef_node`, preserving Ruff's exclusions for strings used as `Literal` values and `Annotated` metadata.

## `ruff-pth201-path-constructor-current-directory`

The original was right. In `PTH201-nested-string-annotation.py`, Ruff reports all three original-only keys, including `a: """List['Path(".")']"""` at line 4. The translation now checks the parsed `Path(".")` call through `tdef_call`, `tdef_call_only` and `tdef_plain_str`; the ordinary file-tree clause remains unchanged.

## `ruff-pth202-os-path-getsize`

The original was right. Ruff reports `a: "List['os.path.getsize(1)']"` at line 4, along with the nested calls at lines 7 and 8, and skips the `Literal` and `Annotated` value strings at lines 5 and 6. The added parsed-tree clause uses `tdef_single_arg`, with the same qualified name and `filename` parameter as the ordinary-source clause.

## `ruff-pth203-os-path-getatime`

The original was right. Ruff reports `a: "List['os.path.getatime(1)']"` at line 4, along with the nested calls at lines 7 and 8, and skips the `Literal` and `Annotated` value strings. The added parsed-tree clause uses `tdef_single_arg`, with the same qualified name and `filename` parameter as the ordinary-source clause.

## `ruff-pth204-os-path-getmtime`

The original was right. Ruff reports `a: "List['os.path.getmtime(1)']"` at line 4, along with the nested calls at lines 7 and 8, and skips the `Literal` and `Annotated` value strings. The added parsed-tree clause uses `tdef_single_arg`, with the same qualified name and `filename` parameter as the ordinary-source clause.

## `ruff-pth205-os-path-getctime`

The original was right. Ruff reports `a: "List['os.path.getctime(1)']"` at line 4, along with the nested calls at lines 7 and 8, and skips the `Literal` and `Annotated` value strings. The added parsed-tree clause uses `tdef_single_arg`, with the same qualified name and `filename` parameter as the ordinary-source clause.

## Library and engine gaps

`python/python-22.rerail` is the shared library change. `typedef_node` supplies the correct parsed-tree domain, but the existing `pth_call`, `single_arg`, `unparen`, qualified-name and literal relations have only file-tree rows. The new `tdef_*` relations repeat those operations with every step rooted in `typedef_node`; no new engine fact or primitive is needed.
