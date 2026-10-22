"""Records Ruff 0.16.9 for the Python pack's ported rules (rerail/packs/python/conformance.json).
Run offline, never in CI, under CPython 3.14:

    python3.14 generate.py fixtures <Ruff checkout at tag 0.16.9> <ruff executable, 0.16.9>
    python3.14 generate.py corpus <ruff executable, 0.16.9> <name>=<corpus checkout> [...]

`fixtures` copies each rule's own test fixtures (the files its tests lint with default settings, under
`crates/ruff_linter/resources/test/fixtures/<linter>/`; see RULES) to fixtures/<linter>/<file> (the pack's
licenses/ruff/UPSTREAM.md lists them), writes Rerail's own cases (NOQA, PROBES) to fixtures/rerail/ (listed in
upstream/ruff/UPSTREAM.md), and runs Ruff with every ported rule selected over every file. `corpus` runs the
same rules over a corpus's pack paths, files CPython 3.14 does not parse left out. Both run Ruff isolated from any
configuration, at target version py310 (Ruff's CLI default and the engine's default `pythonVersion`, which the ports
keep), with `# noqa` honoured, as Ruff runs by default. Each finding is the rule's code, path, line and column
(Ruff's, 1-based, in characters)."""

from __future__ import annotations

import json
import pathlib
import re
import shutil
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import common  # noqa: E402

VERSION = "0.16.9"
ORIGIN = "https://github.com/astral-sh/ruff@0be08a206f9c3180afd3e93bcc792ed5cb1f4db1"
FIXTURES = "crates/ruff_linter/resources/test/fixtures"

# Each ported rule, shipped (rules/) or candidate (candidates/rules/), and its own default-settings fixtures: every
# .py and .pyi file the tests in `crates/ruff_linter/src/rules/<linter>/mod.rs` (and `src/linter.rs`'s syntax-error
# tests) lint with settings that enable the rule and change no option, preview mode or target version. Notebooks
# (.ipynb), which the host does not route to Python, are left out. Fixtures under the upstream paths that
# licenses/ruff/UPSTREAM.md lists as "Not copied" (ports of copyleft plugins: pylint, flake8-builtins, flake8-django,
# flake8-executable, refurb) are never copied (`fixtures` refuses one), so their rules are not recorded: not the
# candidate A005, nor DJ* or EXE*. Nor is the candidate INP001, which Ruff's tests lint only with a
# `namespace-packages` option.
RULES: dict[str, list[str]] = {
    "AIR001": ["airflow/AIR001.py"],
    "AIR002": ["airflow/AIR002.py"],
    "AIR301": [
        "airflow/AIR301_airflow_plugin.py",
        "airflow/AIR301_args.py",
        "airflow/AIR301_class_attribute.py",
        "airflow/AIR301_context.py",
        "airflow/AIR301_decorator.py",
        "airflow/AIR301_names.py",
        "airflow/AIR301_names_fix.py",
        "airflow/AIR301_names_try.py",
        "airflow/AIR301_provider_names_fix.py",
    ],
    "AIR302": [
        "airflow/AIR302_amazon.py",
        "airflow/AIR302_celery.py",
        "airflow/AIR302_common_sql.py",
        "airflow/AIR302_daskexecutor.py",
        "airflow/AIR302_druid.py",
        "airflow/AIR302_fab.py",
        "airflow/AIR302_hdfs.py",
        "airflow/AIR302_hive.py",
        "airflow/AIR302_http.py",
        "airflow/AIR302_jdbc.py",
        "airflow/AIR302_kubernetes.py",
        "airflow/AIR302_mysql.py",
        "airflow/AIR302_oracle.py",
        "airflow/AIR302_papermill.py",
        "airflow/AIR302_pig.py",
        "airflow/AIR302_postgres.py",
        "airflow/AIR302_presto.py",
        "airflow/AIR302_samba.py",
        "airflow/AIR302_slack.py",
        "airflow/AIR302_smtp.py",
        "airflow/AIR302_sqlite.py",
        "airflow/AIR302_standard.py",
        "airflow/AIR302_try.py",
        "airflow/AIR302_zendesk.py",
    ],
    "AIR303": ["airflow/AIR303.py"],
    "AIR311": ["airflow/AIR311_args.py", "airflow/AIR311_names.py", "airflow/AIR311_try.py"],
    "AIR312": ["airflow/AIR312.py", "airflow/AIR312_try.py"],
    "ANN001": ["flake8_annotations/annotation_presence.py"],
    "ANN002": ["flake8_annotations/annotation_presence.py"],
    "ANN003": ["flake8_annotations/annotation_presence.py"],
    "ANN201": [
        "flake8_annotations/allow_nested_overload.py",
        "flake8_annotations/allow_overload.py",
        "flake8_annotations/annotation_presence.py",
        "flake8_annotations/auto_return_type.py",
        "flake8_annotations/shadowed_builtins.py",
    ],
    "ANN202": [
        "flake8_annotations/allow_nested_overload.py",
        "flake8_annotations/allow_overload.py",
        "flake8_annotations/annotation_presence.py",
        "flake8_annotations/auto_return_type.py",
        "flake8_annotations/shadowed_builtins.py",
    ],
    "ANN204": [
        "flake8_annotations/allow_nested_overload.py",
        "flake8_annotations/allow_overload.py",
        "flake8_annotations/annotation_presence.py",
        "flake8_annotations/auto_return_type.py",
        "flake8_annotations/shadowed_builtins.py",
        "flake8_annotations/simple_magic_methods.py",
    ],
    "ANN205": [
        "flake8_annotations/allow_nested_overload.py",
        "flake8_annotations/allow_overload.py",
        "flake8_annotations/annotation_presence.py",
        "flake8_annotations/auto_return_type.py",
        "flake8_annotations/shadowed_builtins.py",
    ],
    "ANN206": [
        "flake8_annotations/allow_nested_overload.py",
        "flake8_annotations/allow_overload.py",
        "flake8_annotations/annotation_presence.py",
        "flake8_annotations/auto_return_type.py",
        "flake8_annotations/shadowed_builtins.py",
    ],
    "ANN401": ["flake8_annotations/annotation_presence.py"],
    "ARG001": ["flake8_unused_arguments/ARG.py"],
    "ARG002": ["flake8_unused_arguments/ARG.py"],
    "ARG003": ["flake8_unused_arguments/ARG.py"],
    "ARG004": ["flake8_unused_arguments/ARG.py"],
    "ARG005": ["flake8_unused_arguments/ARG.py"],
    "ASYNC100": ["flake8_async/ASYNC100.py"],
    "ASYNC105": ["flake8_async/ASYNC105.py"],
    "ASYNC109": ["flake8_async/ASYNC109_0.py", "flake8_async/ASYNC109_1.py"],
    "ASYNC110": ["flake8_async/ASYNC110.py"],
    "ASYNC115": ["flake8_async/ASYNC115.py"],
    "ASYNC116": ["flake8_async/ASYNC116.py"],
    "ASYNC210": ["flake8_async/ASYNC210.py"],
    "ASYNC212": ["flake8_async/ASYNC212.py"],
    "ASYNC220": ["flake8_async/ASYNC22x.py"],
    "ASYNC221": ["flake8_async/ASYNC22x.py"],
    "ASYNC222": ["flake8_async/ASYNC22x.py"],
    "ASYNC230": ["flake8_async/ASYNC230.py"],
    "ASYNC240": ["flake8_async/ASYNC240.py"],
    "ASYNC250": ["flake8_async/ASYNC250.py"],
    "ASYNC251": ["flake8_async/ASYNC251.py"],
    "B002": ["flake8_bugbear/B002.py"],
    "B003": ["flake8_bugbear/B003.py"],
    "B004": ["flake8_bugbear/B004.py"],
    "B005": ["flake8_bugbear/B005.py"],
    "B006": [
        "flake8_bugbear/B006_1.py",
        "flake8_bugbear/B006_1.pyi",
        "flake8_bugbear/B006_2.py",
        "flake8_bugbear/B006_3.py",
        "flake8_bugbear/B006_4.py",
        "flake8_bugbear/B006_5.py",
        "flake8_bugbear/B006_6.py",
        "flake8_bugbear/B006_7.py",
        "flake8_bugbear/B006_8.py",
        "flake8_bugbear/B006_9.py",
        "flake8_bugbear/B006_B008.py",
    ],
    "B007": ["flake8_bugbear/B007.py"],
    "B008": ["flake8_bugbear/B006_B008.py"],
    "B009": ["flake8_bugbear/B009_B010.py"],
    "B010": ["flake8_bugbear/B009_B010.py"],
    "B011": ["flake8_bugbear/B011.py"],
    "B012": ["flake8_bugbear/B012.py"],
    "B013": ["flake8_bugbear/B013.py"],
    "B014": ["flake8_bugbear/B014.py"],
    "B015": ["flake8_bugbear/B015.py"],
    "B016": ["flake8_bugbear/B016.py"],
    "B017": ["flake8_bugbear/B017_0.py", "flake8_bugbear/B017_1.py"],
    "B018": ["flake8_bugbear/B018.py"],
    "B019": ["flake8_bugbear/B019.py"],
    "B020": ["flake8_bugbear/B020.py"],
    "B021": ["flake8_bugbear/B021.py"],
    "B022": ["flake8_bugbear/B022.py"],
    "B023": ["flake8_bugbear/B023.py"],
    "B024": ["flake8_bugbear/B024.py"],
    "B025": ["flake8_bugbear/B025.py"],
    "B026": ["flake8_bugbear/B026.py"],
    "B027": ["flake8_bugbear/B027.py", "flake8_bugbear/B027.pyi"],
    "B028": ["flake8_bugbear/B028.py"],
    "B029": ["flake8_bugbear/B029.py"],
    "B030": ["flake8_bugbear/B030.py"],
    "B031": ["flake8_bugbear/B031.py"],
    "B032": ["flake8_bugbear/B032.py"],
    "B033": ["flake8_bugbear/B033.py"],
    "B034": ["flake8_bugbear/B034.py"],
    "B035": ["flake8_bugbear/B035.py"],
    "B039": ["flake8_bugbear/B039.py"],
    "B904": ["flake8_bugbear/B904.py"],
    "B905": ["flake8_bugbear/B905.py"],
    "B911": ["flake8_bugbear/B911.py"],
    "B912": ["flake8_bugbear/B912.py"],
    "BLE001": ["flake8_blind_except/BLE.py"],
    "C400": ["flake8_comprehensions/C400.py"],
    "C401": ["flake8_comprehensions/C401.py"],
    "C402": ["flake8_comprehensions/C402.py"],
    "C403": ["flake8_comprehensions/C403.py"],
    "C404": ["flake8_comprehensions/C404.py"],
    "C405": ["flake8_comprehensions/C405.py"],
    "C406": ["flake8_comprehensions/C406.py"],
    "C408": ["flake8_comprehensions/C408.py"],
    "C409": ["flake8_comprehensions/C409.py"],
    "C410": ["flake8_comprehensions/C410.py"],
    "C411": ["flake8_comprehensions/C411.py"],
    "C413": ["flake8_comprehensions/C413.py"],
    "C414": ["flake8_comprehensions/C414.py"],
    "C415": ["flake8_comprehensions/C415.py"],
    "C416": ["flake8_comprehensions/C416.py"],
    "C417": ["flake8_comprehensions/C417.py", "flake8_comprehensions/C417_1.py"],
    "C418": ["flake8_comprehensions/C418.py"],
    "C419": ["flake8_comprehensions/C419.py", "flake8_comprehensions/C419_2.py"],
    "C420": [
        "flake8_comprehensions/C420.py",
        "flake8_comprehensions/C420_1.py",
        "flake8_comprehensions/C420_2.py",
        "flake8_comprehensions/C420_3.py",
    ],
    "C901": ["mccabe/C901.py"],
    "COM818": ["flake8_commas/COM81.py", "flake8_commas/COM81_syntax_error.py"],
    "D100": ["pydocstyle/all.py"],
    "D101": ["pydocstyle/all.py"],
    "D102": ["pydocstyle/all.py"],
    "D103": ["pydocstyle/all.py"],
    "D104": ["pydocstyle/all.py"],
    "D105": ["pydocstyle/all.py"],
    "D106": ["pydocstyle/all.py"],
    "D107": ["pydocstyle/all.py"],
    "D200": ["pydocstyle/D.py", "pydocstyle/D200.py"],
    "D205": ["pydocstyle/D.py"],
    "D209": ["pydocstyle/D.py", "pydocstyle/D209_D400.py"],
    "D212": ["pydocstyle/D.py"],
    "D215": ["pydocstyle/D215.py", "pydocstyle/sections.py"],
    "D300": ["pydocstyle/bom.py"],
    "D301": ["pydocstyle/D.py", "pydocstyle/D301.py"],
    "D400": ["pydocstyle/D209_D400.py"],
    "D401": ["pydocstyle/D401.py"],
    "D402": ["pydocstyle/D.py", "pydocstyle/D402.py"],
    "D403": ["pydocstyle/D.py", "pydocstyle/D403.py"],
    "D404": ["pydocstyle/D.py"],
    "D405": ["pydocstyle/sections.py", "pydocstyle/sphinx_directive.py"],
    "D406": ["pydocstyle/sections.py"],
    "D407": ["pydocstyle/sections.py"],
    "D408": ["pydocstyle/sections.py"],
    "D409": ["pydocstyle/sections.py"],
    "D410": ["pydocstyle/D410.py", "pydocstyle/sections.py"],
    "D411": ["pydocstyle/sections.py"],
    "D412": ["pydocstyle/sections.py", "pydocstyle/sphinx.py"],
    "D413": ["pydocstyle/D413.py", "pydocstyle/sections.py", "pydocstyle/sphinx_directive.py"],
    "D414": ["pydocstyle/sections.py"],
    "D415": ["pydocstyle/D.py", "pydocstyle/D400_415.py"],
    "D416": ["pydocstyle/D.py"],
    "D418": ["pydocstyle/D.py", "pydocstyle/D418.pyi"],
    "D419": ["pydocstyle/D.py"],
    "DTZ001": ["flake8_datetimez/DTZ001.py"],
    "DTZ002": ["flake8_datetimez/DTZ002.py"],
    "DTZ003": ["flake8_datetimez/DTZ003.py"],
    "DTZ004": ["flake8_datetimez/DTZ004.py"],
    "DTZ005": ["flake8_datetimez/DTZ005.py"],
    "DTZ006": ["flake8_datetimez/DTZ006.py"],
    "DTZ007": ["flake8_datetimez/DTZ007.py"],
    "DTZ011": ["flake8_datetimez/DTZ011.py"],
    "DTZ012": ["flake8_datetimez/DTZ012.py"],
    "DTZ901": ["flake8_datetimez/DTZ901.py"],
    "E401": ["pycodestyle/E40.py"],
    "E402": [
        "pycodestyle/E40.py",
        "pycodestyle/E402_0.py",
        "pycodestyle/E402_1.py",
        "pycodestyle/E402_2.py",
        "pycodestyle/E402_3.py",
        "pycodestyle/E402_4.py",
        "pycodestyle/E402_5.py",
    ],
    "E711": ["pycodestyle/E711.py", "pycodestyle/constant_literals.py"],
    "E712": ["pycodestyle/E712.py", "pycodestyle/constant_literals.py"],
    "E713": ["pycodestyle/E713.py"],
    "E714": ["pycodestyle/E714.py"],
    "E721": ["pycodestyle/E721.py"],
    "E722": ["pycodestyle/E722.py"],
    "E731": ["pycodestyle/E731.py"],
    "E741": ["pycodestyle/E741.py", "pycodestyle/E741.pyi", "ruff/noqa.py"],
    "E742": ["pycodestyle/E742.py"],
    "E743": ["pycodestyle/E743.py"],
    "EM101": ["flake8_errmsg/EM.py", "flake8_errmsg/EM101_byte_string.py"],
    "EM102": ["flake8_errmsg/EM.py"],
    "EM103": ["flake8_errmsg/EM.py"],
    "ERA001": ["eradicate/ERA001.py", "ruff/RUF100_5.py"],
    "F401": [
        "pyflakes/F401_0.py",
        "pyflakes/F401_1.py",
        "pyflakes/F401_10.py",
        "pyflakes/F401_11.py",
        "pyflakes/F401_12.py",
        "pyflakes/F401_13.py",
        "pyflakes/F401_14.py",
        "pyflakes/F401_15.py",
        "pyflakes/F401_16.py",
        "pyflakes/F401_17.py",
        "pyflakes/F401_18.py",
        "pyflakes/F401_19.py",
        "pyflakes/F401_2.py",
        "pyflakes/F401_20.py",
        "pyflakes/F401_21.py",
        "pyflakes/F401_22.py",
        "pyflakes/F401_23.py",
        "pyflakes/F401_24/__init__.py",
        "pyflakes/F401_25__all_nonempty/__init__.py",
        "pyflakes/F401_26__all_empty/__init__.py",
        "pyflakes/F401_27__all_mistyped/__init__.py",
        "pyflakes/F401_28__all_multiple/__init__.py",
        "pyflakes/F401_29__all_conditional/__init__.py",
        "pyflakes/F401_3.py",
        "pyflakes/F401_32.py",
        "pyflakes/F401_34.py",
        "pyflakes/F401_35.py",
        "pyflakes/F401_4.py",
        "pyflakes/F401_5.py",
        "pyflakes/F401_6.py",
        "pyflakes/F401_7.py",
        "pyflakes/F401_8.py",
        "pyflakes/F401_9.py",
        "pyflakes/__init__.py",
        "pyflakes/future_annotations.py",
        "pyflakes/multi_statement_lines.py",
        "ruff/RUF050_F401.py",
        "ruff/RUF100_1.py",
        "ruff/RUF100_2.py",
        "ruff/RUF100_4.py",
        "ruff/flake8_noqa.py",
        "ruff/ruff_noqa_all.py",
        "ruff/ruff_noqa_codes.py",
        "ruff/ruff_noqa_invalid.py",
    ],
    "F402": ["pyflakes/F402.py"],
    "F403": ["pyflakes/F403.py"],
    "F404": ["pyflakes/F404_0.py", "pyflakes/F404_1.py", "syntax_errors/late_future_import.py"],
    "F405": ["pyflakes/F405.py"],
    "F406": ["pyflakes/F406.py"],
    "F407": ["pyflakes/F407.py"],
    "F501": ["pyflakes/F50x.py"],
    "F502": ["pyflakes/F502.py", "pyflakes/F50x.py"],
    "F503": ["pyflakes/F503.py", "pyflakes/F50x.py"],
    "F504": ["pyflakes/F504.py", "pyflakes/F50x.py"],
    "F505": ["pyflakes/F504.py", "pyflakes/F50x.py"],
    "F506": ["pyflakes/F50x.py"],
    "F507": ["pyflakes/F50x.py"],
    "F508": ["pyflakes/F50x.py"],
    "F509": ["pyflakes/F50x.py"],
    "F521": ["pyflakes/F521.py"],
    "F522": ["pyflakes/F522.py"],
    "F523": ["pyflakes/F523.py"],
    "F524": ["pyflakes/F524.py"],
    "F525": ["pyflakes/F525.py"],
    "F541": ["pyflakes/F541.py", "ruff/RUF100_7.py"],
    "F601": ["pyflakes/F601.py"],
    "F602": ["pyflakes/F602.py"],
    "F622": ["pyflakes/F622.py"],
    "F631": ["pyflakes/F631.py"],
    "F632": ["pycodestyle/constant_literals.py", "pyflakes/F632.py"],
    "F633": ["pyflakes/F633.py"],
    "F634": ["pyflakes/F634.py"],
    "F701": ["pyflakes/F701.py"],
    "F702": ["pyflakes/F702.py"],
    "F704": ["pyflakes/F704.py", "syntax_errors/yield_scope.py"],
    "F706": ["pyflakes/F706.py", "syntax_errors/return_outside_function.py"],
    "F707": ["pyflakes/F707.py"],
    "F722": ["pyflakes/F722.py", "pyflakes/F722_1.py"],
    "F811": [f"pyflakes/F811_{n}.py" for n in [*range(0, 29), 30, 31, 32, 35, 36]]
    + ["pyflakes/F811_29.pyi", "pyflakes/F811_33.pyi"],
    "F821": [
        "pyflakes/F821_0.py",
        "pyflakes/F821_1.py",
        "pyflakes/F821_10.py",
        "pyflakes/F821_11.py",
        "pyflakes/F821_11.pyi",
        "pyflakes/F821_12.py",
        "pyflakes/F821_13.py",
        "pyflakes/F821_14.py",
        "pyflakes/F821_15.py",
        "pyflakes/F821_16.py",
        "pyflakes/F821_17.py",
        "pyflakes/F821_18.py",
        "pyflakes/F821_19.py",
        "pyflakes/F821_2.py",
        "pyflakes/F821_20.py",
        "pyflakes/F821_21.py",
        "pyflakes/F821_23.py",
        "pyflakes/F821_24.py",
        "pyflakes/F821_25.py",
        "pyflakes/F821_26.py",
        "pyflakes/F821_26.pyi",
        "pyflakes/F821_27.py",
        "pyflakes/F821_28.py",
        "pyflakes/F821_3.py",
        "pyflakes/F821_30.py",
        "pyflakes/F821_31.py",
        "pyflakes/F821_32.pyi",
        "pyflakes/F821_33.py",
        "pyflakes/F821_34.py",
        "pyflakes/F821_34.pyi",
        "pyflakes/F821_4.py",
        "pyflakes/F821_5.py",
        "pyflakes/F821_5.pyi",
        "pyflakes/F821_6.py",
        "pyflakes/F821_7.py",
        "pyflakes/F821_8.pyi",
        "pyflakes/F821_9.py",
        "pyflakes/__init__.py",
        "pyflakes/builtins.py",
        "pyflakes/future_annotations.py",
        "pyflakes/typing_modules.py",
        "ruff/RUF100_3.py",
    ],
    "F822": [
        "pyflakes/F822_0.py",
        "pyflakes/F822_0.pyi",
        "pyflakes/F822_1.py",
        "pyflakes/F822_1b.py",
        "pyflakes/F822_2.py",
        "pyflakes/F822_3.py",
        "pyflakes/__init__.py",
    ],
    "F823": ["pyflakes/F823.py"],
    "F841": [
        "pyflakes/F841_0.py",
        "pyflakes/F841_1.py",
        "pyflakes/F841_2.py",
        "pyflakes/F841_3.py",
        "ruff/RUF100_7.py",
        "ruff/flake8_noqa.py",
        "ruff/noqa.py",
        "ruff/ruff_noqa_all.py",
        "ruff/ruff_noqa_codes.py",
        "ruff/ruff_noqa_invalid.py",
    ],
    "F842": ["pyflakes/F842.py"],
    "F901": ["pyflakes/F901.py"],
    "FA100": [
        "flake8_future_annotations/edge_case.py",
        "flake8_future_annotations/from_typing_import.py",
        "flake8_future_annotations/from_typing_import_many.py",
        "flake8_future_annotations/import_typing.py",
        "flake8_future_annotations/import_typing_as.py",
        "flake8_future_annotations/no_future_import_uses_lowercase.py",
        "flake8_future_annotations/no_future_import_uses_union.py",
        "flake8_future_annotations/no_future_import_uses_union_inner.py",
        "flake8_future_annotations/ok_no_types.py",
        "flake8_future_annotations/ok_non_simplifiable_types.py",
        "flake8_future_annotations/ok_uses_future.py",
        "flake8_future_annotations/ok_variable_name.py",
        "pyupgrade/UP006_4.py",
        "pyupgrade/UP007_2.py",
        "pyupgrade/UP045_2.py",
    ],
    "FA102": [
        "flake8_future_annotations/no_future_import_uses_lowercase.py",
        "flake8_future_annotations/no_future_import_uses_preview_generics.py",
        "flake8_future_annotations/no_future_import_uses_union.py",
        "flake8_future_annotations/no_future_import_uses_union_inner.py",
        "flake8_future_annotations/ok_no_types.py",
        "flake8_future_annotations/ok_quoted_type.py",
        "flake8_future_annotations/ok_uses_future.py",
    ],
    "FAST001": ["fastapi/FAST001.py"],
    "FAST002": ["fastapi/FAST002_0.py", "fastapi/FAST002_1.py", "fastapi/FAST002_2.py"],
    "FAST003": ["fastapi/FAST003.py"],
    "FBT001": ["flake8_boolean_trap/FBT.py"],
    "FBT002": ["flake8_boolean_trap/FBT.py"],
    "FBT003": ["flake8_boolean_trap/FBT.py"],
    "FIX001": ["flake8_fixme/T00.py"],
    "FIX002": ["flake8_fixme/T00.py"],
    "FIX003": ["flake8_fixme/T00.py"],
    "FIX004": ["flake8_fixme/T00.py"],
    "FLY002": ["flynt/FLY002.py"],
    "G001": [
        "flake8_logging_format/G001.py",
        "flake8_logging_format/G002.py",
        "flake8_logging_format/G003.py",
        "flake8_logging_format/G004.py",
        "flake8_logging_format/G004_arg_order.py",
        "flake8_logging_format/G004_implicit_concat.py",
        "flake8_logging_format/G010.py",
        "flake8_logging_format/G101_1.py",
        "flake8_logging_format/G101_2.py",
        "flake8_logging_format/G201.py",
        "flake8_logging_format/G202.py",
        "flake8_logging_format/G_argparse_parser_error_ok.py",
        "flake8_logging_format/G_extra_ok.py",
        "flake8_logging_format/G_extra_str_format_ok.py",
        "flake8_logging_format/G_simple_ok.py",
        "flake8_logging_format/G_warnings_ok.py",
    ],
    "G002": [
        "flake8_logging_format/G001.py",
        "flake8_logging_format/G002.py",
        "flake8_logging_format/G003.py",
        "flake8_logging_format/G004.py",
        "flake8_logging_format/G004_arg_order.py",
        "flake8_logging_format/G004_implicit_concat.py",
        "flake8_logging_format/G010.py",
        "flake8_logging_format/G101_1.py",
        "flake8_logging_format/G101_2.py",
        "flake8_logging_format/G201.py",
        "flake8_logging_format/G202.py",
        "flake8_logging_format/G_argparse_parser_error_ok.py",
        "flake8_logging_format/G_extra_ok.py",
        "flake8_logging_format/G_extra_str_format_ok.py",
        "flake8_logging_format/G_simple_ok.py",
        "flake8_logging_format/G_warnings_ok.py",
    ],
    "G003": [
        "flake8_logging_format/G001.py",
        "flake8_logging_format/G002.py",
        "flake8_logging_format/G003.py",
        "flake8_logging_format/G004.py",
        "flake8_logging_format/G004_arg_order.py",
        "flake8_logging_format/G004_implicit_concat.py",
        "flake8_logging_format/G010.py",
        "flake8_logging_format/G101_1.py",
        "flake8_logging_format/G101_2.py",
        "flake8_logging_format/G201.py",
        "flake8_logging_format/G202.py",
        "flake8_logging_format/G_argparse_parser_error_ok.py",
        "flake8_logging_format/G_extra_ok.py",
        "flake8_logging_format/G_extra_str_format_ok.py",
        "flake8_logging_format/G_simple_ok.py",
        "flake8_logging_format/G_warnings_ok.py",
    ],
    "G004": [
        "flake8_logging_format/G001.py",
        "flake8_logging_format/G002.py",
        "flake8_logging_format/G003.py",
        "flake8_logging_format/G004.py",
        "flake8_logging_format/G004_arg_order.py",
        "flake8_logging_format/G004_implicit_concat.py",
        "flake8_logging_format/G010.py",
        "flake8_logging_format/G101_1.py",
        "flake8_logging_format/G101_2.py",
        "flake8_logging_format/G201.py",
        "flake8_logging_format/G202.py",
        "flake8_logging_format/G_argparse_parser_error_ok.py",
        "flake8_logging_format/G_extra_ok.py",
        "flake8_logging_format/G_extra_str_format_ok.py",
        "flake8_logging_format/G_simple_ok.py",
        "flake8_logging_format/G_warnings_ok.py",
    ],
    "G010": [
        "flake8_logging_format/G001.py",
        "flake8_logging_format/G002.py",
        "flake8_logging_format/G003.py",
        "flake8_logging_format/G004.py",
        "flake8_logging_format/G004_arg_order.py",
        "flake8_logging_format/G004_implicit_concat.py",
        "flake8_logging_format/G010.py",
        "flake8_logging_format/G101_1.py",
        "flake8_logging_format/G101_2.py",
        "flake8_logging_format/G201.py",
        "flake8_logging_format/G202.py",
        "flake8_logging_format/G_argparse_parser_error_ok.py",
        "flake8_logging_format/G_extra_ok.py",
        "flake8_logging_format/G_extra_str_format_ok.py",
        "flake8_logging_format/G_simple_ok.py",
        "flake8_logging_format/G_warnings_ok.py",
    ],
    "G101": [
        "flake8_logging_format/G001.py",
        "flake8_logging_format/G002.py",
        "flake8_logging_format/G003.py",
        "flake8_logging_format/G004.py",
        "flake8_logging_format/G004_arg_order.py",
        "flake8_logging_format/G004_implicit_concat.py",
        "flake8_logging_format/G010.py",
        "flake8_logging_format/G101_1.py",
        "flake8_logging_format/G101_2.py",
        "flake8_logging_format/G201.py",
        "flake8_logging_format/G202.py",
        "flake8_logging_format/G_argparse_parser_error_ok.py",
        "flake8_logging_format/G_extra_ok.py",
        "flake8_logging_format/G_extra_str_format_ok.py",
        "flake8_logging_format/G_simple_ok.py",
        "flake8_logging_format/G_warnings_ok.py",
    ],
    "G201": [
        "flake8_logging_format/G001.py",
        "flake8_logging_format/G002.py",
        "flake8_logging_format/G003.py",
        "flake8_logging_format/G004.py",
        "flake8_logging_format/G004_arg_order.py",
        "flake8_logging_format/G004_implicit_concat.py",
        "flake8_logging_format/G010.py",
        "flake8_logging_format/G101_1.py",
        "flake8_logging_format/G101_2.py",
        "flake8_logging_format/G201.py",
        "flake8_logging_format/G202.py",
        "flake8_logging_format/G_argparse_parser_error_ok.py",
        "flake8_logging_format/G_extra_ok.py",
        "flake8_logging_format/G_extra_str_format_ok.py",
        "flake8_logging_format/G_simple_ok.py",
        "flake8_logging_format/G_warnings_ok.py",
    ],
    "G202": [
        "flake8_logging_format/G001.py",
        "flake8_logging_format/G002.py",
        "flake8_logging_format/G003.py",
        "flake8_logging_format/G004.py",
        "flake8_logging_format/G004_arg_order.py",
        "flake8_logging_format/G004_implicit_concat.py",
        "flake8_logging_format/G010.py",
        "flake8_logging_format/G101_1.py",
        "flake8_logging_format/G101_2.py",
        "flake8_logging_format/G201.py",
        "flake8_logging_format/G202.py",
        "flake8_logging_format/G_argparse_parser_error_ok.py",
        "flake8_logging_format/G_extra_ok.py",
        "flake8_logging_format/G_extra_str_format_ok.py",
        "flake8_logging_format/G_simple_ok.py",
        "flake8_logging_format/G_warnings_ok.py",
    ],
    "ICN001": ["flake8_import_conventions/defaults.py", "flake8_import_conventions/tricky.py"],
    "INP001": [
        "flake8_no_pep420/test_fail_empty/example.py",
        "flake8_no_pep420/test_fail_nonempty/example.py",
        "flake8_no_pep420/test_ignored/example.py",
        "flake8_no_pep420/test_pass_init/example.py",
        "flake8_no_pep420/test_pass_namespace_package/example.py",
        "flake8_no_pep420/test_pass_pep723/script.py",
        "flake8_no_pep420/test_pass_pyi/example.pyi",
        "flake8_no_pep420/test_pass_shebang/example.py",
    ],
    "INT001": ["flake8_gettext/INT001.py"],
    "INT002": ["flake8_gettext/INT002.py"],
    "INT003": ["flake8_gettext/INT003.py"],
    "ISC001": [
        "flake8_implicit_str_concat/ISC.py",
        "flake8_implicit_str_concat/ISC_syntax_error.py",
        "flake8_implicit_str_concat/ISC_syntax_error_2.py",
    ],
    "ISC003": ["flake8_implicit_str_concat/ISC.py", "flake8_implicit_str_concat/ISC003_docstring.py"],
    "ISC004": ["flake8_implicit_str_concat/ISC004.py"],
    "LOG001": ["flake8_logging/LOG001.py"],
    "LOG002": ["flake8_logging/LOG002.py"],
    "LOG004": ["flake8_logging/LOG004_0.py", "flake8_logging/LOG004_1.py"],
    "LOG007": ["flake8_logging/LOG007.py"],
    "LOG009": ["flake8_logging/LOG009.py"],
    "LOG014": ["flake8_logging/LOG014_0.py", "flake8_logging/LOG014_1.py"],
    "LOG015": ["flake8_logging/LOG015.py"],
    "N801": ["pep8_naming/N801.py"],
    "N802": ["pep8_naming/N802.py"],
    "N803": ["pep8_naming/N803.py", "pep8_naming/N804.py"],
    "N804": ["pep8_naming/N804.py"],
    "N805": ["pep8_naming/N805.py"],
    "N806": ["pep8_naming/N806.py"],
    "N807": ["pep8_naming/N807.py"],
    "N811": ["pep8_naming/N811.py"],
    "N812": ["pep8_naming/N812.py"],
    "N813": ["pep8_naming/N813.py"],
    "N814": ["pep8_naming/N814.py"],
    "N815": ["pep8_naming/N815.py"],
    "N816": ["pep8_naming/N816.py"],
    "N817": ["pep8_naming/N817.py"],
    "N818": ["pep8_naming/N818.py"],
    "N999": [
        "pep8_naming/N999/module/MODULE/__init__.py",
        "pep8_naming/N999/module/MODULE/file.py",
        "pep8_naming/N999/module/flake9/__init__.py",
        "pep8_naming/N999/module/invalid_name/0001_initial.py",
        "pep8_naming/N999/module/invalid_name/import.py",
        "pep8_naming/N999/module/mod with spaces/__init__.py",
        "pep8_naming/N999/module/mod with spaces/file.py",
        "pep8_naming/N999/module/mod-with-dashes/__init__.py",
        "pep8_naming/N999/module/valid_name/__init__.py",
        "pep8_naming/N999/module/valid_name/__main__.py",
        "pep8_naming/N999/module/valid_name/__setup__.py",
        "pep8_naming/N999/module/valid_name/file-with-dashes.py",
    ],
    "NPY001": ["numpy/NPY001.py"],
    "NPY002": ["numpy/NPY002.py"],
    "NPY003": ["numpy/NPY003.py"],
    "NPY201": ["numpy/NPY201.py", "numpy/NPY201_2.py", "numpy/NPY201_3.py"],
    "PD002": ["pandas_vet/PD002.py"],
    "PD012": ["pandas_vet/pandas_use_of_dot_read_table.py"],
    "PD101": ["pandas_vet/PD101.py"],
    "PERF101": ["perflint/PERF101.py"],
    "PERF102": ["perflint/PERF102.py"],
    "PERF203": ["perflint/PERF203.py"],
    "PERF401": ["perflint/PERF401.py"],
    "PERF402": ["perflint/PERF402.py"],
    "PERF403": ["perflint/PERF403.py"],
    "PGH003": ["pygrep_hooks/PGH003_0.py", "pygrep_hooks/PGH003_1.py"],
    "PGH004": [
        "pygrep_hooks/PGH004_0.py",
        "pygrep_hooks/PGH004_1.py",
        "pygrep_hooks/PGH004_2.py",
        "pygrep_hooks/PGH004_3.py",
    ],
    "PGH005": ["pygrep_hooks/PGH005_0.py"],
    "PIE790": ["flake8_pie/PIE790.py", "flake8_pyi/PYI021_1.pyi"],
    "PIE794": ["flake8_pie/PIE794.py"],
    "PIE796": ["flake8_pie/PIE796.py", "flake8_pie/PIE796.pyi"],
    "PIE800": ["flake8_pie/PIE800.py"],
    "PIE804": ["flake8_pie/PIE804.py"],
    "PIE807": ["flake8_pie/PIE807.py"],
    "PIE808": ["flake8_pie/PIE808.py"],
    "PIE810": ["flake8_pie/PIE810.py"],
    "PT001": ["flake8_pytest_style/PT001.py"],
    "PT002": ["flake8_pytest_style/PT002.py"],
    "PT003": ["flake8_pytest_style/PT003.py"],
    "PT006": ["flake8_pytest_style/PT006_and_PT007.py"],
    "PT007": ["flake8_pytest_style/PT006_and_PT007.py"],
    "PT008": ["flake8_pytest_style/PT008.py"],
    "PT009": ["flake8_pytest_style/PT009.py"],
    "PT010": ["flake8_pytest_style/PT010.py"],
    "PT011": ["flake8_pytest_style/PT011.py"],
    "PT012": ["flake8_pytest_style/PT012.py"],
    "PT013": ["flake8_pytest_style/PT013.py"],
    "PT014": ["flake8_pytest_style/PT014.py"],
    "PT015": ["flake8_pytest_style/PT015.py"],
    "PT016": ["flake8_pytest_style/PT016.py"],
    "PT017": ["flake8_pytest_style/PT017.py"],
    "PT018": ["flake8_pytest_style/PT018.py"],
    "PT019": ["flake8_pytest_style/PT019.py"],
    "PT020": ["flake8_pytest_style/PT020.py", "flake8_pytest_style/PT020_1.py", "flake8_pytest_style/PT020_2.py"],
    "PT021": ["flake8_pytest_style/PT021.py"],
    "PT022": ["flake8_pytest_style/PT022.py"],
    "PT023": ["flake8_pytest_style/PT023.py"],
    "PT024": ["flake8_pytest_style/PT024.py"],
    "PT025": ["flake8_pytest_style/PT025.py"],
    "PT026": ["flake8_pytest_style/PT026.py"],
    "PT027": ["flake8_pytest_style/PT027_0.py", "flake8_pytest_style/PT027_1.py"],
    "PT028": ["flake8_pytest_style/PT028.py", "flake8_pytest_style/is_pytest_test.py"],
    "PT030": ["flake8_pytest_style/PT030.py"],
    "PT031": ["flake8_pytest_style/PT031.py"],
    "PTH100": [
        "flake8_use_pathlib/full_name.py",
        "flake8_use_pathlib/import_as.py",
        "flake8_use_pathlib/import_from.py",
        "flake8_use_pathlib/import_from_as.py",
        "flake8_use_pathlib/use_pathlib.py",
    ],
    "PTH101": [
        "flake8_use_pathlib/full_name.py",
        "flake8_use_pathlib/import_as.py",
        "flake8_use_pathlib/import_from.py",
        "flake8_use_pathlib/import_from_as.py",
        "flake8_use_pathlib/use_pathlib.py",
    ],
    "PTH102": [
        "flake8_use_pathlib/full_name.py",
        "flake8_use_pathlib/import_as.py",
        "flake8_use_pathlib/import_from.py",
        "flake8_use_pathlib/import_from_as.py",
        "flake8_use_pathlib/use_pathlib.py",
    ],
    "PTH103": [
        "flake8_use_pathlib/full_name.py",
        "flake8_use_pathlib/import_as.py",
        "flake8_use_pathlib/import_from.py",
        "flake8_use_pathlib/import_from_as.py",
        "flake8_use_pathlib/use_pathlib.py",
    ],
    "PTH104": [
        "flake8_use_pathlib/full_name.py",
        "flake8_use_pathlib/import_as.py",
        "flake8_use_pathlib/import_from.py",
        "flake8_use_pathlib/import_from_as.py",
        "flake8_use_pathlib/use_pathlib.py",
    ],
    "PTH105": [
        "flake8_use_pathlib/full_name.py",
        "flake8_use_pathlib/import_as.py",
        "flake8_use_pathlib/import_from.py",
        "flake8_use_pathlib/import_from_as.py",
        "flake8_use_pathlib/use_pathlib.py",
    ],
    "PTH106": [
        "flake8_use_pathlib/full_name.py",
        "flake8_use_pathlib/import_as.py",
        "flake8_use_pathlib/import_from.py",
        "flake8_use_pathlib/import_from_as.py",
        "flake8_use_pathlib/use_pathlib.py",
    ],
    "PTH107": [
        "flake8_use_pathlib/full_name.py",
        "flake8_use_pathlib/import_as.py",
        "flake8_use_pathlib/import_from.py",
        "flake8_use_pathlib/import_from_as.py",
        "flake8_use_pathlib/use_pathlib.py",
    ],
    "PTH108": [
        "flake8_use_pathlib/full_name.py",
        "flake8_use_pathlib/import_as.py",
        "flake8_use_pathlib/import_from.py",
        "flake8_use_pathlib/import_from_as.py",
        "flake8_use_pathlib/use_pathlib.py",
    ],
    "PTH109": [
        "flake8_use_pathlib/full_name.py",
        "flake8_use_pathlib/import_as.py",
        "flake8_use_pathlib/import_from.py",
        "flake8_use_pathlib/import_from_as.py",
        "flake8_use_pathlib/use_pathlib.py",
    ],
    "PTH110": [
        "flake8_use_pathlib/full_name.py",
        "flake8_use_pathlib/import_as.py",
        "flake8_use_pathlib/import_from.py",
        "flake8_use_pathlib/import_from_as.py",
        "flake8_use_pathlib/use_pathlib.py",
    ],
    "PTH111": [
        "flake8_use_pathlib/full_name.py",
        "flake8_use_pathlib/import_as.py",
        "flake8_use_pathlib/import_from.py",
        "flake8_use_pathlib/import_from_as.py",
        "flake8_use_pathlib/use_pathlib.py",
    ],
    "PTH112": [
        "flake8_use_pathlib/full_name.py",
        "flake8_use_pathlib/import_as.py",
        "flake8_use_pathlib/import_from.py",
        "flake8_use_pathlib/import_from_as.py",
        "flake8_use_pathlib/use_pathlib.py",
    ],
    "PTH113": [
        "flake8_use_pathlib/full_name.py",
        "flake8_use_pathlib/import_as.py",
        "flake8_use_pathlib/import_from.py",
        "flake8_use_pathlib/import_from_as.py",
        "flake8_use_pathlib/use_pathlib.py",
    ],
    "PTH114": [
        "flake8_use_pathlib/full_name.py",
        "flake8_use_pathlib/import_as.py",
        "flake8_use_pathlib/import_from.py",
        "flake8_use_pathlib/import_from_as.py",
        "flake8_use_pathlib/use_pathlib.py",
    ],
    "PTH115": [
        "flake8_use_pathlib/full_name.py",
        "flake8_use_pathlib/import_as.py",
        "flake8_use_pathlib/import_from.py",
        "flake8_use_pathlib/import_from_as.py",
        "flake8_use_pathlib/use_pathlib.py",
    ],
    "PTH116": [
        "flake8_use_pathlib/full_name.py",
        "flake8_use_pathlib/import_as.py",
        "flake8_use_pathlib/import_from.py",
        "flake8_use_pathlib/import_from_as.py",
        "flake8_use_pathlib/use_pathlib.py",
    ],
    "PTH117": [
        "flake8_use_pathlib/full_name.py",
        "flake8_use_pathlib/import_as.py",
        "flake8_use_pathlib/import_from.py",
        "flake8_use_pathlib/import_from_as.py",
        "flake8_use_pathlib/use_pathlib.py",
    ],
    "PTH118": [
        "flake8_use_pathlib/full_name.py",
        "flake8_use_pathlib/import_as.py",
        "flake8_use_pathlib/import_from.py",
        "flake8_use_pathlib/import_from_as.py",
        "flake8_use_pathlib/use_pathlib.py",
    ],
    "PTH119": [
        "flake8_use_pathlib/full_name.py",
        "flake8_use_pathlib/import_as.py",
        "flake8_use_pathlib/import_from.py",
        "flake8_use_pathlib/import_from_as.py",
        "flake8_use_pathlib/use_pathlib.py",
    ],
    "PTH120": [
        "flake8_use_pathlib/full_name.py",
        "flake8_use_pathlib/import_as.py",
        "flake8_use_pathlib/import_from.py",
        "flake8_use_pathlib/import_from_as.py",
        "flake8_use_pathlib/use_pathlib.py",
    ],
    "PTH121": [
        "flake8_use_pathlib/full_name.py",
        "flake8_use_pathlib/import_as.py",
        "flake8_use_pathlib/import_from.py",
        "flake8_use_pathlib/import_from_as.py",
        "flake8_use_pathlib/use_pathlib.py",
    ],
    "PTH122": [
        "flake8_use_pathlib/full_name.py",
        "flake8_use_pathlib/import_as.py",
        "flake8_use_pathlib/import_from.py",
        "flake8_use_pathlib/import_from_as.py",
        "flake8_use_pathlib/use_pathlib.py",
    ],
    "PTH123": [
        "flake8_use_pathlib/PTH123.py",
        "flake8_use_pathlib/full_name.py",
        "flake8_use_pathlib/import_as.py",
        "flake8_use_pathlib/import_from.py",
        "flake8_use_pathlib/import_from_as.py",
        "flake8_use_pathlib/use_pathlib.py",
    ],
    "PTH124": ["flake8_use_pathlib/py_path_1.py", "flake8_use_pathlib/py_path_2.py"],
    "PTH201": ["flake8_use_pathlib/PTH201.py"],
    "PTH202": ["flake8_use_pathlib/PTH202.py", "flake8_use_pathlib/PTH202_2.py"],
    "PTH203": ["flake8_use_pathlib/PTH203.py"],
    "PTH204": ["flake8_use_pathlib/PTH204.py"],
    "PTH205": ["flake8_use_pathlib/PTH205.py"],
    "PTH206": ["flake8_use_pathlib/PTH206.py"],
    "PTH207": ["flake8_use_pathlib/PTH207.py"],
    "PTH208": ["flake8_use_pathlib/PTH208.py"],
    "PTH210": ["flake8_use_pathlib/PTH210.py", "flake8_use_pathlib/PTH210_1.py"],
    "PTH211": ["flake8_use_pathlib/PTH211.py"],
    "PYI001": ["flake8_pyi/PYI001.py", "flake8_pyi/PYI001.pyi"],
    "PYI002": ["flake8_pyi/PYI002.py", "flake8_pyi/PYI002.pyi"],
    "PYI003": ["flake8_pyi/PYI003.py", "flake8_pyi/PYI003.pyi"],
    "PYI004": ["flake8_pyi/PYI004.py", "flake8_pyi/PYI004.pyi"],
    "PYI005": ["flake8_pyi/PYI005.py", "flake8_pyi/PYI005.pyi"],
    "PYI006": ["flake8_pyi/PYI006.py", "flake8_pyi/PYI006.pyi"],
    "PYI007": ["flake8_pyi/PYI007.py", "flake8_pyi/PYI007.pyi"],
    "PYI008": ["flake8_pyi/PYI008.py", "flake8_pyi/PYI008.pyi"],
    "PYI009": ["flake8_pyi/PYI009.py", "flake8_pyi/PYI009.pyi"],
    "PYI010": ["flake8_pyi/PYI010.py", "flake8_pyi/PYI010.pyi"],
    "PYI011": ["flake8_pyi/PYI011.py", "flake8_pyi/PYI011.pyi"],
    "PYI012": ["flake8_pyi/PYI012.py", "flake8_pyi/PYI012.pyi"],
    "PYI013": ["flake8_pyi/PYI013.py", "flake8_pyi/PYI013.pyi"],
    "PYI014": ["flake8_pyi/PYI014.py", "flake8_pyi/PYI014.pyi"],
    "PYI015": ["flake8_pyi/PYI015.py", "flake8_pyi/PYI015.pyi"],
    "PYI016": ["flake8_pyi/PYI016.py", "flake8_pyi/PYI016.pyi"],
    "PYI017": ["flake8_pyi/PYI017.py", "flake8_pyi/PYI017.pyi"],
    "PYI018": ["flake8_pyi/PYI018.py", "flake8_pyi/PYI018.pyi"],
    "PYI019": ["flake8_pyi/PYI019_0.py", "flake8_pyi/PYI019_0.pyi", "flake8_pyi/PYI019_1.pyi"],
    "PYI020": ["flake8_pyi/PYI020.py", "flake8_pyi/PYI020.pyi"],
    "PYI021": ["flake8_pyi/PYI021.py", "flake8_pyi/PYI021.pyi", "flake8_pyi/PYI021_1.pyi"],
    "PYI024": ["flake8_pyi/PYI024.py", "flake8_pyi/PYI024.pyi"],
    "PYI025": [
        "flake8_pyi/PYI025_1.py",
        "flake8_pyi/PYI025_1.pyi",
        "flake8_pyi/PYI025_2.py",
        "flake8_pyi/PYI025_2.pyi",
        "flake8_pyi/PYI025_3.py",
        "flake8_pyi/PYI025_3.pyi",
    ],
    "PYI026": ["flake8_pyi/PYI026.py", "flake8_pyi/PYI026.pyi"],
    "PYI029": ["flake8_pyi/PYI029.py", "flake8_pyi/PYI029.pyi"],
    "PYI030": ["flake8_pyi/PYI030.py", "flake8_pyi/PYI030.pyi"],
    "PYI032": ["flake8_pyi/PYI032.py", "flake8_pyi/PYI032.pyi"],
    "PYI033": ["flake8_pyi/PYI033.py", "flake8_pyi/PYI033.pyi"],
    "PYI034": ["flake8_pyi/PYI034.py", "flake8_pyi/PYI034.pyi"],
    "PYI035": ["flake8_pyi/PYI035.py", "flake8_pyi/PYI035.pyi"],
    "PYI036": ["flake8_pyi/PYI036.py", "flake8_pyi/PYI036.pyi"],
    "PYI041": [
        "flake8_pyi/PYI041_1.py",
        "flake8_pyi/PYI041_1.pyi",
        "flake8_pyi/PYI041_2.py",
        "flake8_pyi/PYI041_3.py",
        "flake8_pyi/PYI041_4.py",
    ],
    "PYI042": ["flake8_pyi/PYI042.py", "flake8_pyi/PYI042.pyi"],
    "PYI043": ["flake8_pyi/PYI043.py", "flake8_pyi/PYI043.pyi"],
    "PYI044": ["flake8_pyi/PYI044.py", "flake8_pyi/PYI044.pyi"],
    "PYI045": ["flake8_pyi/PYI045.py", "flake8_pyi/PYI045.pyi"],
    "PYI046": ["flake8_pyi/PYI046.py", "flake8_pyi/PYI046.pyi"],
    "PYI047": ["flake8_pyi/PYI047.py", "flake8_pyi/PYI047.pyi"],
    "PYI048": ["flake8_pyi/PYI048.py", "flake8_pyi/PYI048.pyi"],
    "PYI049": ["flake8_pyi/PYI049.py", "flake8_pyi/PYI049.pyi"],
    "PYI050": ["flake8_pyi/PYI050.py", "flake8_pyi/PYI050.pyi"],
    "PYI051": ["flake8_pyi/PYI051.py", "flake8_pyi/PYI051.pyi"],
    "PYI052": ["flake8_pyi/PYI052.py", "flake8_pyi/PYI052.pyi"],
    "PYI053": ["flake8_pyi/PYI053.py", "flake8_pyi/PYI053.pyi"],
    "PYI054": ["flake8_pyi/PYI054.py", "flake8_pyi/PYI054.pyi"],
    "PYI055": ["flake8_pyi/PYI055.py", "flake8_pyi/PYI055.pyi"],
    "PYI056": ["flake8_pyi/PYI056.py", "flake8_pyi/PYI056.pyi"],
    "PYI057": ["flake8_pyi/PYI057.py", "flake8_pyi/PYI057.pyi"],
    "PYI058": ["flake8_pyi/PYI058.py", "flake8_pyi/PYI058.pyi"],
    "PYI059": ["flake8_pyi/PYI059.py", "flake8_pyi/PYI059.pyi"],
    "PYI061": ["flake8_pyi/PYI061.py", "flake8_pyi/PYI061.pyi"],
    "PYI062": ["flake8_pyi/PYI062.py", "flake8_pyi/PYI062.pyi"],
    "PYI063": ["flake8_pyi/PYI063.py", "flake8_pyi/PYI063.pyi"],
    "PYI064": ["flake8_pyi/PYI064.py", "flake8_pyi/PYI064.pyi"],
    "PYI066": ["flake8_pyi/PYI066.py", "flake8_pyi/PYI066.pyi"],
    "RET501": ["flake8_return/RET501.py"],
    "RET502": ["flake8_return/RET502.py"],
    "RET503": ["flake8_return/RET503.py"],
    "RET504": ["flake8_return/RET504.py"],
    "RET505": ["flake8_return/RET505.py"],
    "RET506": ["flake8_return/RET506.py"],
    "RET507": ["flake8_return/RET507.py"],
    "RET508": ["flake8_return/RET508.py"],
    "RSE102": ["flake8_raise/RSE102.py"],
    "RUF001": ["ruff/confusables.py"],
    "RUF002": ["ruff/confusables.py"],
    "RUF003": ["ruff/confusables.py"],
    "RUF005": ["ruff/RUF005.py", "ruff/RUF005_slices.py"],
    "RUF006": ["ruff/RUF006.py"],
    "RUF007": ["ruff/RUF007.py"],
    "RUF008": ["ruff/RUF008.py", "ruff/RUF008_attrs.py", "ruff/RUF008_deferred.py"],
    "RUF009": [
        "ruff/RUF009.py",
        "ruff/RUF009_attrs.py",
        "ruff/RUF009_attrs_auto_attribs.py",
        "ruff/RUF009_deferred.py",
    ],
    "RUF010": ["ruff/RUF010.py"],
    "RUF012": ["ruff/RUF012.py", "ruff/RUF012_deferred.py"],
    "RUF013": ["ruff/RUF013_0.py", "ruff/RUF013_1.py", "ruff/RUF013_2.py", "ruff/RUF013_3.py", "ruff/RUF013_4.py"],
    "RUF015": ["ruff/RUF015.py"],
    "RUF016": ["ruff/RUF016.py"],
    "RUF017": ["ruff/RUF017_0.py", "ruff/RUF017_1.py"],
    "RUF018": ["ruff/RUF018.py"],
    "RUF019": ["ruff/RUF019.py"],
    "RUF020": ["ruff/RUF020.py"],
    "RUF021": ["ruff/RUF021.py"],
    "RUF022": ["ruff/RUF022.py"],
    "RUF023": ["ruff/RUF023.py"],
    "RUF024": ["ruff/RUF024.py"],
    "RUF026": ["ruff/RUF026.py"],
    "RUF028": ["ruff/RUF028.py"],
    "RUF030": ["ruff/RUF030.py"],
    "RUF032": ["ruff/RUF032.py"],
    "RUF033": ["ruff/RUF033.py"],
    "RUF034": ["ruff/RUF034.py"],
    "RUF036": ["ruff/RUF036.py", "ruff/RUF036.pyi"],
    "RUF037": ["ruff/RUF037.py"],
    "RUF040": ["ruff/RUF040.py"],
    "RUF041": ["ruff/RUF041.py", "ruff/RUF041.pyi"],
    "RUF043": ["ruff/RUF043.py"],
    "RUF046": ["ruff/RUF046.py", "ruff/RUF046_CR.py", "ruff/RUF046_LF.py"],
    "RUF048": ["ruff/RUF048.py", "ruff/RUF048_1.py"],
    "RUF049": ["ruff/RUF049.py"],
    "RUF051": ["ruff/RUF051.py"],
    "RUF053": ["ruff/RUF053.py"],
    "RUF057": ["ruff/RUF057.py"],
    "RUF058": ["ruff/RUF058_0.py", "ruff/RUF058_1.py"],
    "RUF059": ["ruff/RUF059_0.py", "ruff/RUF059_1.py", "ruff/RUF059_2.py", "ruff/RUF059_3.py"],
    "RUF060": ["ruff/RUF060.py"],
    "RUF061": ["ruff/RUF061_deprecated_call.py", "ruff/RUF061_raises.py", "ruff/RUF061_warns.py"],
    "RUF063": ["ruff/RUF063.py"],
    "RUF064": ["ruff/RUF064.py"],
    "RUF068": ["ruff/RUF068.py"],
    "SIM101": ["flake8_simplify/SIM101.py"],
    "SIM102": ["flake8_simplify/SIM102.py"],
    "SIM103": ["flake8_simplify/SIM103.py"],
    "SIM105": [
        "flake8_simplify/SIM105_0.py",
        "flake8_simplify/SIM105_1.py",
        "flake8_simplify/SIM105_2.py",
        "flake8_simplify/SIM105_3.py",
        "flake8_simplify/SIM105_4.py",
        "ruff/RUF072_RUF047_SIM105.py",
        "ruff/RUF072_SIM105.py",
    ],
    "SIM107": ["flake8_simplify/SIM107.py"],
    "SIM108": ["flake8_simplify/SIM108.py"],
    "SIM109": ["flake8_simplify/SIM109.py"],
    "SIM110": ["flake8_simplify/SIM110.py", "flake8_simplify/SIM111.py"],
    "SIM112": ["flake8_simplify/SIM112.py"],
    "SIM113": ["flake8_simplify/SIM113.py"],
    "SIM114": ["flake8_simplify/SIM114.py"],
    "SIM115": ["flake8_simplify/SIM115.py"],
    "SIM116": ["flake8_simplify/SIM116.py"],
    "SIM117": ["flake8_simplify/SIM117.py"],
    "SIM118": ["flake8_simplify/SIM118.py"],
    "SIM201": ["flake8_simplify/SIM201.py"],
    "SIM202": ["flake8_simplify/SIM202.py"],
    "SIM208": ["flake8_simplify/SIM208.py"],
    "SIM210": ["flake8_simplify/SIM210.py"],
    "SIM211": ["flake8_simplify/SIM211.py"],
    "SIM212": ["flake8_simplify/SIM212.py"],
    "SIM220": ["flake8_simplify/SIM220.py"],
    "SIM221": ["flake8_simplify/SIM221.py"],
    "SIM222": ["flake8_simplify/SIM222.py"],
    "SIM223": ["flake8_simplify/SIM223.py"],
    "SIM300": ["flake8_simplify/SIM300.py"],
    "SIM401": ["flake8_simplify/SIM401.py"],
    "SIM905": ["flake8_simplify/SIM905.py"],
    "SIM910": ["flake8_simplify/SIM910.py"],
    "SIM911": ["flake8_simplify/SIM911.py"],
    "SLF001": ["flake8_self/SLF001.py", "flake8_self/SLF001_1.py", "flake8_self/SLF001_2.py"],
    "SLOT000": ["flake8_slots/SLOT000.py"],
    "SLOT001": ["flake8_slots/SLOT001.py"],
    "SLOT002": ["flake8_slots/SLOT002.py"],
    "T100": ["flake8_debugger/T100.py"],
    "T201": ["flake8_print/T201.py"],
    "T203": ["flake8_print/T203.py"],
    "TC001": ["flake8_type_checking/TC001.py"],
    "TC002": [
        "flake8_type_checking/TC002.py",
        "flake8_type_checking/quote.py",
        "flake8_type_checking/singledispatch.py",
        "flake8_type_checking/strict.py",
        "flake8_type_checking/typing_modules_1.py",
        "flake8_type_checking/typing_modules_2.py",
    ],
    "TC003": [
        "flake8_type_checking/TC003.py",
        "flake8_type_checking/init_var.py",
        "flake8_type_checking/kw_only.py",
        "flake8_type_checking/singledispatchmethod.py",
        "flake8_type_checking/snapshot.py",
    ],
    "TC004": [
        "flake8_type_checking/TC004_1.py",
        "flake8_type_checking/TC004_10.py",
        "flake8_type_checking/TC004_11.py",
        "flake8_type_checking/TC004_12.py",
        "flake8_type_checking/TC004_13.py",
        "flake8_type_checking/TC004_14.pyi",
        "flake8_type_checking/TC004_15.py",
        "flake8_type_checking/TC004_16.py",
        "flake8_type_checking/TC004_17.py",
        "flake8_type_checking/TC004_2.py",
        "flake8_type_checking/TC004_3.py",
        "flake8_type_checking/TC004_4.py",
        "flake8_type_checking/TC004_5.py",
        "flake8_type_checking/TC004_6.py",
        "flake8_type_checking/TC004_7.py",
        "flake8_type_checking/TC004_8.py",
        "flake8_type_checking/TC004_9.py",
        "flake8_type_checking/quote.py",
        "flake8_type_checking/whitespace.py",
    ],
    "TC005": ["flake8_type_checking/TC005.py"],
    "TC006": ["flake8_type_checking/TC006.py"],
    "TC007": [
        "flake8_type_checking/TC007.py",
        "flake8_type_checking/TC008.py",
        "flake8_type_checking/TC008_typing_execution_context.py",
    ],
    "TC010": ["flake8_type_checking/TC010_1.py", "flake8_type_checking/TC010_2.py"],
    "TD001": ["flake8_todos/TD001.py"],
    "TD002": ["flake8_todos/TD002.py"],
    "TD003": ["flake8_todos/TD003.py"],
    "TD004": ["flake8_todos/TD004.py"],
    "TD005": ["flake8_todos/TD005.py"],
    "TD006": ["flake8_todos/TD006.py"],
    "TD007": ["flake8_todos/TD007.py"],
    "TID252": ["flake8_tidy_imports/TID/my_package/sublib/api/application.py", "flake8_tidy_imports/TID252.py"],
    "TRY002": ["tryceratops/TRY002.py"],
    "TRY003": ["tryceratops/TRY003.py"],
    "TRY004": ["tryceratops/TRY004.py"],
    "TRY201": ["tryceratops/TRY201.py"],
    "TRY203": ["tryceratops/TRY203.py"],
    "TRY300": ["tryceratops/TRY300.py"],
    "TRY301": ["tryceratops/TRY301.py"],
    "TRY400": ["tryceratops/TRY400.py"],
    "TRY401": ["tryceratops/TRY401.py"],
    "UP001": ["pyupgrade/UP001.py"],
    "UP003": ["pyupgrade/UP003.py"],
    "UP004": ["pyupgrade/UP004.py"],
    "UP005": ["pyupgrade/UP005.py"],
    "UP006": ["pyupgrade/UP006_0.py", "pyupgrade/UP006_1.py", "pyupgrade/UP006_2.py", "pyupgrade/UP006_3.py"],
    "UP007": ["pyupgrade/UP007_1.py", "ruff/redirects.py"],
    "UP008": ["pyupgrade/UP008.py"],
    "UP009": [
        "pyupgrade/UP009_0.py",
        "pyupgrade/UP009_1.py",
        "pyupgrade/UP009_10.py",
        "pyupgrade/UP009_2.py",
        "pyupgrade/UP009_3.py",
        "pyupgrade/UP009_4.py",
        "pyupgrade/UP009_5.py",
        "pyupgrade/UP009_6.py",
        "pyupgrade/UP009_7.py",
        "pyupgrade/UP009_8.py",
        "pyupgrade/UP009_9.py",
        "pyupgrade/UP009_code_utf8_utf8.py",
        "pyupgrade/UP009_hashbang_utf8_other.py",
        "pyupgrade/UP009_many_empty_lines.py",
        "pyupgrade/UP009_other_other.py",
        "pyupgrade/UP009_other_utf8.py",
        "pyupgrade/UP009_utf8_code_other.py",
        "pyupgrade/UP009_utf8_other.py",
        "pyupgrade/UP009_utf8_utf8.py",
        "pyupgrade/UP009_utf8_utf8_other.py",
    ],
    "UP010": ["pyupgrade/UP010_0.py", "pyupgrade/UP010_1.py"],
    "UP011": ["pyupgrade/UP011.py"],
    "UP012": ["pyupgrade/UP012.py"],
    "UP013": ["pyupgrade/UP013.py"],
    "UP014": ["pyupgrade/UP014.py"],
    "UP015": ["pyupgrade/UP015.py", "pyupgrade/UP015_1.py"],
    "UP017": ["pyupgrade/UP017.py"],
    "UP018": ["pyupgrade/UP018.py", "pyupgrade/UP018_CR.py", "pyupgrade/UP018_LF.py"],
    "UP019": ["pyupgrade/UP019.py"],
    "UP020": ["pyupgrade/UP020.py"],
    "UP021": ["pyupgrade/UP021.py"],
    "UP022": ["pyupgrade/UP022.py"],
    "UP023": ["pyupgrade/UP023.py"],
    "UP024": [
        "pyupgrade/UP024_0.py",
        "pyupgrade/UP024_1.py",
        "pyupgrade/UP024_2.py",
        "pyupgrade/UP024_3.py",
        "pyupgrade/UP024_4.py",
    ],
    "UP025": ["pyupgrade/UP025.py"],
    "UP026": ["pyupgrade/UP026.py"],
    "UP028": ["pyupgrade/UP028_0.py", "pyupgrade/UP028_1.py"],
    "UP029": ["pyupgrade/UP029_0.py", "pyupgrade/UP029_2.py", "pyupgrade/UP029_3.py"],
    "UP030": ["pyupgrade/UP030_0.py", "pyupgrade/UP030_1.py"],
    "UP031": ["pyupgrade/UP031_0.py", "pyupgrade/UP031_1.py"],
    "UP032": ["pyupgrade/UP032_0.py", "pyupgrade/UP032_1.py", "pyupgrade/UP032_2.py", "pyupgrade/UP032_3.py"],
    "UP033": ["pyupgrade/UP033_0.py", "pyupgrade/UP033_1.py"],
    "UP034": ["pyupgrade/UP034.py"],
    "UP035": ["pyupgrade/UP035.py"],
    "UP036": [
        "pyupgrade/UP036_0.py",
        "pyupgrade/UP036_1.py",
        "pyupgrade/UP036_2.py",
        "pyupgrade/UP036_3.py",
        "pyupgrade/UP036_4.py",
        "pyupgrade/UP036_5.py",
    ],
    "UP037": ["pyupgrade/UP037_0.py", "pyupgrade/UP037_1.py", "pyupgrade/UP037_2.pyi", "pyupgrade/UP037_3.py"],
    "UP039": ["pyupgrade/UP039.py"],
    "UP040": ["pyupgrade/UP040.py", "pyupgrade/UP040.pyi"],
    "UP041": ["pyupgrade/UP041.py"],
    "UP042": ["pyupgrade/UP042.py"],
    "UP043": ["pyupgrade/UP043.py"],
    "UP044": ["pyupgrade/UP044.py"],
    "UP045": ["pyupgrade/UP045_1.py"],
    "UP046": ["pyupgrade/UP046_0.py", "pyupgrade/UP046_1.py"],
    "UP047": ["pyupgrade/UP047_0.py"],
    "UP049": ["pyupgrade/UP049_0.py", "pyupgrade/UP049_1.py"],
    "UP050": ["pyupgrade/UP050.py"],
    "W605": ["pycodestyle/W605_0.py", "pycodestyle/W605_1.py"],
    "YTT101": ["flake8_2020/YTT101.py"],
    "YTT102": ["flake8_2020/YTT102.py"],
    "YTT103": ["flake8_2020/YTT103.py"],
    "YTT201": ["flake8_2020/YTT201.py"],
    "YTT202": ["flake8_2020/YTT202.py"],
    "YTT203": ["flake8_2020/YTT203.py"],
    "YTT204": ["flake8_2020/YTT204.py"],
    "YTT301": ["flake8_2020/YTT301.py"],
    "YTT302": ["flake8_2020/YTT302.py"],
    "YTT303": ["flake8_2020/YTT303.py"],
}


# Ours: one case per shipped Ruff rule that Ruff reports, `{c}` where the reported line ends (`{{`, `}}` are braces).
# fixtures/rerail/noqa/<CODE>.py holds it once without a comment and once after each of NOQA_COMMENTS, so check 4 fails
# when a port stops honouring `# noqa` as Ruff does.
NOQA = {
    "ASYNC100": "import asyncio\nasync def f():\n    async with asyncio.timeout(1):{c}\n        pass\n",
    "ASYNC105": "import trio\nasync def f():\n    trio.sleep(0){c}\n",
    "ASYNC109": "import trio\nasync def f(timeout):{c}\n    pass\n",
    "ASYNC110": "import asyncio\nasync def f(ready):\n    while not ready:{c}\n        await asyncio.sleep(1)\n",
    "ASYNC115": "import trio\nasync def f():\n    await trio.sleep(0){c}\n",
    "ASYNC116": "import trio\nasync def f():\n    await trio.sleep(100000){c}\n",
    "ASYNC210": "import requests\nasync def f():\n    requests.get('https://example.com'){c}\n",
    "ASYNC212": "import httpx\nasync def f():\n    client = httpx.Client()\n    client.get('https://example.com'){c}\n",
    "ASYNC220": "import os\nasync def f():\n    os.popen('ls'){c}\n",
    "ASYNC221": "import subprocess\nasync def f():\n    subprocess.run(['ls']){c}\n",
    "ASYNC222": "import os\nasync def f():\n    os.wait(){c}\n",
    "ASYNC230": "async def f():\n    open('data.txt'){c}\n",
    "ASYNC240": "import os.path\nasync def f():\n    os.path.exists('data.txt'){c}\n",
    "ASYNC250": "async def f():\n    input(){c}\n",
    "ASYNC251": "import time\nasync def f():\n    time.sleep(1){c}\n",
    "B002": "n = 1\n++n{c}\n",
    "B003": "import os\nos.environ = {}{c}\n",
    "B004": "obj = 1\nhasattr(obj, '__call__'){c}\n",
    "B005": "s = 'abc'\ns.strip('.example.com'){c}\n",
    "B007": "for i in range(3):{c}\n    pass\n",
    "B009": "obj = 1\ngetattr(obj, 'name'){c}\n",
    "B010": "obj = 1\nsetattr(obj, 'name', 1){c}\n",
    "B011": "assert False{c}\n",
    "B013": "try:\n    pass\nexcept (ValueError,):{c}\n    pass\n",
    "B014": "try:\n    pass\nexcept (ValueError, ValueError):{c}\n    pass\n",
    "B016": "raise 'error'{c}\n",
    "B017": "import unittest\nclass T(unittest.TestCase):\n    def test(self):\n"
            "        with self.assertRaises(Exception):{c}\n            pass\n",
    "B019": "import functools\nclass C:\n    @functools.lru_cache{c}\n    def f(self):\n        pass\n",
    "B020": "items = [1]\nfor items in items:{c}\n    pass\n",
    "B021": "def f():\n    f'''doc {{1}}'''{c}\n",
    "B022": "import contextlib\nwith contextlib.suppress():{c}\n    pass\n",
    "B023": "fs = []\nfor x in range(3):\n    fs.append(lambda: x){c}\n",
    "B025": "try:\n    pass\nexcept ValueError:\n    pass\nexcept ValueError:{c}\n    pass\n",
    "B026": "def f(*args, **kwargs):\n    pass\nf(a=1, *[2]){c}\n",
    "B028": "import warnings\nwarnings.warn('message'){c}\n",
    "B029": "try:\n    pass\nexcept ():{c}\n    pass\n",
    "B030": "try:\n    pass\nexcept 1:{c}\n    pass\n",
    "B032": "d = {}\nd['key']: int{c}\n",
    "B033": "s = {1, 1}{c}\n",
    "B034": "import re\nre.sub('a', 'b', 'aaa', 1){c}\n",
    "B039": "from contextvars import ContextVar\nv = ContextVar('v', default=[]){c}\n",
    "B904": "try:\n    pass\nexcept ValueError:\n    raise RuntimeError('x'){c}\n",
    "B905": "zip([1], [2]){c}\n",
    "E711": "x = 1\nif x == None:{c}\n    pass\n",
    "E712": "x = 1\nif x == True:{c}\n    pass\n",
    "E713": "x, y = 1, [1]\nif not x in y:{c}\n    pass\n",
    "E714": "x, y = 1, 2\nif not x is y:{c}\n    pass\n",
    "E721": "x = 1\nif type(x) == type(1):{c}\n    pass\n",
    "E722": "try:\n    pass\nexcept:{c}\n    pass\n",
    "E742": "class I:{c}\n    pass\n",
    "E743": "def l():{c}\n    pass\n",
    "F403": "from os import *{c}\n",
    "F404": "import os\nfrom __future__ import annotations{c}\n",
    "F405": "from os import *\npath{c}\n",
    "F406": "def f():\n    from os import *{c}\n",
    "F407": "from __future__ import unknown_feature{c}\n",
    "F501": "'%(a)' % {{'a': 1}}{c}\n",
    "F502": "'%(a)s' % (1,){c}\n",
    "F503": "'%s %s' % {{'a': 1}}{c}\n",
    "F504": "'%(a)s' % {{'a': 1, 'b': 2}}{c}\n",
    "F505": "'%(a)s %(b)s' % {{'a': 1}}{c}\n",
    "F506": "'%(a)s %s' % (1,){c}\n",
    "F507": "'%s %s' % (1,){c}\n",
    "F508": "'%(a)*d' % {{'a': 1}}{c}\n",
    "F509": "'%y' % (1,){c}\n",
    "F521": "'{{'.format(1){c}\n",
    "F522": "'{{a}}'.format(a=1, b=2){c}\n",
    "F523": "'{{0}}'.format(1, 2){c}\n",
    "F524": "'{{0}} {{1}}'.format(1){c}\n",
    "F525": "'{{}} {{0}}'.format(1, 2){c}\n",
    "F541": "f'abc'{c}\n",
    "F601": "d = {{'a': 1, 'a': 2}}{c}\n",
    "F602": "a = 'k'\nd = {{a: 1, a: 2}}{c}\n",
    "F622": "*a, *b = [1, 2]{c}\n",
    "F631": "assert (False, 'message'){c}\n",
    "F632": "x = 1\nif x is 'abc':{c}\n    pass\n",
    "F633": "import sys\nprint >> sys.stderr{c}\n",
    "F634": "if (1, 2):{c}\n    pass\n",
    "F701": "break{c}\n",
    "F702": "continue{c}\n",
    "F706": "return 1{c}\n",
    "F707": "try:\n    pass\nexcept:{c}\n    pass\nexcept ValueError:\n    pass\n",
    "F722": "x: 'List[int' = []{c}\n",
    "F822": "__all__ = ['missing']{c}\n",
    "F823": "x = 1\ndef f():\n    print(x){c}\n    x = 2\n",
    "F842": "def f():\n    x: int{c}\n",
    "F901": "def f():\n    raise NotImplemented{c}\n",
    "PIE790": "def f():\n    '''Doc.'''\n    pass{c}\n",
    "PIE794": "class C:\n    a = 1\n    a = 2{c}\n",
    "PIE796": "import enum\nclass E(enum.Enum):\n    A = 1\n    B = 1{c}\n",
    "PIE800": "d = {{**{{'a': 1}}}}{c}\n",
    "PIE804": "def f(**kwargs):\n    pass\nf(**{{'a': 1}}){c}\n",
    "PIE807": "from dataclasses import dataclass, field\n@dataclass\nclass C:\n"
              "    items: list = field(default_factory=lambda: []){c}\n",
    "PIE808": "range(0, 10){c}\n",
    "PIE810": "s = 'abc'\ns.startswith('a') or s.startswith('b'){c}\n",
    "RET501": "def f():\n    print()\n    return None{c}\n",
    "RET502": "def f(x):\n    if x:\n        return{c}\n    return 1\n",
    "RET504": "def f():\n    x = 1\n    return x{c}\n",
    "RET505": "def f(x):\n    if x:\n        return 1\n    else:{c}\n        return 2\n",
    "RET506": "def f(x):\n    if x:\n        raise ValueError\n    else:{c}\n        return 2\n",
    "RET507": "def f(xs):\n    for x in xs:\n        if x:\n            continue\n        else:{c}\n"
              "            print(x)\n",
    "RET508": "def f(xs):\n    for x in xs:\n        if x:\n            break\n        else:{c}\n"
              "            print(x)\n",
    "SIM101": "x = 1\nisinstance(x, int) or isinstance(x, float){c}\n",
    "SIM107": "def f():\n    try:\n        return 1\n    finally:\n        return 2{c}\n",
    "SIM109": "a, b, c = 1, 2, 3\nif a == b or a == c:{c}\n    pass\n",
    "SIM112": "import os\nos.environ['foo']{c}\n",
    "SIM113": "items = [1]\nidx = 0\nfor item in items:\n    print(item)\n    idx += 1{c}\n",
    "SIM115": "f = open('data.txt'){c}\nf.close()\n",
    "SIM116": "def f(x):\n    if x == 'a':{c}\n        return 1\n    elif x == 'b':\n        return 2\n"
              "    elif x == 'c':\n        return 3\n",
    "SIM201": "a, b = 1, 2\nif not a == b:{c}\n    pass\n",
    "SIM202": "a, b = 1, 2\nif not a != b:{c}\n    pass\n",
    "SIM208": "a = 1\nif not (not a):{c}\n    pass\n",
    "SIM210": "a = 1\nb = True if a else False{c}\n",
    "SIM211": "a = 1\nb = False if a else True{c}\n",
    "SIM212": "a, b = 1, 2\nc = b if not a else a{c}\n",
    "SIM220": "a = 1\nif a and not a:{c}\n    pass\n",
    "SIM221": "a = 1\nif a or not a:{c}\n    pass\n",
    "SIM300": "x = 1\nif 'a' == x:{c}\n    pass\n",
    "SIM905": "'a,b,c'.split(','){c}\n",
    "SIM910": "d = {{}}\nd.get('key', None){c}\n",
    "T100": "breakpoint(){c}\n",
    # The rest of the shipped ports, one case each.
    "AIR001": "from airflow.operators.python import PythonOperator\n\n"
              "my_task = PythonOperator(task_id=\"my_other_task\", python_callable=print){c}\n",
    "AIR002": "from airflow import DAG\n\nDAG(dag_id=\"x\"){c}\n",
    "AIR302": "from airflow.auth.managers.fab.fab_auth_manager import FabAuthManager\n\n"
              "FabAuthManager().get_fastapi_app(){c}\n",
    "AIR303": "from airflow.lineage.hook import HookLineageCollector\n\n"
              "HookLineageCollector().create_asset(\"s3://bucket/key\"){c}\n",
    "AIR311": "from airflow import Dataset\n\nDataset(uri=\"test://test/\"){c}\n",
    "AIR312": "from airflow.operators.python import PythonOperator\n\n"
              "PythonOperator(task_id=\"t\", python_callable=print){c}\n",
    "ANN001": "def f(x):{c}\n    pass\n",
    "ANN002": "def f(*args):{c}\n    pass\n",
    "ANN003": "def f(**kwargs):{c}\n    pass\n",
    "ANN201": "def add(a: int, b: int):{c}\n    return a + b\n",
    "ANN202": "def _add(a: int, b: int):{c}\n    return a + b\n",
    "ANN204": "class C:\n    def __init__(self):{c}\n        pass\n",
    "ANN205": "class C:\n    @staticmethod\n    def f():{c}\n        pass\n",
    "ANN206": "class C:\n    @classmethod\n    def f(cls):{c}\n        pass\n",
    "ANN401": "from typing import Any\n\n\ndef f(x: Any) -> None:{c}\n    pass\n",
    "ARG003": "class C:\n    @classmethod\n    def f(cls, x):{c}\n        return 1\n",
    "ARG004": "class C:\n    @staticmethod\n    def f(x):{c}\n        return 1\n",
    "ARG005": "f = lambda x: 1{c}\n",
    "C400": "list(x for x in range(3)){c}\n",
    "C401": "set(x for x in range(3)){c}\n",
    "C402": "dict((x, x) for x in range(3)){c}\n",
    "C403": "set([x for x in range(3)]){c}\n",
    "C404": "dict([(x, x) for x in range(3)]){c}\n",
    "C405": "set([1, 2]){c}\n",
    "C406": "dict([(1, 2)]){c}\n",
    "C408": "x = dict(){c}\n",
    "C409": "x = tuple([1, 2]){c}\n",
    "C410": "x = list([1, 2]){c}\n",
    "C411": "x = list([i + 1 for i in range(3)]){c}\n",
    "C413": "x = list(sorted([2, 1])){c}\n",
    "C414": "x = sorted(list([2, 1])){c}\n",
    "C415": "x = set([1, 2][::-1]){c}\n",
    "C416": "x = [i for i in range(3)]{c}\n",
    "C417": "x = list(map(lambda i: i * 2, range(3))){c}\n",
    "C419": "x = any([i > 1 for i in range(3)]){c}\n",
    "C420": "x = {{k: 1 for k in range(3)}}{c}\n",
    "COM818": "x = 1,{c}\n",
    "D101": "class A: ...{c}\n",
    "D102": "class A:\n    def f(self): ...{c}\n",
    "D103": "def f(): ...{c}\n",
    "D105": "class A:\n    def __str__(self): ...{c}\n",
    "D106": "class A:\n    \"\"\"Doc.\"\"\"\n\n    class B: ...{c}\n",
    "D107": "class A:\n    \"\"\"Doc.\"\"\"\n\n    def __init__(self): ...{c}\n",
    "D200": "def f():\n    \"\"\"\n    Only line.\n    \"\"\"{c}\n",
    "D205": "def f():\n    \"\"\"Summary.\n    Description.\n    \"\"\"{c}\n",
    "D209": "def f():\n    \"\"\"Summary.\n\n    Description.\"\"\"{c}\n",
    "D212": "def f():\n    \"\"\"\n    Summary.\n\n    Description.\n    \"\"\"{c}\n",
    "D300": "def f():\n    '''Doc.'''{c}\n",
    "D301": "def f():\n    \"\"\"Doc \\n.\"\"\"{c}\n",
    "D402": "def f(a):\n    \"\"\"f(a) -> int.\"\"\"{c}\n",
    "D403": "def f():\n    \"\"\"do it.\"\"\"{c}\n",
    "D404": "def f():\n    \"\"\"This does it.\"\"\"{c}\n",
    "D418": "from typing import overload\n\n\n@overload\ndef f(a: int) -> int:{c}\n    \"\"\"Doc.\"\"\"\n",
    "D419": "def f():\n    \"\"\"\"\"\"{c}\n",
    "DTZ001": "import datetime\ndatetime.datetime(2000, 1, 1){c}\n",
    "DTZ002": "import datetime\ndatetime.datetime.today(){c}\n",
    "DTZ003": "import datetime\ndatetime.datetime.utcnow(){c}\n",
    "DTZ004": "import datetime\ndatetime.datetime.utcfromtimestamp(1234){c}\n",
    "DTZ005": "import datetime\ndatetime.datetime.now(){c}\n",
    "DTZ006": "import datetime\ndatetime.datetime.fromtimestamp(1234){c}\n",
    "DTZ011": "import datetime\ndatetime.date.today(){c}\n",
    "DTZ012": "import datetime\ndatetime.date.fromtimestamp(1234){c}\n",
    "DTZ901": "import datetime\ndatetime.datetime.min{c}\n",
    "E401": "import os, sys{c}\n",
    "EM101": "raise RuntimeError(\"msg\"){c}\n",
    "EM102": "x = 1\nraise RuntimeError(f\"{{x}}\"){c}\n",
    "EM103": "raise RuntimeError(\"{{}}\".format(1)){c}\n",
    "FAST002": "from fastapi import FastAPI, Query\napp = FastAPI()\n\n@app.get(\"/\")\n"
               "def f(a: int = Query(1)): ...{c}\n",
    "FAST003": "from fastapi import FastAPI\napp = FastAPI()\n\n@app.get(\"/things/{{thing_id}}\"){c}\ndef f(): ...\n",
    "FBT001": "def f(flag: bool): ...{c}\n",
    "FBT002": "def f(flag=True): ...{c}\n",
    "FBT003": "def g(x): ...\n\ng(True){c}\n",
    "FIX001": "# FIXME: fix this{c}\n",
    "FIX002": "# TODO: fix this{c}\n",
    "FIX003": "# XXX: fix this{c}\n",
    "FIX004": "# HACK: fix this{c}\n",
    "FLY002": "\", \".join([\"a\", \"b\"]){c}\n",
    "G001": "import logging\nlogging.info(\"Hello {{}}\".format(\"World!\")){c}\n",
    "G002": "import logging\nlogging.info(\"Hello %s\" % \"World!\"){c}\n",
    "G003": "import logging\nlogging.info(\"Hello \" + \"World!\"){c}\n",
    "G004": "import logging\nname = \"World\"\nlogging.info(f\"Hello {{name}}\"){c}\n",
    "G010": "import logging\nlogging.warn(\"Something happened\"){c}\n",
    "G101": "import logging\nlogging.info(\"Hello\", extra={{\"name\": \"World\"}}){c}\n",
    "G201": "import logging\ntry:\n    pass\nexcept:\n    logging.error(\"Exception occurred\", exc_info=True){c}\n",
    "G202": "import logging\ntry:\n    pass\nexcept:\n    logging.exception(\"Exception occurred\", exc_info=True){c}\n",
    "ICN001": "import numpy{c}\n",
    "INT001": "from gettext import gettext as _\nname = \"x\"\n_(f\"hello {{name}}\"){c}\n",
    "INT002": "from gettext import gettext as _\n_(\"hello {{}}\".format(\"x\")){c}\n",
    "INT003": "from gettext import gettext as _\n_(\"hello %s\" % \"x\"){c}\n",
    "ISC001": "x = \"a\" \"b\"{c}\n",
    "ISC003": "x = (\n    \"a\"{c}\n    + \"b\"\n)\n",
    "ISC004": "x = [\"a\" \"b\", \"c\"]{c}\n",
    "LOG001": "import logging\nlogger = logging.Logger(__name__){c}\n",
    "LOG002": "import logging\nlogger = logging.getLogger(__file__){c}\n",
    "LOG004": "import logging\nlogging.exception(\"x\"){c}\n",
    "LOG007": "import logging\nlogger = logging.getLogger(__name__)\nlogger.exception(\"x\", exc_info=False){c}\n",
    "LOG009": "import logging\nlogging.WARN{c}\n",
    "LOG014": "import logging\nlogger = logging.getLogger(__name__)\nlogger.info(\"x\", exc_info=True){c}\n",
    "LOG015": "import logging\nlogging.info(\"x\"){c}\n",
    "N801": "class my_class:{c}\n    pass\n",
    "N802": "def myFunction():{c}\n    pass\n",
    "N803": "def f(myArg):{c}\n    pass\n",
    "N804": "class A:\n    @classmethod\n    def f(self):{c}\n        pass\n",
    "N805": "class A:\n    def f(x):{c}\n        pass\n",
    "N806": "def f():\n    Foo = 1{c}\n",
    "N807": "def __my__():{c}\n    pass\n",
    "N811": "from m import CONSTANT as constant{c}\n",
    "N812": "from m import foo as Foo{c}\n",
    "N813": "from m import CamelCase as camelcase{c}\n",
    "N814": "from m import CamelCase as CAMELCASE{c}\n",
    "N815": "class A:\n    myVar = 1{c}\n",
    "N816": "myVar = 1{c}\n",
    "N818": "class Foo(Exception):{c}\n    pass\n",
    "NPY001": "import numpy as np\n\nnp.int{c}\n",
    "NPY002": "import numpy as np\n\nnp.random.randint(1, 5){c}\n",
    "NPY003": "import numpy as np\n\nnp.alltrue([]){c}\n",
    "NPY201": "import numpy as np\n\nnp.NaN{c}\n",
    "PD012": "import pandas as pd\n\npd.read_table(\"x.csv\", sep=\",\"){c}\n",
    "PERF101": "x = [1, 2]\nfor i in list(x):{c}\n    print(i)\n",
    "PERF102": "d = {{}}\nfor k, v in d.items():{c}\n    print(k)\n",
    "PERF203": "for i in range(3):\n    try:\n        int(\"x\")\n    except ValueError:{c}\n        continue\n",
    "PERF401": "def f():\n    out = []\n    for i in range(3):\n        out.append(i * 2){c}\n",
    "PERF402": "def f(x: list):\n    out = []\n    for i in x:\n        out.append(i){c}\n",
    "PERF403": "def f(d: dict):\n    out = {{}}\n    for k, v in d.items():\n        out[k] = v{c}\n",
    "PGH003": "x = 1  # type: ignore{c}\n",
    "PGH005": "from unittest import mock\n\nmock.Mock().assert_called_once{c}\n",
    "PT001": "import pytest\n\n\n@pytest.fixture(){c}\ndef a():\n    pass\n",
    "PT002": "import pytest\n\n\n@pytest.fixture(\"x\"){c}\ndef a():\n    pass\n",
    "PT003": "import pytest\n\n\n@pytest.fixture(scope=\"function\"){c}\ndef my_fixture(): ...\n",
    "PT006": "import pytest\n\n\n@pytest.mark.parametrize(\"a, b\", [(1, 2)]){c}\ndef test_x(a, b): ...\n",
    "PT007": "import pytest\n\n\n@pytest.mark.parametrize(\"a\", (1, 2)){c}\ndef test_x(a): ...\n",
    "PT008": "import os\n\n\ndef test_x(mocker):\n    mocker.patch.object(os, \"getcwd\", lambda: \"x\"){c}\n",
    "PT009": "import unittest\n\n\nclass T(unittest.TestCase):\n    def test_x(self):\n"
             "        self.assertEqual(1, 1){c}\n",
    "PT010": "import pytest\n\nwith pytest.raises():  {c}\n    pass\n",
    "PT011": "import pytest\n\nwith pytest.raises(ValueError):  {c}\n    raise ValueError\n",
    "PT012": "import pytest\n\nwith pytest.raises(ValueError, match=\"x\"):  {c}\n    a = 1\n    raise ValueError(a)\n",
    "PT013": "from pytest import fixture{c}\n",
    "PT014": "import pytest\n\n\n@pytest.mark.parametrize(\"x\", [1, 1]){c}\ndef test_x(x): ...\n",
    "PT015": "assert None{c}\n",
    "PT016": "import pytest\n\npytest.fail(){c}\n",
    "PT017": "try:\n    pass\nexcept Exception as e:\n    assert e.args{c}\n",
    "PT018": "assert a and b{c}\n",
    "PT019": "def test_x(_fixture): ...{c}\n",
    "PT020": "import pytest\n\n\n@pytest.yield_fixture{c}\ndef f(): ...\n",
    "PT021": "import pytest\n\n\n@pytest.fixture()\ndef f(request):\n    request.addfinalizer(lambda: None){c}\n",
    "PT022": "import pytest\n\n\n@pytest.fixture()\ndef f():\n    x = 1\n    yield x{c}\n",
    "PT023": "import pytest\n\n\n@pytest.mark.foo(){c}\ndef test_x(): ...\n",
    "PT024": "import pytest\n\n\n@pytest.mark.asyncio{c}\n@pytest.fixture()\ndef f(): ...\n",
    "PT025": "import pytest\n\n\n@pytest.mark.usefixtures(\"a\"){c}\n@pytest.fixture()\ndef f(): ...\n",
    "PT026": "import pytest\n\n\n@pytest.mark.usefixtures(){c}\ndef test_x(): ...\n",
    "PT027": "import unittest\n\n\nclass T(unittest.TestCase):\n    def test_x(self):\n"
             "        with self.assertRaises(ValueError):  {c}\n            raise ValueError\n",
    "PT028": "def test_x(a=1):{c}\n    pass\n",
    "PT030": "import pytest\n\nwith pytest.warns(Warning):  {c}\n    pass\n",
    "PT031": "import pytest\n\nwith pytest.warns(UserWarning):{c}\n    foo()\n    bar()\n",
    "PTH100": "import os\nos.path.abspath(\"..\"){c}\n",
    "PTH101": "import os\nos.chmod(\"file\", 0o644){c}\n",
    "PTH102": "import os\nos.mkdir(\"dir\"){c}\n",
    "PTH103": "import os\nos.makedirs(\"dir\"){c}\n",
    "PTH104": "import os\nos.rename(\"a\", \"b\"){c}\n",
    "PTH105": "import os\nos.replace(\"a\", \"b\"){c}\n",
    "PTH106": "import os\nos.rmdir(\"dir\"){c}\n",
    "PTH107": "import os\nos.remove(\"file\"){c}\n",
    "PTH108": "import os\nos.unlink(\"file\"){c}\n",
    "PTH109": "import os\nos.getcwd(){c}\n",
    "PTH110": "import os\nos.path.exists(\"file\"){c}\n",
    "PTH111": "import os\nos.path.expanduser(\"~\"){c}\n",
    "PTH112": "import os\nos.path.isdir(\"dir\"){c}\n",
    "PTH113": "import os\nos.path.isfile(\"file\"){c}\n",
    "PTH114": "import os\nos.path.islink(\"file\"){c}\n",
    "PTH115": "import os\nos.readlink(\"file\"){c}\n",
    "PTH116": "import os\nos.stat(\"file\"){c}\n",
    "PTH117": "import os\nos.path.isabs(\"file\"){c}\n",
    "PTH118": "import os\nos.path.join(\"a\", \"b\"){c}\n",
    "PTH119": "import os\nos.path.basename(\"a/b\"){c}\n",
    "PTH120": "import os\nos.path.dirname(\"a/b\"){c}\n",
    "PTH121": "import os\nos.path.samefile(\"a\", \"b\"){c}\n",
    "PTH122": "import os\nos.path.splitext(\"a.b\"){c}\n",
    "PTH123": "open(\"file\"){c}\n",
    "PTH124": "import py.path\n\np = py.path.local(\"/foo/bar\"){c}\n",
    "PTH201": "from pathlib import Path\n\np = Path(\".\"){c}\n",
    "PTH202": "import os\n\nos.path.getsize(\"file.py\"){c}\n",
    "PTH203": "import os\n\nos.path.getatime(\"file.py\"){c}\n",
    "PTH204": "import os\n\nos.path.getmtime(\"file.py\"){c}\n",
    "PTH205": "import os\n\nos.path.getctime(\"file.py\"){c}\n",
    "PTH206": "import os\n\nparts = \"a/b\".split(os.sep){c}\n",
    "PTH207": "import glob\n\nglob.glob(\"*.py\"){c}\n",
    "PTH208": "import os\n\nos.listdir(\".\"){c}\n",
    "PTH210": "from pathlib import Path\n\nPath(\"foo.bar\").with_suffix(\"baz\"){c}\n",
    "PTH211": "import os\n\nos.symlink(\"usr/bin/python\", \"tmp/python\"){c}\n",
    "PYI001": "from typing import TypeVar\nT = TypeVar(\"T\"){c}\n",
    "PYI002": "import sys\nif sys:{c}\n    x: int\n",
    "PYI003": "import sys\nif sys.version_info >= (3, \"9\"):{c}\n    x: int\n",
    "PYI004": "import sys\nif sys.version_info >= (3, 4, 3):{c}\n    x: int\n",
    "PYI005": "import sys\nif sys.version_info[:2] == (3,):{c}\n    x: int\n",
    "PYI006": "import sys\n\nif sys.version_info <= (3, 10):{c}\n    pass\n",
    "PYI007": "import sys\nif sys.platform > \"linux\":{c}\n    x: int\n",
    "PYI008": "import sys\nif sys.platform == \"linus\":{c}\n    x: int\n",
    "PYI009": "class Foo:\n    pass{c}\n",
    "PYI010": "def foo() -> int:\n    return 1{c}\n",
    "PYI011": "def foo(arg: list[int] = list(range(10_000))) -> None: ...{c}\n",
    "PYI012": "class Foo:\n    x: int\n    pass{c}\n",
    "PYI013": "class Foo:\n    ...{c}\n    value: int\n",
    "PYI014": "def foo(arg=bar()) -> None: ...{c}\n",
    "PYI015": "foo: str = bar(){c}\n",
    "PYI017": "x = y = 1{c}\n",
    "PYI018": "from typing import TypeVar\ndef f():\n    _T = TypeVar(\"_T\"){c}\n",
    "PYI019": "from typing import TypeVar\n_S = TypeVar(\"_S\", bound=\"C\")\nclass C:\n"
              "    def m(self: _S) -> _S: ...{c}\n",
    "PYI020": "def f(x: \"int\"): ...  {c}\n",
    "PYI021": "class A:\n    \"\"\"Doc.\"\"\"  {c}\n",
    "PYI024": "from collections import namedtuple\nX = namedtuple(\"X\", [\"a\"]){c}\n",
    "PYI025": "from collections.abc import Set{c}\n",
    "PYI026": "Vector = list[float]  {c}\n",
    "PYI029": "class A:\n    def __str__(self) -> str: ...  {c}\n",
    "PYI030": "from typing import Literal\ndef f(x: Literal[1] | Literal[2]): ...{c}\n",
    "PYI032": "from typing import Any\nclass C:\n    def __eq__(self, other: Any) -> bool: ...{c}\n",
    "PYI033": "x = 1  # type: int {c}\n",
    "PYI034": "class C:\n    def __new__(cls) -> C: ...{c}\n",
    "PYI035": "__all__: list[str]  {c}\n",
    "PYI036": "class C:\n    def __exit__(self, *args: int) -> None: ...{c}\n",
    "PYI041": "def f(x: int | float): ...{c}\n",
    "PYI042": "from typing import TypeAlias\nmy_alias: TypeAlias = int{c}\n",
    "PYI043": "from typing import TypeAlias\n_MyT: TypeAlias = int{c}\n",
    "PYI044": "from __future__ import annotations  {c}\n",
    "PYI045": "from typing import Iterable\nclass C:\n    def __iter__(self) -> Iterable[int]: ...{c}\n",
    "PYI046": "from typing import Protocol\ndef f():\n    class _P(Protocol): ...{c}\n",
    "PYI047": "from typing import TypeAlias\ndef f():\n    _A: TypeAlias = int{c}\n",
    "PYI048": "def f():  {c}\n    x = 1\n    y = 2\n",
    "PYI049": "from typing import TypedDict\ndef f():\n    class _T(TypedDict):{c}\n        a: int\n",
    "PYI050": "from typing import NoReturn\n\ndef f(x: NoReturn): ...  {c}\n",
    "PYI051": "from typing import Literal\n\nx: Literal[\"a\"] | str  {c}\n",
    "PYI052": "x = 1  {c}\n",
    "PYI054": "x: int = 12345678901  {c}\n",
    "PYI056": "__all__ = [\"a\"]\n__all__.append(\"b\")  {c}\n",
    "PYI057": "from typing import ByteString  {c}\n",
    "PYI058": "from typing import Iterator, Generator\n\nclass C:\n"
              "    def __iter__(self) -> Generator[int, None, None]:  {c}\n        yield 1\n",
    "PYI059": "from typing import Generic, TypeVar\nT = TypeVar(\"T\")\n\nclass C(Generic[T], list):  {c}\n    ...\n",
    "PYI061": "from typing import Literal\n\ndef f(x: Literal[None]): ...  {c}\n",
    "PYI062": "from typing import Literal\n\nx: Literal[\"a\", \"a\"]  {c}\n",
    "PYI063": "def f(__x: int): ...  {c}\n",
    "PYI064": "from typing import Final, Literal\n\nx: Final[Literal[1]] = 1  {c}\n",
    "PYI066": "import sys\n\nif sys.version_info < (3, 9):  {c}\n    a: int\nelse:\n    a: str\n",
    "RSE102": "raise ValueError()  {c}\n",
    "RUF002": "def f():\n    \"\"\"Docstring with ambiguous ‘ quote.\"\"\"  {c}\n",
    "RUF003": "x = 1  # ambiguous ‘ quote {c}\n",
    "RUF005": "foo = [1]\nbar = foo + [2]  {c}\n",
    "RUF006": "import asyncio\n\nasync def f():\n    asyncio.create_task(asyncio.sleep(1))  {c}\n",
    "RUF007": "from itertools import starmap\nx = [1, 2]\ny = zip(x, x[1:])  {c}\n",
    "RUF008": "from dataclasses import dataclass\n\n@dataclass\nclass A:\n    x: list[int] = []  {c}\n",
    "RUF009": "from dataclasses import dataclass\n\ndef f() -> list[int]:\n    return []\n\n@dataclass\nclass A:\n"
              "    x: list[int] = f()  {c}\n",
    "RUF010": "x = 1\ny = f\"{{str(x)}}\"  {c}\n",
    "RUF012": "class A:\n    x: list[int] = []  {c}\n",
    "RUF013": "def f(a: int = None): ...  {c}\n",
    "RUF015": "x = [1, 2]\ny = list(x)[0]  {c}\n",
    "RUF016": "x = [1, 2, 3][\"a\"]  {c}\n",
    "RUF017": "x = sum([[1], [2]], [])  {c}\n",
    "RUF018": "assert (x := 1) == 1  {c}\nprint(x)\n",
    "RUF019": "d = {{}}\nk = \"a\"\nif k in d and d[k]:  {c}\n    pass\n",
    "RUF020": "from typing import Never\ndef f(x: Never | int): ...  {c}\n",
    "RUF021": "x = a or b and c  {c}\n",
    "RUF022": "__all__ = [\"b\", \"a\"]  {c}\n",
    "RUF023": "class A:\n    __slots__ = [\"b\", \"a\"]  {c}\n",
    "RUF024": "x = dict.fromkeys([\"a\"], [])  {c}\n",
    "RUF026": "from collections import defaultdict\nx = defaultdict(default_factory=int)  {c}\n",
    "RUF030": "assert x, print(\"m\")  {c}\n",
    "RUF032": "from decimal import Decimal\nx = Decimal(0.1)  {c}\n",
    "RUF033": "from dataclasses import dataclass, InitVar\n@dataclass\nclass A:\n    a: InitVar[int] = 1\n"
              "    def __post_init__(self, a: int = 1) -> None:  {c}\n        pass\n",
    "RUF034": "x = 1 if c else 1  {c}\n",
    "RUF037": "from collections import deque\nx = deque([])  {c}\n",
    "RUF040": "assert x, 1  {c}\n",
    "RUF041": "from typing import Literal\nx: Literal[Literal[1], 2]  {c}\n",
    "RUF046": "x = int(round(1.5))  {c}\n",
    "RUF048": "x = tuple(map(int, __version__.split(\".\")))  {c}\n",
    "RUF049": "from dataclasses import dataclass\nfrom enum import Enum\n@dataclass  {c}\nclass A(Enum):\n    X = 1\n",
    "RUF051": "def f(k):\n    d = {{}}\n    if k in d:\n        del d[k]  {c}\n",
    "RUF057": "x = round(42)  {c}\n",
    "RUF058": "import itertools\nx = itertools.starmap(f, zip(a, b))  {c}\n",
    "RUF059": "def f():\n    a, b = g()  {c}\n    return a\n",
    "RUF060": "x = 1 in []  {c}\n",
    "RUF061": "import pytest\npytest.raises(ZeroDivisionError, int, \"x\"){c}\n",
    "RUF063": "class A: ...\nA.__dict__[\"__annotations__\"]{c}\n",
    "RUF064": "import os\nos.chmod(\"f\", 444){c}\n",
    "RUF068": "__all__ = [\"a\", \"a\"]{c}\n",
    "SLF001": "foo._bar{c}\n",
    "SLOT000": "class A(str):{c}\n    pass\n",
    "SLOT001": "class A(tuple):{c}\n    pass\n",
    "SLOT002": "from collections import namedtuple\nclass A(namedtuple(\"A\", \"x\")):{c}\n    pass\n",
    "T201": "print(\"x\"){c}\n",
    "T203": "from pprint import pprint\npprint(\"x\"){c}\n",
    "TC005": "from typing import TYPE_CHECKING\nif TYPE_CHECKING:\n    pass{c}\n",
    "TD001": "# FIXME(a): b https://x.y{c}\n",
    "TD002": "# TODO: b https://x.y{c}\n",
    "TD003": "# TODO(a): b{c}\n",
    "TD004": "# TODO(a) b https://x.y{c}\n",
    "TD005": "# TODO(a):{c}\n",
    "TD006": "# todo(a): b https://x.y{c}\n",
    "TD007": "# TODO(a):b https://x.y{c}\n",
    "TID252": "from .. import a{c}\n",
    "TRY002": "raise Exception{c}\n",
    "TRY003": "raise ValueError(\"a long message\"){c}\n",
    "TRY004": "if isinstance(x, int):\n    pass\nelse:\n    raise ValueError{c}\n",
    "TRY201": "try:\n    pass\nexcept ValueError as e:\n    raise e{c}\n",
    "TRY203": "try:\n    pass\nexcept ValueError:{c}\n    raise\n",
    "TRY300": "try:\n    a = 1\n    return a{c}\nexcept ValueError:\n    pass\n",
    "TRY301": "try:\n    raise ValueError{c}\nexcept Exception:\n    pass\n",
    "TRY400": "import logging\n\ntry:\n    pass\nexcept Exception:\n    logging.error(\"failed\"){c}\n",
    "TRY401": "import logging\n\ntry:\n    pass\nexcept Exception as e:\n    logging.exception(f\"failed: {{e}}\"){c}\n",
    "UP001": "class A:\n    __metaclass__ = type{c}\n",
    "UP003": "type(\"\"){c}\n",
    "UP004": "class A(object):{c}\n    pass\n",
    "UP005": "import unittest\n\n\nclass A(unittest.TestCase):\n    def test(self):\n"
             "        self.assertEquals(1, 1){c}\n",
    "UP006": "from typing import List\n\nx: List[int] = []{c}\n",
    "UP007": "from typing import Union\n\nx: Union[int, str] = 1{c}\n",
    "UP008": "class A:\n    pass\n\n\nclass B(A):\n    def f(self):\n        super(B, self).f(){c}\n",
    "UP010": "from __future__ import print_function{c}\n",
    "UP011": "import functools\n\n\n@functools.lru_cache(){c}\ndef f():\n    pass\n",
    "UP012": "\"foo\".encode(\"utf-8\"){c}\n",
    "UP013": "from typing import TypedDict\n\nA = TypedDict(\"A\", {{\"a\": int}}){c}\n",
    "UP014": "from typing import NamedTuple\n\nA = NamedTuple(\"A\", [(\"a\", int)]){c}\n",
    "UP015": "open(\"f\", \"r\"){c}\n",
    "UP018": "str(\"foo\"){c}\n",
    "UP019": "from typing import Text\n\nx: Text = \"\"{c}\n",
    "UP020": "import io\n\nio.open(\"f\"){c}\n",
    "UP021": "import subprocess\n\nsubprocess.run([\"ls\"], universal_newlines=True){c}\n",
    "UP022": "import subprocess\n\nsubprocess.run([\"ls\"], stdout=subprocess.PIPE, stderr=subprocess.PIPE){c}\n",
    "UP023": "import xml.etree.cElementTree as ET{c}\n",
    "UP024": "try:\n    pass\nexcept IOError:{c}\n    pass\n",
    "UP025": "x = u\"foo\"{c}\n",
    "UP028": "def f():\n    for x in range(3):{c}\n        yield x\n",
    "UP029": "def f():\n    from builtins import str{c}\n",
    "UP030": "print(\"{{0}}\".format(1)){c}\n",
    "UP032": "a = 1\nprint(\"{{}}\".format(a)){c}\n",
    "UP033": "from functools import lru_cache\n\n@lru_cache(maxsize=None){c}\ndef f():\n    pass\n",
    "UP034": "print((\"foo\")){c}\n",
    "UP035": "from typing import List{c}\n",
    "UP036": "import sys\n\nif sys.version_info < (3, 0):{c}\n    pass\n",
    "UP039": "class A():{c}\n    pass\n",
    "UP041": "import socket\n\ntry:\n    pass\nexcept socket.timeout:{c}\n    pass\n",
    "UP043": "from collections.abc import Generator\n\ndef f() -> Generator[int, None, None]: ...{c}\n",
    "UP049": "class A[_T]:{c}\n    pass\n",
    "UP050": "class A(metaclass=type):{c}\n    pass\n",
    "YTT101": "import sys\nx = sys.version[:3]{c}\n",
    "YTT102": "import sys\nx = sys.version[2]{c}\n",
    "YTT103": "import sys\nx = sys.version > \"3.9\"{c}\n",
    "YTT201": "import sys\nx = sys.version_info[0] == 3{c}\n",
    "YTT202": "import six\nx = six.PY3{c}\n",
    "YTT203": "import sys\nx = sys.version_info[1] >= 5{c}\n",
    "YTT204": "import sys\nx = sys.version_info.minor > 3{c}\n",
    "YTT301": "import sys\nx = sys.version[0]{c}\n",
    "YTT302": "import sys\nx = sys.version >= \"3\"{c}\n",
    "YTT303": "import sys\nx = sys.version[:1]{c}\n",

}

NOQA_COMMENTS = ["", "  # noqa", "  # noqa: {code}", "  # NOQA: {code}", "  # noqa: E501"]

# Rules Ruff reports only in stubs: their case is a stub, fixtures/rerail/noqa/<CODE>.pyi.
NOQA_STUBS: tuple[str, ...] = (
    "PYI001",
    "PYI002",
    "PYI003",
    "PYI004",
    "PYI005",
    "PYI007",
    "PYI008",
    "PYI009",
    "PYI010",
    "PYI011",
    "PYI012",
    "PYI014",
    "PYI015",
    "PYI017",
    "PYI020",
    "PYI021",
    "PYI026",
    "PYI029",
    "PYI033",
    "PYI035",
    "PYI044",
    "PYI048",
    "PYI050",
    "PYI052",
    "PYI054",
    "PYI066",
    "UP043",
)

# Shipped rules with no case, and why.
NOQA_EXEMPT = {
    "D104": "Ruff reports it once per package `__init__.py`, at its first line, so one file cannot hold the case five times.",
}

# Ours: inputs a port once misread, each now a golden case (fixtures/rerail/probes/<name>.py).
PROBES = {
    # B025: a nested `try` inside a later handler has handlers of its own.
    "B025-nested-try.py": "try:\n    a()\nexcept ValueError:\n    pass\nexcept Exception:\n    try:\n        b()\n"
                          "    except ValueError:\n        pass\n",
    # F602: a string key and a same-named variable key are different keys.
    "F602-string-and-name-key.py": "a = 1\nd = {\"a\": 1, a: 2}\ne = {a: 1, \"a\": 2}\n",
    # SIM905: a comment before a positional argument does not make it absent.
    "SIM905-comment-before-argument.py": "SEP = \",\"\n\"a,b\".split(  # configured separator\n    SEP\n)\n"
                                         "\"a,b\".split(\n    # the separator\n    \",\"\n)\n"
                                         "\"a b\".split(None,  # at most once\n    1)\n",
    # From Moti's review of #29: parentheses, comments, `\` continuations and t-strings that change the tree-sitter
    # shape but not Python's AST. Each port still misreads its case, so it stays a candidate until it agrees.
    "B017-comment-before-argument.py": "import pytest\ndef test_a():\n    with pytest.raises(Exception,  # c\n"
                                       "                       \"op()\"):\n        pass\n",
    "B017-continuation.py": "import pytest\ndef test_b():\n    with pytest.raises(Exception, \\\n    ):\n        pass\n",
    "B019-parenthesized-decorator.py": "import functools\nclass C:\n    @(staticmethod)\n    @functools.cache\n"
                                       "    def f(x):\n        return x\n    @(classmethod)\n    @functools.cache\n"
                                       "    def g(cls):\n        return cls\n",
    "E712-comment-in-tuple.py": "if True == (1,  # c\n            2):\n    pass\n",
    "E713-E714-comment-in-operator.py": "a = b = c = 1\nif not (a in b not  # c\n        in c):\n    pass\n"
                                        "if not (a is b is  # c\n        not c):\n    pass\n",
    "F404-concatenated-docstring.py": "(\"module \"  # comment\n \"docstring\")\nfrom __future__ import annotations\n",
    "F631-F634-continuation-empty-tuple.py": "assert ( \\\n)\nif ( \\\n):\n    pass\n",
    "PIE790-parenthesized-type-checking.py": "from typing import TYPE_CHECKING\nif (TYPE_CHECKING):\n    pass\nelse:\n"
                                             "    ...\nif (TYPE_CHECKING):\n    \"\"\"doc\"\"\"\n    ...\n",
    "PIE808-comment-after-start.py": "for i in range(0,  # c\n               ):\n    pass\n",
    "RET504-parenthesized-suppress.py": "from contextlib import suppress\ndef f():\n"
                                        "    with (suppress(ValueError)) as cm:\n        value = int(\"1\")\n"
                                        "    return value\n",
    "SIM115-parenthesized-argument.py": "import contextlib\nwith contextlib.ExitStack() as stack:\n"
                                        "    f = stack.enter_context((open(\"f\")))\n",
    "SIM201-SIM202-dunder-comprehension-lambda.py": "class C:\n    def __ne__(self, other):\n"
                                                    "        a = [x for x in (not self == other,)]\n"
                                                    "        g = lambda y=not self == other: y\n        return a, g\n"
                                                    "    def __eq__(self, other):\n"
                                                    "        a = [x for x in (not self != other,)]\n"
                                                    "        g = lambda y=not self != other: y\n        return a, g\n",
    "SIM220-SIM221-t-strings.py": "a = True\nx = a and (t\"x\" + t\"y\") and not a\ny = a or (t\"x\" + t\"y\") or not a\n",
    # From the audit of #29's ASYNC rules, the same way: each port still misreads its case.
    # ASYNC109: a parenthesized `@(override)` is still an override.
    "ASYNC109-parenthesized-override.py": "import trio\nfrom typing import override\nclass C:\n    @(override)\n"
                                          "    async def i(self, timeout):\n        pass\n",
    # ASYNC110: a tuple whose first item is the `await` is not a lone sleep.
    "ASYNC110-tuple-body.py": "import trio\nasync def f(c):\n    while c:\n        await trio.sleep(1), 1\n",
    # ASYNC212, ASYNC240: a module binding named like a builtin shadows it, so it is not the only binding.
    "ASYNC212-ASYNC240-builtin-name-module-binding.py": "import httpx\nfrom pathlib import Path\n"
                                                        "license = Path(\"LICENSE\")\nhelp = httpx.Client()\n"
                                                        "async def f():\n    license.read_text()\n"
                                                        "    help.get(\"https://example.com\")\n",
    # ASYNC220, ASYNC221: a mode bound by a `with` item is `os.P_WAIT`, so the call is ASYNC221's.
    "ASYNC220-ASYNC221-with-item-mode.py": "import os\nasync def f():\n    with os.P_WAIT as m:\n"
                                           "        os.spawnl(m, \"x\")\n",
    # UP039: a backslash continuation (and a comment) inside a class's empty parentheses is trivia, not an argument; starred and ** arguments are arguments
    "UP039-line-continuation-in-parentheses.py": "class A(\\\n"
                                                 "):\n"
                                                 "    pass\n"
                                                 "\n"
                                                 "\n"
                                                 "class B(  # c\n"
                                                 "    \\\n"
                                                 "):\n"
                                                 "    pass\n"
                                                 "\n"
                                                 "\n"
                                                 "class C[T]():\n"
                                                 "    pass\n"
                                                 "\n"
                                                 "\n"
                                                 "class D(*()):\n"
                                                 "    pass\n"
                                                 "\n"
                                                 "\n"
                                                 "class E(**{}):\n"
                                                 "    pass\n",
    # UP041: parentheses around except types, tuple elements, raised names and callees are transparent; a raise's from-cause and a list element are no site
    "UP041-parenthesized-sites.py": "import socket\n"
                                    "from socket import timeout\n"
                                    "\n"
                                    "try:\n"
                                    "    pass\n"
                                    "except (socket.timeout):\n"
                                    "    pass\n"
                                    "\n"
                                    "try:\n"
                                    "    pass\n"
                                    "except ((socket.timeout), KeyError) as e:\n"
                                    "    pass\n"
                                    "\n"
                                    "try:\n"
                                    "    pass\n"
                                    "except ((  # comment\n"
                                    "    timeout,\n"
                                    ")):\n"
                                    "    pass\n"
                                    "\n"
                                    "try:\n"
                                    "    pass\n"
                                    "except (KeyError, (ValueError, socket.timeout)):\n"
                                    "    pass\n"
                                    "\n"
                                    "raise (socket\n"
                                    "       .timeout)\n"
                                    "raise KeyError from socket.timeout\n"
                                    "raise socket.timeout()\n"
                                    "x = (socket.timeout)(\"message\")\n"
                                    "y = [socket.timeout]\n",
    # B905: a comment or backslash in an argument list was counted as an argument; [*zip(a, b)] parses as a call of *zip
    "B905-trivia-and-starred-call.py": "import itertools\n"
                                       "\n"
                                       "# A comment or backslash in the argument list is not an argument.\n"
                                       "zip(a, \\\n"
                                       "    )\n"
                                       "zip(a,  # c\n"
                                       "    )\n"
                                       "zip(a, itertools.repeat(1, \\\n"
                                       "                        ))\n"
                                       "zip(a, itertools.repeat(1, times  # c\n"
                                       "                        =None))\n"
                                       "zip(a, itertools.repeat(1, \\\n"
                                       "                        None))\n"
                                       "zip(a, b \\\n"
                                       "    )\n"
                                       "\n"
                                       "# tree-sitter reads a starred call first in a list display as a call of `*zip`.\n"
                                       "[*zip(a, b)][0]\n"
                                       "[*zip(a, b, strict=True)]\n"
                                       "[*zip(a)]\n",
    # UP001: tree-sitter reads (__metaclass__) as a one-element tuple_pattern, a Name target in Ruff; annotated, tuple and chained targets stay silent
    "UP001-parenthesized-target.py": "class A:\n"
                                     "    (__metaclass__) = type\n"
                                     "    ((__metaclass__)) = (  # comment\n"
                                     "        type\n"
                                     "    )\n"
                                     "    __metaclass__ = \\\n"
                                     "        type\n"
                                     "    __metaclass__: type = type\n"
                                     "    (__metaclass__,) = type\n"
                                     "    a = __metaclass__ = type\n"
                                     "    __metaclass__ = type; x = 1\n"
                                     "    __metaclass__ = (type)  # noqa: UP001\n"
                                     "\n"
                                     "\n"
                                     "__metaclass__ = type\n",
    # W605: nested format specs are not scanned; a literal ending in a backslash takes the next { or }; bytes accept \N \u \U
    "W605-format-spec-and-field-edges.py": "a = b = 1\n"
                                           "nested = f\"{a:{b:\\w}\\d}\"\n"
                                           "field = f\"\\{a}\\\\{a}\\\\\\{a}\"\n"
                                           "spec_end = f\"{a!r:\\d\\\\}\"\n"
                                           "spec_odd = f\"{a:\\\\\\}\"\n"
                                           "spec_even = f\"{a:\\\\}\"\n"
                                           "spec_brace = f\"{a:\\{b}}\"\n"
                                           "spec_string = f\"{a:{'x'}\\%}\"\n"
                                           "bytes_n = b\"\\N\\u\\U\\q\"\n"
                                           "cont = \"x\\\n"
                                           "\\q\"\n",
    # UP003: one positional literal through parentheses, comments and a backslash, keywords beside it, 0j/1.5e3J as complex, f-strings, None and -1 silent
    "UP003-argument-shapes.py": "import builtins\n"
                                "type(\"\", x=1)\n"
                                "type((\"a\"))\n"
                                "type(  # c\n"
                                "    0j\n"
                                ")\n"
                                "type(1, 2)\n"
                                "type(*\"a\")\n"
                                "type(f\"a\")\n"
                                "type(\"a\" f\"b\")\n"
                                "type(b\"a\" b\"c\")\n"
                                "type(True)\n"
                                "type(None)\n"
                                "type(...)\n"
                                "type(-1)\n"
                                "type(0x1F)\n"
                                "type(1.5e3J)\n"
                                "builtins.type(1)\n"
                                "(type)(1)\n"
                                "type(1,)\n"
                                "type(\\\n"
                                "1)\n"
                                "type(1)(2)\n"
                                "def f(type):\n"
                                "    type(1)\n",
    # F722: the annotation grammar refused a value starting with a backslash-newline, which Ruff reads as nothing
    "F722-leading-continuation.py": "# Ruff reads a backslash-newline at the start of a string annotation as nothing.\n"
                                    "a: '''\\\\\n"
                                    "list[int]''' = [42]\n"
                                    "b: '''\\\\\n"
                                    "\\\\\n"
                                    "int''' = 1\n"
                                    "c: \"\\\\\\nint\" = 1\n"
                                    "# A trailing one, or an indented line after it, is a syntax error.\n"
                                    "d: '''list[int]\\\\\n"
                                    "''' = [42]\n"
                                    "e: \"\\\\\\n int\" = 1\n",
    # UP004: a base inside parentheses is reported on the name within them; keyword, starred and shadowed bases stay silent
    "UP004-parenthesized-bases.py": "import builtins\n"
                                    "import builtins as b\n"
                                    "\n"
                                    "\n"
                                    "class A((object)):\n"
                                    "    ...\n"
                                    "\n"
                                    "\n"
                                    "class B(  # comment\n"
                                    "    (  # inner\n"
                                    "        builtins.object\n"
                                    "    ),\n"
                                    "    metaclass=object,\n"
                                    "):\n"
                                    "    ...\n"
                                    "\n"
                                    "\n"
                                    "class C(*object):\n"
                                    "    ...\n"
                                    "\n"
                                    "\n"
                                    "class D(b.object, \\\n"
                                    "        object):\n"
                                    "    ...\n"
                                    "\n"
                                    "\n"
                                    "def f(object):\n"
                                    "    class E(object):\n"
                                    "        ...\n"
                                    "\n"
                                    "\n"
                                    "class F(object()):\n"
                                    "    ...\n",
    # F631/F634: a line continuation between `assert`/`if`/`elif` and the tuple test, and a continuation inside an empty tuple
    "F631-F634-continuation-before-test.py": "x = 0\n"
                                             "assert \\\n"
                                             "    (1, 2)\n"
                                             "assert ((\\\n"
                                             "))\n"
                                             "if x:\n"
                                             "    pass\n"
                                             "elif \\\n"
                                             "        (1,):\n"
                                             "    pass\n"
                                             "if \\\n"
                                             "        (x, x):\n"
                                             "    pass\n",
    # From the ports' reviews: one probe each, an edge Ruff's own tests leave out.
    # AIR001: an `airflow.providers.**.operators.*` callee written as a dotted chain broken by a comment, newline or parentheses still resolves, as in Ruff
    "AIR001-provider-chain-comments.py": "import airflow.providers\n\n"
                                         "a = (airflow.providers.google.operators  # comment\n"
                                         "     .bq.Op)(task_id=\"b\")\nc = (airflow.providers.google\n"
                                         "     .operators.bq.Op)(task_id=\"d\")\n"
                                         "e = airflow.providers.google. operators .bq.Op(task_id=\"f\")\n"
                                         "import airflow\n"
                                         "g = (airflow.providers.google).operators.bq.Op(task_id=\"h\")\n"
                                         "i = (airflow.providers.google.operators).bq.Op(task_id=\"j\")\n"
                                         "k = airflow.providers.google.operators.Op.method(task_id=\"l\")\n"
                                         "m = airflow.providers.operators.hooks.x.y(task_id=\"n\")\n",
    # AIR002: multi-line DAG call, decorator call, and module-alias attribute callees
    "AIR002-deep-forms.py": "from airflow.models.dag import DAG\nfrom airflow.sdk.definitions.dag import dag\n"
                            "import airflow.sdk.definitions.dag as defs\n\nDAG(\n    dag_id=\"x\",\n)\n"
                            "dag()(lambda: None)\ndefs.DAG(\"a\")\ndefs.dag(\"b\")\ndefs.other(\"c\")\n",
    # AIR301: each class family checks only its own suffix (TaskHandler, AuthManager, Backend, Hook) as Ruff does, not any of the four
    "AIR301-family-suffixes.py": "from airflow.log.x import FooHook, FooBackend, FooAuthManager\n"
                                 "from airflow.secrets.x import BarHook, BarTaskHandler, BarAuthManager\n"
                                 "from airflow.auth.manager.x import BazHook, BazBackend, BazTaskHandler\n"
                                 "from airflow.providers.p.auth_manager.y import QuxBackend\n"
                                 "from airflow.providers.p.log.y import QuxAuthManager\n"
                                 "from airflow.datasets.secrets import DsBackend\n\nFooHook(filename_template=\"x\")\n"
                                 "FooBackend(filename_template=\"x\")\nFooAuthManager(filename_template=\"x\")\n"
                                 "BazHook(1)\nBazBackend(1)\nBazTaskHandler(1)\nQuxBackend(1)\n"
                                 "QuxAuthManager(filename_template=\"x\")\nb = BarHook()\nb.get_conn_uri()\n"
                                 "b.get_connections()\nc = BarTaskHandler()\nc.get_conn_uri()\nd = BarAuthManager()\n"
                                 "d.get_conn_uri()\ne = BazBackend()\ne.is_authorized_dataset()\nf = DsBackend()\n"
                                 "f.get_conn_uri()\nf.iter_datasets()\n",
    # AIR302: Pins the exact (module, name) comparison of the try/except guard: a wrong module, a nested exception tuple, an aliased dotted import, an attribute read of another name, and a rename read by its new name, plus string annotations.
    "AIR302-exact-replacement-guard.py": "import airflow\n\ntry:\n"
                                         "    from airflow.providers.standard.operators.python import S3Hook\n"
                                         "except ImportError:\n    from airflow.hooks.S3_hook import S3Hook\nS3Hook()\n"
                                         "try:\n"
                                         "    from airflow.providers.amazon.aws.hooks.s3 import provide_bucket_name\n"
                                         "except (ValueError, (ImportError,)):\n"
                                         "    from airflow.hooks.S3_hook import provide_bucket_name\n"
                                         "provide_bucket_name()\ntry:\n"
                                         "    from airflow.providers.postgres.hooks.postgres import PostgresHook\n"
                                         "except ImportError:\n"
                                         "    import airflow.hooks.postgres_hook.PostgresHook as PostgresHook\n"
                                         "PostgresHook()\ntry:\n    airflow.providers.amazon.aws.hooks.s3.Other\n"
                                         "except AttributeError:\n    airflow.hooks.S3_hook.S3Hook()\ntry:\n"
                                         "    airflow.providers.amazon.aws.transfers.google_api_to_s3.GoogleApiToS3Operator\n"
                                         "except AttributeError:\n"
                                         "    airflow.operators.google_api_to_s3_transfer.GoogleApiToS3Transfer()\n\n\n"
                                         "def f(a: \"S3Hook\", b: \"list[airflow.hooks.S3_hook.S3Hook]\"): ...\n",
    # AIR303: Pins the get_link base forms (aliased dotted import, module alias, nested subscript not stripped) and which signatures pass: positional-only, keyword-only after *args, typed and defaulted ti_key, and the invalid shapes.
    "AIR303-get-link-bases-and-signatures.py": "import airflow.sdk.BaseOperatorLink as BOL\n"
                                               "from airflow.models import baseoperatorlink as bol\n"
                                               "from airflow.sdk import BaseOperatorLink\n\n\nclass A(BOL):\n"
                                               "    def get_link(self, operator): ...\n\n\n"
                                               "class B(bol.BaseOperatorLink):\n    def get_link(self, operator): ...\n"
                                               "\n\nclass C(BaseOperatorLink[int][str]):\n"
                                               "    def get_link(self, operator): ...\n\n\nclass D(BaseOperatorLink):\n"
                                               "    async def get_link(self, operator): ...\n\n    @staticmethod\n"
                                               "    def get_link(operator, *, ti_key): ...\n\n\n"
                                               "class E(BaseOperatorLink):\n"
                                               "    def get_link(self, operator, /, ti_key): ...\n"
                                               "    def get_link(self, /, operator, *, ti_key=None): ...\n"
                                               "    def get_link(self, operator, *args, ti_key): ...\n"
                                               "    def get_link(self, operator, ti_key, *, x): ...\n"
                                               "    def get_link(self, operator, ti_key, **kw): ...\n"
                                               "    def get_link(self, operator, *, ti_key: int = 1): ...\n"
                                               "    def get_link(self, operator, ti_key: int): ...\n"
                                               "    def get_link(self, operator, *, other, ti_key): ...\n\n\n"
                                               "class F(BaseOperatorLink):\n"
                                               "    def get_link(self, ti_key, operator): ...\n"
                                               "    def get_link(self, operator, x, *, ti_key): ...\n"
                                               "    def get_link(self, *, ti_key): ...\n"
                                               "    def get_link(self, operator, *args): ...\n"
                                               "    def get_link(self, operator, *ti_key): ...\n"
                                               "    def get_link(self, operator, **ti_key): ...\n"
                                               "    def get_link(self, operator, *args: int, ti_key): ...\n"
                                               "    def get_link(self, operator, ti_key=None, x=1): ...\n",
    # AIR311: Pins the keyword clauses: an aliased dotted `dag` callee, parenthesized callees, only the first duplicate keyword, and the provider `operators` segment read from the qualified name, not from an alias, a sibling import name or a comment.
    "AIR311-sla-callee-forms.py": "import airflow.models.dag as X\n"
                                  "import airflow.providers.google.cloud.operators.bigquery\nfrom airflow import DAG\n"
                                  "from airflow.operators.bash import BashOperator\n"
                                  "from airflow.providers.google.cloud.hooks import operators, BigQueryOperator\n"
                                  "from airflow.providers.google import operators as ops\n\nX(sla_miss_callback=1)\n"
                                  "(DAG)(sla_miss_callback=1, sla_miss_callback=2)\n(BashOperator)(sla=1)\n"
                                  "airflow.providers.google.cloud.hooks.x.FooOperator(sla=1)\nBigQueryOperator(sla=1)\n"
                                  "ops.FooOperator(sla=1)\n"
                                  "from airflow.providers.google.cloud import hooks as operators\n\n"
                                  "operators.FooOperator(sla=1)\n"
                                  "from airflow.providers.google.x import (  # operators\n    BarOperator,\n)\n\n"
                                  "BarOperator(sla=1)\n",
    # AIR312: Pins four behaviours. A string annotation reads the module binding first, so a function-local guarded import does not hide a module-level unguarded one. An attribute chain in a string ignores a same-named guarded import. Nested strings are read. `import a.b.X as Y` in the handler counts as guarded.
    "AIR312-forward-reference-guards.py": "import airflow\nfrom airflow.operators.python import PythonOperator\n\n"
                                          "try:\n"
                                          "    from airflow.providers.standard.operators.bash import BashOperator\n"
                                          "except ImportError:\n"
                                          "    import airflow.operators.bash.BashOperator as BashOperator\n\n\n"
                                          "def load():\n    try:\n"
                                          "        from airflow.providers.standard.operators.python import PythonOperator\n"
                                          "    except ImportError:\n"
                                          "        from airflow.operators.python import PythonOperator\n"
                                          "    hint: \"PythonOperator\" = PythonOperator()\n    return hint\n\n\n"
                                          "def run(op: \"BashOperator\", raw: \"airflow.operators.bash.BashOperator\") -> \"list['PythonOperator']\":\n"
                                          "    return BashOperator(), op, raw\n",
    # ANN001: A module-level rebinding of staticmethod/classmethod after the classes still disables the builtin decorators everywhere (Ruff resolves them in the module's final state), including in classes nested in functions.
    "ANN001-staticmethod-shadowed-later-in-module.py": "class Early:\n    @staticmethod\n    def s(a, b):\n"
                                                       "        return a\n\n    @classmethod\n    def c(cls, a):\n"
                                                       "        return a\n\n\ndef factory():\n    class Late:\n"
                                                       "        @staticmethod\n        def s(a, b):\n"
                                                       "            return a\n\n        @classmethod\n"
                                                       "        def c(cls, a):\n            return a\n\n"
                                                       "    return Late\n\n\nstaticmethod = lambda fn: fn\n"
                                                       "classmethod = lambda fn: fn\n\n\nclass After:\n"
                                                       "    @staticmethod\n    def s(a, b):\n        return a\n\n"
                                                       "    @classmethod\n    def c(cls, a):\n        return a\n",
    # ANN002: Comments between the star and the name, overload implementations in modules and classes, receiver-less methods, nested functions and lambdas.
    "ANN002-star-spacing-and-overloads.py": "from typing import overload\n\n\ndef spaced(\n    *  # star comment\n"
                                            "    args,\n    **  # dstar comment\n    kwargs,\n):\n    pass\n\n\n"
                                            "@overload\ndef impl(*args: int) -> int: ...\ndef impl(*args, **kwargs):\n"
                                            "    return 0\n\n\nclass A:\n    @overload\n"
                                            "    def m(self, *a: int) -> int: ...\n    def m(self, *a, **k):\n"
                                            "        return 0\n\n    def n(*args, **kwargs):\n        return 0\n\n\n"
                                            "def outer(*args, **kwargs):\n    def inner(*a, **k):\n        return a\n"
                                            "    return lambda *x, **y: x\n\n\ndef only_kw(*, a, **rest):\n    pass\n\n"
                                            "\ndef typed_kwargs(*args: \"tuple[int, ...]\", **kwargs: dict):\n"
                                            "    pass\n",
    # ANN003: Spacing and comments after `**`, static and receiver-less methods, dummy names and lambdas.
    "ANN003-kwargs-layout-and-lambda.py": "def f(*args, **kwargs):\n    pass\n\n\n"
                                          "def g(*args: int, **kwargs: str) -> None:\n    pass\n\n\n"
                                          "def h(a, *rest, b, **opts):\n    pass\n\n\ndef i(*, a, **kw):\n    pass\n\n"
                                          "\ndef j(* args, ** kwargs):\n    pass\n\n\ndef k(\n    *args,  # c\n"
                                          "    **kwargs  # d\n):\n    pass\n\n\nclass A:\n"
                                          "    def m(self, *args, **kwargs):\n        pass\n\n    @staticmethod\n"
                                          "    def s(*args, **kwargs):\n        pass\n\n"
                                          "    def only_star(*args, **kwargs):\n        pass\n\n\n"
                                          "lam = lambda *a, **k: 0\n\n\ndef l(*_, **__):\n    pass\n\n\n"
                                          "def m(a, *args: \"int\", **kwargs: \"Any\"):\n    pass\n",
    # ANN201: Only list()/tuple() calls with exactly one positional literal argument (parentheses aside) list names; extra arguments, star arguments, keywords, other builtins and non-string elements do not, and a self-concatenation adds its literal operand.
    "ANN201-dunder-all-builtin-call-shapes.py": "__all__ = list([\"a\"], [\"b\"])\n__all__ += list(*[[\"c\"]])\n"
                                                "__all__ += tuple([\"d\"], key=1)\n__all__ += tuple(([\"e\"]))\n"
                                                "__all__ += list()\n__all__ += sorted([\"f\"])\ny = 1\n"
                                                "__all__ += [y, \"g\"]\n__all__ = __all__ + [\"h\"]\n\n\n"
                                                "def a(): pass\ndef b(): pass\ndef c(): pass\ndef d(): pass\n"
                                                "def e(): pass\ndef f(): pass\ndef g(): pass\ndef h(): pass\n",
    # ANN202: Starred and non-string `__all__` elements, a call value and a chained `x = __all__ = ...` add no names, so the unlisted function is private while the literal entry stays public.
    "ANN202-nonliteral-dunder-all-entries.py": "import other\n\n__all__ = [*other.names, \"a\", other.b]\n"
                                               "__all__ += other.more()\nx = __all__ = [\"b\"]\n\n\ndef a(p):\n"
                                               "    return p\n\n\ndef b(p):\n    return p\n",
    # ANN204: Pins that a module-level bare annotation of `staticmethod` and an `except ... as classmethod` do not shadow the builtins, so the static `__new__` and class `__init_subclass__` go to the other rules and only `__init__` is reported.
    "ANN204-annotation-only-and-except-bound-builtins.py": "staticmethod: type\n\ntry:\n    pass\n"
                                                           "except Exception as classmethod:\n    pass\n\n\nclass A:\n"
                                                           "    @staticmethod\n    def __new__(cls): ...\n"
                                                           "    @classmethod\n    def __init_subclass__(cls): ...\n"
                                                           "    def __init__(self): ...\n",
    # ANN205: Pins that a module-level bare annotation and an `except ... as staticmethod` leave `staticmethod` the builtin (two methods reported, dunder included), while classmethod still wins over staticmethod.
    "ANN205-annotation-only-and-except-bound-staticmethod.py": "staticmethod: object\n\ntry:\n    pass\n"
                                                               "except Exception as staticmethod:\n    pass\n\n\n"
                                                               "class A:\n    @staticmethod\n    def s(): ...\n\n"
                                                               "    @staticmethod\n    def __new__(cls): ...\n\n"
                                                               "    @staticmethod\n    @classmethod\n"
                                                               "    def c(cls): ...\n\n",
    # ANN206: Pins that a module-level bare annotation and an `except ... as classmethod` leave `classmethod` the builtin, including on a method also decorated with staticmethod.
    "ANN206-annotation-only-and-except-bound-classmethod.py": "classmethod: object\n\ntry:\n    pass\n"
                                                              "except Exception as classmethod:\n    pass\n\n\n"
                                                              "class A:\n    @classmethod\n    def c(cls): ...\n\n"
                                                              "    @classmethod\n    @staticmethod\n"
                                                              "    def d(cls): ...\n",
    # ANN401: Pins Ruff's end-of-module fallback for typing names unbound at a module- or class-level signature: later aliased and module imports count, a function-local import does not, and a later rebinding cancels it.
    "ANN401-late-module-imports.py": "def f(a: A, b: t.Any, c: Opt, d: t.Union[int, A], e: Late) -> Opt[A]: ...\n\n\n"
                                     "class K:\n    def m(self, x: A, y: t.Optional) -> t.Any: ...\n\n\ndef setup():\n"
                                     "    from typing import Any as Local\n    return Local\n\n\ndef g(a: Local): ...\n"
                                     "\n\nfrom typing import Any as A, Optional as Opt\n"
                                     "from typing import Any as Late\nimport typing as t\n\nLate = object\n",
    # ARG001: Pins Ruff's variable-stub exemption when the raise passes a lone generator expression: exempt with a non-name target, reported with a name target; Ruff's fixtures only use a name argument.
    "ARG001-variable-stub-generator-argument.py": "def a(x):\n    obj.msg = \"not implemented\"\n"
                                                  "    raise NotImplementedError(i for i in ())\n\n\ndef b(x):\n"
                                                  "    msg = \"not implemented\"\n"
                                                  "    raise NotImplementedError(i for i in ())\n\n\n"
                                                  "def c(x, y=lambda z: locals()):\n    obj.msg = f\"{y}\"\n"
                                                  "    raise NotImplementedError(*y)\n",
    # ARG002: Pins the variable-stub exemption for a method whose raise passes a lone generator expression (exempt with `self.msg`, reported with `msg`), which Ruff's fixtures never use.
    "ARG002-variable-stub-generator-argument.py": "class C:\n    def a(self, x):\n"
                                                  "        self.msg = \"not implemented\"\n"
                                                  "        raise NotImplementedError(i for i in ())\n\n"
                                                  "    def b(self, x):\n        msg = \"not implemented\"\n"
                                                  "        raise NotImplementedError(i for i in ())\n\n"
                                                  "    def c(self, x):\n        self.msg = \"not implemented\"\n"
                                                  "        raise NotImplementedError(x for x in ())\n",
    # ARG003: Pins which parameter a class method skips (the first non-variadic one, including a keyword-only or defaulted first parameter and a missing cls) across positional-only, keyword-only, variadic and defaulted shapes, plus the implicit class methods, which Ruff's ARG fixtures do not exercise.
    "ARG003-skipped-first-parameter-forms.py": "class C:\n    def a(self, x, /, y, *, z, **kw): print(1)\n"
                                               "    def b(self, /, x): print(1)\n    def c(*, x): print(1)\n"
                                               "    def d(self, *, x): print(1)\n"
                                               "    def e(self, *, x=1, y=2): print(1)\n"
                                               "    def f(self=None, x=1): print(1)\n"
                                               "    def g(self: \"C\", *args: int, **kwargs: str): print(1)\n"
                                               "    def h(self, x=lambda y: 0): print(1)\n"
                                               "    def i(*args, x): print(1)\n    def j(*args, **kwargs): print(1)\n"
                                               "    def k(**kwargs): print(1)\n\n    @classmethod\n"
                                               "    def ca(cls, /, x, *, y): print(1)\n\n    @classmethod\n"
                                               "    def cb(*, x): print(1)\n\n    @classmethod\n"
                                               "    def cc(*args, x): print(1)\n\n    @classmethod\n"
                                               "    def cd(): print(1)\n\n    @staticmethod\n"
                                               "    def sa(x, /, y, *, z, **kw): print(1)\n\n    @staticmethod\n"
                                               "    def sb(*, x): print(1)\n\n    @staticmethod\n"
                                               "    def sc(): print(1)\n\n    def __new__(cls, /, x, *, y): print(1)\n"
                                               "\n    def __new__(*, x): print(1)\n\n"
                                               "    def __init_subclass__(cls, /, x, *, y): print(1)\n\n"
                                               "    def __class_getitem__(cls, x, y): print(1)\n\n\n"
                                               "def fn(a, /, b, *, c): print(1)\ndef fn2(*, a): print(1)\n"
                                               "def fn3(a=1, /, b=2, *c, d=3, **e): print(1)\n"
                                               "lam1 = lambda a, /, b, *, c: 0\nlam2 = lambda *, a: 0\n"
                                               "lam3 = lambda a=1, /, b=2, *c, d=3, **e: 0\n",
    # ARG004: Pins the implicit static `__new__` (first parameter skipped, magic-name exemption not applied, `**kw` checked) next to metaclass magic methods, an unmarked enum hook, protocol stubs and dataclass hooks, which Ruff's ARG fixtures do not combine.
    "ARG004-metaclass-and-enum-static-methods.py": "import enum\nfrom dataclasses import dataclass\n"
                                                   "from typing import NamedTuple, Protocol\n\n\n@dataclass\nclass D:\n"
                                                   "    x: int\n\n    def m(self, a):\n        return self.x\n\n"
                                                   "    def __post_init__(self, init_only):\n        pass\n\n\n"
                                                   "class P(Protocol):\n    def m(self, a) -> int: ...\n\n"
                                                   "    def n(self, a) -> int:\n        \"\"\"Doc.\"\"\"\n\n"
                                                   "    def o(self, a) -> int:\n        return 0\n\n\n"
                                                   "class Meta(type):\n    def __new__(mcs, name, bases, ns, **kw):\n"
                                                   "        return super().__new__(mcs, name, bases, ns)\n\n"
                                                   "    def __prepare__(mcs, name, bases, **kw):\n        return {}\n\n"
                                                   "    def __call__(cls, *args, **kwargs):\n"
                                                   "        return super().__call__()\n\n    def method(cls, a):\n"
                                                   "        print(1)\n\n\nclass E(enum.Enum):\n    A = 1\n\n"
                                                   "    def _generate_next_value_(name, start, count, last_values):\n"
                                                   "        return name\n\n    @staticmethod\n"
                                                   "    def _gen(name, start, count, last_values):\n"
                                                   "        return name\n\n\nclass N(NamedTuple):\n    a: int\n\n"
                                                   "    def m(self, b):\n        return self.a\n",
    # ARG005: Pins the locals() rules Ruff's single lambda fixture never reaches: a lambda inside any class is read in the implicit __class__ scope so its parameters stay reported, a comprehension's first iterable belongs to the lambda while later clauses do not, a lone splat of locals() counts, and a lambda nested in another lambda's parameter default is exempted only by its own locals() call.
    "ARG005-lambda-locals-scopes.py": "def deco(*a):\n    return lambda f: f\n\n\nclass A(deco(lambda x: locals())):\n"
                                      "    pass\n\n\nclass B(metaclass=deco(lambda x: locals())):\n    pass\n\n\n"
                                      "@deco(lambda x: locals())\nclass C:\n    pass\n\n\nclass D:\n"
                                      "    @deco(lambda x: locals())\n    def m(self, f=lambda y: locals()):\n"
                                      "        pass\n\n    def n(self):\n        def inner():\n"
                                      "            return lambda z: locals()\n\n        return inner\n\n"
                                      "    e = [lambda w: locals() for _ in range(2)]\n"
                                      "    f = {k: (lambda v: locals()) for k in range(2)}\n"
                                      "    g = lambda a: (lambda b: locals())\n"
                                      "    h = lambda a: [(lambda b: locals()) for _ in range(2)]\n"
                                      "    i = lambda a: (locals)()\n    j = lambda a: (locals())\n"
                                      "    k = lambda a: locals() if a else None\n"
                                      "    l = lambda a: [y for y in (locals)()]\n\n\nclass E(D):\n    class F:\n"
                                      "        o = lambda a: locals()\n\n    p = lambda a: locals()\n\n\ndef outer():\n"
                                      "    class G:\n        pass\n\n    return lambda a: locals()\n\n\n"
                                      "lam = lambda a: (lambda b: locals())\n"
                                      "lam2 = lambda a: [(lambda b: locals()) for _ in range(2)]\n"
                                      "lam3 = lambda a: (locals)()\nlam4 = lambda a: locals() if a else None\n"
                                      "lam5 = lambda a: [y for y in (locals)()]\n"
                                      "lam6 = lambda a: [y for y in (locals)() if y]\n"
                                      "lam7 = lambda a: [y for y in [z for z in range(1) if locals()]]\n"
                                      "lam8 = lambda a: [y for y in range(3) if (z := locals())]\n"
                                      "lam9 = lambda a: {**locals()}\nlam10 = lambda a: [*locals()]\n"
                                      "lam11 = lambda a: locals()[a]\nlam12 = lambda a: locals().get(\"a\")\n"
                                      "lam13 = lambda a: print(a) or locals()\nlam14 = lambda a: not locals()\n"
                                      "lam15 = lambda a: [y for y in range(3)][locals()]\n"
                                      "lam16 = lambda a: lambda: [y for y in locals()]\n"
                                      "lam17 = lambda a=lambda b: locals(): a\n"
                                      "lam18 = lambda a=(lambda b: [locals()]): a\n"
                                      "lam19 = lambda a=lambda b: 0: locals()\nlam20 = lambda a, c=a: c\n",
    # C400: Pins reports inside string annotations (nested string, return annotation in module scope), the silent plain string, and builtin resolution from the annotation's own scope (a parameter and a class attribute shadow).
    "C400-string-annotations.py": "from typing import Dict\nxs = range(3)\na: \"list(x for x in xs)\"\n"
                                  "b: \"Dict['list(2 * x for x in xs)', int]\"\nc = \"list(x for x in xs)\"\n\n\n"
                                  "def f(list: int) -> \"list(x for x in xs)\":\n    d: \"list(x for x in xs)\"\n"
                                  "    return list\n\n\nclass C:\n    list = tuple\n    e: \"list(x for x in xs)\"\n",
    # C401: Pins reports inside string annotations (nested string, return annotation in module scope), the silent plain string, and builtin resolution from the annotation's own scope (a parameter and a class attribute shadow).
    "C401-string-annotations.py": "from typing import Dict\nxs = range(3)\na: \"set(x for x in xs)\"\n"
                                  "b: \"Dict['set(2 * x for x in xs)', int]\"\nc = \"set(x for x in xs)\"\n\n\n"
                                  "def f(set: int) -> \"set(x for x in xs)\":\n    d: \"set(x for x in xs)\"\n"
                                  "    return set\n\n\nclass C:\n    set = tuple\n    e: \"set(x for x in xs)\"\n",
    # C402: Pins reports inside string annotations, the silent one-tuple in a nested annotation and the silent plain string, and the missing builtin check there (a class attribute named dict is still reported).
    "C402-string-annotations.py": "from typing import Dict\nxs = [(1, 2)]\na: \"dict((k, v) for k, v in xs)\"\n"
                                  "b: \"Dict['dict((k,) for k in xs)', int]\"\nc = \"dict((k, v) for k, v in xs)\"\n\n"
                                  "\nclass C:\n    dict = list\n    e: \"dict((k, v) for k, v in xs)\"\n",
    # C403: Pins reports inside string annotations (nested string, return annotation in module scope), the silent plain string, and builtin resolution from the annotation's own scope (a parameter and a class attribute shadow).
    "C403-string-annotations.py": "from typing import Dict\nxs = range(3)\na: \"set([x for x in xs])\"\n"
                                  "b: \"Dict['set([2 * x for x in xs])', int]\"\nc = \"set([x for x in xs])\"\n\n\n"
                                  "def f(set: int) -> \"set([x for x in xs])\":\n    d: \"set([x for x in xs])\"\n"
                                  "    return set\n\n\nclass C:\n    set = frozenset\n    e: \"set([x for x in xs])\"\n",
    # C404: Ruff parses string type definitions (nested ones too) and reports the call inside at its own position, but not inside a plain string.
    "C404-string-annotation.py": "from typing import List\n\n\n"
                                 "def f(a: \"dict([(i, i) for i in x])\", b: \"List['dict([(k, v) for k, v in y])']\"):\n"
                                 "    pass\n\n\nz = \"dict([(i, i) for i in x])\"\n",
    # C405: Calls inside a string annotation, and inside a string nested in one, are reported at their position in the string, but a plain string is not.
    "C405-string-annotation.py": "from typing import List\n\nx: \"List['set([1])']\" = 1\ny: \"set((1, 2))\" = 2\n"
                                 "z = \"set([1])\"\n",
    # C406: An implicitly concatenated string annotation is parsed and reported where Ruff reports it, while a Literal string is not a type definition.
    "C406-string-annotation.py": "from typing import Literal\n\nx: \"dict\" \"([(1, 2)])\" = 1\n"
                                 "y: Literal[\"dict([(1, 2)])\"] = 2\n",
    # C408: Ruff's seen_module(builtins) depends on traversal order: the global-declared call in a function visited before the import is not reported, while the one visited after it is.
    "C408-builtins-import-order.py": "def a():\n    global dict\n    return dict()\n\n\ndef b():\n    import builtins\n"
                                     "\n\ndef c():\n    global list\n    return list()\n",
    # C409: Pins four things: a comprehension's first iterable is read in the enclosing scope, a del under if or in an except handler does not unbind but one in a for body does, and an annotated assignment's value is read before its target is bound.
    "C409-del-order-comprehension-scope.py": "r = [2, 1]\np = [tuple for tuple in tuple([1])]\n"
                                             "q = [y for tuple in r for y in tuple([1])]\n\n\ndef d1(c):\n    if c:\n"
                                             "        del tuple\n    return tuple([1])\n\n\ndef d2(r):\n"
                                             "    for _ in r:\n        del tuple\n    return tuple([1])\n\n\n"
                                             "def d3():\n    try:\n        pass\n    except NameError:\n"
                                             "        del tuple\n    return tuple((1,))\n\n\ndef d4():\n"
                                             "    tuple: type = tuple([1])\n    return tuple\n",
    # C410: Pins deferral. Future-annotation parameter and return annotations, a lambda body and a nested function body all see later bindings, so only the lambda default is reported.
    "C410-deferred-bodies.py": "from __future__ import annotations\n\n\ndef f(a: list([1])) -> list((1,)):\n"
                               "    return a\n\n\ng = lambda: list([1])\nh = lambda a=list((1,)): a\n\n\ndef outer():\n"
                               "    def inner():\n        return list([1])\n\n    list = None\n    return inner\n\n\n"
                               "list = None\n",
    # C411: Pins three things: a bare annotation does not shadow, a global rebinding inside one function does not shadow the builtin elsewhere, and a del in a with body unbinds the name.
    "C411-annotation-global-del.py": "r = [2, 1]\nlist: type\na = list([i for i in r])\n\n\ndef g():\n    global list\n"
                                     "    list = print\n\n\ndef h():\n    return list([i for i in r])\n\n\ndef k():\n"
                                     "    with open(__file__):\n        del list\n    return list([i for i in r])\n",
    # C413: Pins comprehension scoping (only the first iterable is read outside) and that a match capture shadows sorted. A global rebinding in one function does not shadow reversed elsewhere, while a later module binding of list does reach a function body.
    "C413-comprehension-match-global.py": "r = [2, 1]\nx = [reversed(sorted(r)) for reversed in r]\n"
                                          "y = [v for reversed in [r] for v in reversed(sorted(r))]\n"
                                          "z = [v for v in reversed(sorted(r)) for reversed in [r]]\n\n\ndef f(v):\n"
                                          "    match v:\n        case {\"k\": sorted}:\n            pass\n"
                                          "    return reversed(sorted(v))\n\n\ndef g():\n    global reversed\n"
                                          "    reversed = print\n\n\ndef h():\n"
                                          "    return reversed(sorted(r)), list(sorted(r))\n\n\nlist = None\n",
    # C414: Pins that nested sorted calls compare keyword values structurally like ComparableKeyword: parentheses, quote style, implicit concatenation, hex and float spellings are equal; lambdas with different parameter names, reordered or extra keywords and **dict splats are compared as Ruff does.
    "C414-sorted-keyword-equality.py": "x = [3, 1]\nf = abs\nsorted(sorted(x, key=(f)), key=f)\n"
                                       "sorted(sorted(x, key=\"a\"), key='a')\n"
                                       "sorted(sorted(x, key=\"a\" \"b\"), key=\"ab\")\n"
                                       "sorted(sorted(x, key=1), key=0x1)\nsorted(sorted(x, key=1.0), key=1.)\n"
                                       "sorted(sorted(x, key=f, reverse=True), reverse=True, key=f)\n"
                                       "sorted(sorted(x, reverse=True), reverse=False)\n"
                                       "sorted(sorted(x, key=lambda a: a), key=lambda a: a)\n"
                                       "sorted(sorted(x, key=lambda a: a), key=lambda b: b)\n"
                                       "sorted(sorted(x, key=f), key=f, reverse=True)\n"
                                       "sorted(sorted(x, **{\"key\": f}), **{\"key\": f})\n"
                                       "sorted(sorted(x, **{\"key\": f}), **{'key': f})\n"
                                       "sorted(sorted(x, key=f.g), key=f . g)\n"
                                       "sorted(sorted(x, key=[1, 2]), key=[1, 2,])\nsorted(sorted(x, key=f), key=f) \n"
                                       "sorted(sorted(x), key=None)\nsorted(sorted(x, key=None))\n"
                                       "sorted(sorted(x, key=True), key=True)\n"
                                       "sorted(sorted(x, key=b\"a\"), key=b'a')\n"
                                       "sorted(sorted(x, key=f\"{f}\"), key=f'{f}')\n"
                                       "sorted(sorted(x, key=-1), key=-1)\nsorted(sorted(x, key=(1, 2)), key=(1, 2))\n"
                                       "sorted(sorted(x, key=1, reverse=2), key=1, reverse=1)\n",
    # C415: Pins which spellings of the -1 step Ruff accepts: hex, binary, octal and parenthesized integer 1 under unary minus count; floats, complex, bools, double negation, ~1, -10_0 and line-continued forms are checked as Ruff decides.
    "C415-step-literal-spellings.py": "x = [3, 1]\nset(x[::-0x1])\nset(x[::-0b1])\nset(x[::-0o1])\nset(x[::-0X01])\n"
                                      "set(x[::-1_0])\nset(x[::-1e0])\nset(x[::-1j])\nset(x[::-1.])\nset(x[::-True])\n"
                                      "set(x[::+-1])\nset(x[::- -1])\nset(x[::~1])\nset(x[::-(1)])\nset(x[::(-1)])\n"
                                      "set(x[::((-((1))))])\nset(x[::-2])\nset(x[::1])\nset(x[::- 1])\nset(x[::-\\\n"
                                      "1])\nsorted(x[::-10_0])\n",
    # C416: Pins Ruff's builtin lookup for an enclosing comprehension target, an except handler name inside and after its handler, a class binding seen by a comprehension's first iterable, type parameters versus parameter defaults, a walrus earlier in the same lambda body, and a doubly parenthesized items target.
    "C416-scopes-handlers-targets.py": "y = [[1]]\nd = {}\na = [[x for x in z] for list in y]\n"
                                       "b = [(k, w) for ((k, w)) in d.items()]\ntry:\n    pass\n"
                                       "except Exception as list:\n    c = [x for x in y]\n\n    def f():\n"
                                       "        return [x for x in y]\ne = [x for x in y]\n\n\nclass B:\n"
                                       "    def list(self):\n        pass\n    g = [z for z in [x for x in y]]\n"
                                       "    h = [[x for x in y] for z in y]\n\n\ndef m[set](q={x for x in y}):\n"
                                       "    return {x for x in y}\n\n\ndef n(list=[x for x in y]):\n"
                                       "    return [x for x in y]\n\n\nlam = lambda: (list := y, [x for x in y])\n",
    # C417: Pins Ruff's parent-call skip: a map(lambda) passed as a keyword value, a second argument or a ** argument of a builtin list/set/dict call is not reported at all, a *map(...) is, and dict() needs a two-item lambda body.
    "C417-container-parent-skips.py": "y = [1]\ndict(a=map(lambda x: x, y))\nlist(map(lambda x: x, y), 1)\n"
                                      "list(1, map(lambda x: x, y))\ndict(map(lambda x: x, y))\n"
                                      "set(map(lambda x: x, y), key=1)\nlist(**map(lambda x: x, y))\n"
                                      "list(*map(lambda x: x, y))\nf(map(lambda x: x, y))\n"
                                      "list(map(lambda x: x, y) + [])\nlist((map(lambda x: (x, x), y)) for _ in y)\n"
                                      "dict(map(lambda x: [x, x], y))\ndict(map(lambda x: (*x, 1), y))\n"
                                      "dict(map(lambda x: (x,), y))\ndict(map(lambda x: ((x, x)), y))\n"
                                      "dict(map(lambda x: (x, x, x), y))\n",
    # C418: Pins reports inside string type definitions: plain, concatenated (whole-literal span), nested and cast strings report, Literal strings do not, and a shadowing parameter still silences a string annotation.
    "C418-string-annotations.py": "from typing import List, Literal, cast\n\na: \"dict({})\" = 1\n"
                                  "b: \"dict(\" \"{})\" = 1\nc: List[\"dict({1: 2})\"] = []\n"
                                  "d: Literal[\"dict({})\"] = 1\ne = cast(\"(dict)({ })\", None)\n\n\ndef f(dict):\n"
                                  "    x: \"dict({})\" = 1\n    return x\n",
    # C419: Pins reports inside string type definitions: plain, concatenated (whole-literal span), nested and cast strings report, Literal strings do not, and a shadowing parameter silences a string annotation.
    "C419-string-annotations.py": "from typing import List, Literal, cast\n\na: \"any([x for x in y])\" = 1\n"
                                  "b: \"any(\" \"[x for x in y])\" = 1\nc: List[\"all({x for x in y})\"] = []\n"
                                  "d: Literal[\"any([x for x in y])\"] = 1\n"
                                  "e = cast(\"all([x for x in y], 1)\", None)\n\n\ndef f(any):\n"
                                  "    x: \"any([z for z in y])\" = 1\n    return x\n",
    # C420: Pins redundant parentheses around a whole tuple target (reported), 1-tuple versus nested-tuple and list-versus-tuple targets (not reported), and reports inside string annotations, where a self-referential value is still skipped.
    "C420-paren-targets-and-annotations.py": "{(a, b): 1 for ((a, b)) in x}\n{((a, b),): 1 for (a, b), in x}\n"
                                             "{(a, b): 1 for (a, b), in x}\n{[a]: 1 for (a,) in x}\n"
                                             "{a: 1 for ((a)) in x}\nc: \"{a: 1 for (a) in b}\" = 1\n"
                                             "d: \"{(a, b): None for ((a, b)) in c}\" = 1\n"
                                             "e: \"{a: a for a in b}\" = 1\n",
    # C901: Pins which last-case patterns are irrefutable at the exact threshold: as-patterns that wrap `_`, a capture or an or-pattern with `_` cost nothing, while `1 as z`, `None`, `a.b`, `y,`, `[*_]` and `(1 | 2) as z` each count. Ruff's C901.py has no match statement.
    "C901-last-case-forms.py": "# Nine counted cases, then a last case without a guard: an irrefutable one costs nothing (complexity 10, silent),\n"
                               "# a refutable one costs 1 (complexity 11, reported at the default limit of 10).\n\n\n"
                               "def wild_as(x):\n    match x:\n        case 0: pass\n        case 1: pass\n"
                               "        case 2: pass\n        case 3: pass\n        case 4: pass\n"
                               "        case 5: pass\n        case 6: pass\n        case 7: pass\n"
                               "        case 8: pass\n        case _ as z: pass\n\n\ndef capture_as(x):\n    match x:\n"
                               "        case 0: pass\n        case 1: pass\n        case 2: pass\n"
                               "        case 3: pass\n        case 4: pass\n        case 5: pass\n"
                               "        case 6: pass\n        case 7: pass\n        case 8: pass\n"
                               "        case y as z: pass\n\n\ndef group_as(x):\n    match x:\n        case 0: pass\n"
                               "        case 1: pass\n        case 2: pass\n        case 3: pass\n"
                               "        case 4: pass\n        case 5: pass\n        case 6: pass\n"
                               "        case 7: pass\n        case 8: pass\n        case (y as z): pass\n\n\n"
                               "def or_wild_as(x):\n    match x:\n        case 0: pass\n        case 1: pass\n"
                               "        case 2: pass\n        case 3: pass\n        case 4: pass\n"
                               "        case 5: pass\n        case 6: pass\n        case 7: pass\n"
                               "        case 8: pass\n        case (0 | (_)) as z: pass\n\n\ndef value_as(x):\n"
                               "    match x:\n        case 0: pass\n        case 1: pass\n        case 2: pass\n"
                               "        case 3: pass\n        case 4: pass\n        case 5: pass\n"
                               "        case 6: pass\n        case 7: pass\n        case 8: pass\n"
                               "        case 1 as z: pass\n\n\ndef none(x):\n    match x:\n        case 0: pass\n"
                               "        case 1: pass\n        case 2: pass\n        case 3: pass\n"
                               "        case 4: pass\n        case 5: pass\n        case 6: pass\n"
                               "        case 7: pass\n        case 8: pass\n        case None: pass\n\n\n"
                               "def dotted(x):\n    match x:\n        case 0: pass\n        case 1: pass\n"
                               "        case 2: pass\n        case 3: pass\n        case 4: pass\n"
                               "        case 5: pass\n        case 6: pass\n        case 7: pass\n"
                               "        case 8: pass\n        case a.b: pass\n\n\ndef one_tuple(x):\n    match x:\n"
                               "        case 0: pass\n        case 1: pass\n        case 2: pass\n"
                               "        case 3: pass\n        case 4: pass\n        case 5: pass\n"
                               "        case 6: pass\n        case 7: pass\n        case 8: pass\n"
                               "        case y,: pass\n\n\ndef star_list(x):\n    match x:\n        case 0: pass\n"
                               "        case 1: pass\n        case 2: pass\n        case 3: pass\n"
                               "        case 4: pass\n        case 5: pass\n        case 6: pass\n"
                               "        case 7: pass\n        case 8: pass\n        case [*_]: pass\n\n\n"
                               "def values_as(x):\n    match x:\n        case 0: pass\n        case 1: pass\n"
                               "        case 2: pass\n        case 3: pass\n        case 4: pass\n"
                               "        case 5: pass\n        case 6: pass\n        case 7: pass\n"
                               "        case 8: pass\n        case (1 | 2) as z: pass\n",
    # COM818: Pins three tree-sitter shapes the first port missed (0 of 3): a target-only line that tree-sitter joins with the next assignment, a continuation before a blank line after a nested block's last statement, and a double continuation. Also pins silent commas in a continued assignment target and in a multi-line comprehension target.
    "COM818-merged-target-line-and-continuations.py": "def f(self):\n    self.x,\n    self.y = 1\n    if self.x:\n"
                                                      "        return self.y, \\\n\n    a = 1, \\\n        \\\n\n"
                                                      "    b, \\\n        c = a\n    return [b for b,\n"
                                                      "            c in a]\n",
    # D100: A string followed by a comma is a tuple, not a module docstring, so D100 still fires at 1:1.
    "D100-tuple-is-not-docstring.py": "\"\"\"doc\"\"\",\n",
    # D101: A bare-tuple __all__ with parenthesized single-quoted entries and an augmented bare tuple: only the listed classes are public.
    "D101-all-bare-tuple-parenthesized-entries.py": "\"\"\"Doc.\"\"\"\n__all__ = 'Foo', ('f')\n__all__ += (('Zed')),\n"
                                                    "\n\nclass Foo:\n    def m(self): ...\n\n\nclass Zed:\n"
                                                    "    def m(self): ...\n\n\nclass Other:\n    def m(self): ...\n\n\n"
                                                    "def f(): ...\n\n\ndef h(): ...\n",
    # D102: Setter and deleter accessors behind one or two levels of parentheses are private; getter, another name's setter and self.x.setter are not.
    "D102-parenthesized-accessor-decorators.py": "\"\"\"Doc.\"\"\"\n\n\nclass P:\n    \"\"\"Doc.\"\"\"\n\n"
                                                 "    @property\n    def x(self): ...\n\n    @y.setter\n"
                                                 "    def x(self, v): ...\n\n    @x.getter\n    def x(self): ...\n\n"
                                                 "    @((x)).setter\n    def x(self, v): ...\n\n    @(x.deleter)\n"
                                                 "    def x(self): ...\n\n    @self.x.setter\n    def x(self, v): ...\n"
                                                 "\n    @x.setter\n    async def x(self, v): ...\n\n    @x.deleter\n"
                                                 "    def _x(self): ...\n\n    @_x.setter\n    def _x(self, v): ...\n",
    # D103: __all__ built from tuple() over a set with a parenthesized entry, list() and a plain list joined by +: every one of those names is listed.
    "D103-all-builtin-call-and-binop-chain.py": "\"\"\"Doc.\"\"\"\n"
                                                "__all__ = tuple({\"Foo\", (\"f\")}) + list([\"Zed\"]) + [\"g\"]\n\n\n"
                                                "class Foo:\n    def m(self): ...\n\n\nclass Zed:\n"
                                                "    def m(self): ...\n\n\nclass Other:\n    def m(self): ...\n\n\n"
                                                "def f(): ...\n\n\ndef g(): ...\n\n\ndef h(): ...\n",
    # D105: Module classes exported through a `__all__` built by adding tuples and a list are public, an unlisted one is private.
    "D105-all-tuple-plus-list.py": "__all__ = (\"A\",) + (\"B\",) + [\"C\"]\n\n\nclass A:\n"
                                   "    def __eq__(self, o): ...\n\n\nclass B:\n    def __eq__(self, o): ...\n\n\n"
                                   "class C:\n    def __eq__(self, o): ...\n\n\nclass D:\n"
                                   "    def __eq__(self, o): ...\n",
    # D106: An annotated `__all__` plus a `+=` inside an if block both export; the unlisted enclosing class makes its nested class private.
    "D106-annotated-and-conditional-all.py": "__all__: list[str] = [\"Exported\"]\nif True:\n"
                                             "    __all__ += [\"Cond\"]\n\n\nclass Exported:\n    class Inner: ...\n\n"
                                             "\nclass NotExported:\n    class Inner: ...\n\n\nif True:\n"
                                             "    class Cond:\n        class Inner: ...\n",
    # D107: `__init__` exemptions (aliased overload, parenthesized override, own deleter but not getter), continuation-line and concatenated docstrings, and private or local classes.
    "D107-init-decorators-and-forms.py": "import typing\nfrom typing_extensions import overload as ovl\n\n\nclass A:\n"
                                         "    def __init__(self):\n        # comment\n        pass\n\n    @ovl\n"
                                         "    def __init__(self, x): ...\n\n    @__init__.deleter\n"
                                         "    def __init__(self): ...\n\n    @__init__.getter\n"
                                         "    def __init__(self): ...\n\n    @(typing.override)\n"
                                         "    def __init__(self): ...\n\n    def __init__(self): \\\n"
                                         "        \"\"\"Doc.\"\"\"\n\n    def __init__(self): r'''Doc.'''\n\n"
                                         "    def __init__(self):\n        (\"Doc.\"\n         # c\n"
                                         "         \"More.\")\n\n    def __init__(self):\n        b\"Doc.\"\n\n"
                                         "    def __init__(self):\n        x: str = \"Doc.\"\n\n    class Inner:\n"
                                         "        def __init__(self): ...\n\n    class _Inner:\n"
                                         "        def __init__(self): ...\n\n    def method(self):\n"
                                         "        class Local:\n            def __init__(self): ...\n\n"
                                         "        def __init__(self): ...\n\n    if typing.TYPE_CHECKING:\n"
                                         "        def __init__(self): ...\n\n\nclass _B:\n    def __init__(self): ...\n"
                                         "\n\ndef __init__(self): ...\n",
    # D200: A file with bare carriage-return line endings: the break after the opening quotes still makes a one-line docstring multi-line.
    "D200-bare-cr-line-endings.py": "def f():\r    \"\"\"\r    One.\r    \"\"\"\r\r\rdef g():\r    \"\"\"Summary.\r    Desc.\"\"\"\r\n",
    # D205: Bare carriage-return line endings: one blank line after the summary is accepted, two are reported.
    "D205-bare-cr-line-endings.py": "def f():\r    \"\"\"Summary.\r\r    Desc.\"\"\"\r\r\rdef g():\r    \"\"\"Summary.\r\r\r    Desc.\r    \"\"\"\r\r\rdef h():\r    \"\"\"\r\r    One.\r\r    \"\"\"\r\n",
    # D209: An escaped quote or a backslash just before the closing quotes is text on the last line; a tab before them or a backslash line above them is not.
    "D209-escaped-quote-before-closer.py": "def f():\n    \"\"\"Summary.\n\n    Desc.\n    \\\"\"\"\"\n\n\ndef g():\n"
                                           "    \"\"\"Summary.\n\n    \\\\\"\"\"\n\n\ndef h():\n    '''Summary.\n\n"
                                           "    Desc.\\''''\n\n\ndef i():\n    \"\"\"Summary.\n\n    Desc.\n\t\"\"\"\n"
                                           "\n\ndef j():\n    \"\"\"Summary.\n    Desc.\n    \\\n    \"\"\"\n",
    # D212: A backslash continuation right after the opening quotes means the first line is not just the quotes; a docstring whose only text is a backslash is still reported.
    "D212-backslash-after-opener.py": "def f():\n    \"\"\"\\\n\"\"\"\n\n\ndef g():\n    \"\"\"\n\\\n    \"\"\"\n\n\n"
                                      "def h():\n    \"\"\"Summary.\n\\\n    \"\"\"\n\n\ndef i():\n    \"\"\"\\\n"
                                      "    \\\n    \"\"\"\n",
    # D215: Indentation is the opening line's, not the closing quotes': a parenthesized docstring counts its parenthesis, over-indented closing quotes do not hide an over-indented underline, and a header right after the previous underline is still a section.
    "D215-exact-docstring-indentation.py": "def f():\n    (\"\"\"Summary.\n\n    Returns\n     -------\n    int\n"
                                           "    \"\"\")\n\n\ndef g():\n    \"\"\"Summary.\n\n    Returns\n"
                                           "        -------\n        int\n        \"\"\"\n\n\nclass A:\n"
                                           "    \"\"\"Summary.\n\n    Yields\n    ------\n    Raises\n     ------\n"
                                           "    \"\"\"\n",
    # D300: Escape-only bodies are not empty, escaped triple quotes count as triple quotes in the text, and both triples present silences the rule.
    "D300-escapes-and-triple-content.py": "'\\t'\n\n\ndef a():\n    '\\\\'\n\n\ndef b():\n    \"\\\"\"\n\n\ndef c():\n"
                                          "    '\\n'\n\n\ndef d():\n    ' \\\n'\n\n\ndef e():\n    \"a\\\"\\\"\\\"b\"\n"
                                          "\n\ndef f():\n    '''a\\\"\"\"b'''\n\n\ndef g():\n    \"a'''b\"\n\n\n"
                                          "def h():\n    'a\"\"\"b\\'\\'\\'c'\n\n\ndef i():\n    \"\\x41\"\n",
    # D301: The left-to-right backslash scan: escaped triples in both spellings, partial escaped triples, unicode escapes, a multibyte escaped char, u/R/b prefixes and non-docstring strings.
    "D301-backslash-scan.py": "\"\"\"Module \\d doc.\"\"\"\n\n\ndef a():\n    \"\\\\\"\n\n\ndef b():\n    \"a\\\"b\"\n"
                              "\n\ndef c():\n    \"\"\"a\\\"\"\" b\"\"\"\n\n\ndef d():\n    '''a\\''' b'''\n\n\n"
                              "def e():\n    \"\"\"a\\\"\\\"\\\" b\"\"\"\n\n\ndef f():\n"
                              "    \"\\N{EM DASH} ok é \\U0001F600\"\n\n\ndef g():\n    \"é then \\d\"\n\n\ndef h():\n"
                              "    \"\\é\"\n\n\ndef i():\n    u\"\\d\"\n\n\ndef j():\n    R\"\\d\"\n\n\ndef k():\n"
                              "    \"\"\"a\\\"\" b\"\"\"\n\n\ndef l():\n    \"\"\"a\\\"\\\"x\"\"\"\n\n\ndef m():\n"
                              "    \"\"\"line \\\ncontinued\"\"\"\n\n\ndef n():\n    \"\\x41\"\n\n\ndef o():\n"
                              "    'a\\'b'\n\n\ndef p():\n    \"\"\"a\\\"\\\"\"\"\"\n\n\ndef q():\n    rb\"\\d\"\n\n\n"
                              "def r():\n    b\"\\d\"\n\n\ndef s():\n    x = 1\n    \"\\d\"\n\n\ndef t():\n"
                              "    \"\\d\" \"x\"\n\n\ndef u():\n    (\"\\d\")\n\n\ndef v():\n    \"\\\\\\\\\"\n\n\n"
                              "def w():\n    \"\"\"\\'\\'\\' tail\"\"\"\n\n\ndef x():\n    \"\"\"\\'\\'\"\"\"\n\n\n"
                              "def y():\n    \"\\\n\"\n",
    # D400: a lone carriage return ends a line (blank line and underline after a lone CR), next to the same summaries with LF breaks
    "D400-lone-cr-line-breaks.py": "def f():\n    \"\"\"Return x.\r\r    more\"\"\"\n\n\ndef g():\n"
                                   "    \"\"\"Return x\r    ===\r    more.\"\"\"\n\n\ndef h():\n    \"\"\"Return x\n"
                                   "    more.\"\"\"\n\n\ndef k():\n    \"\"\"Return x.\n\n    more\"\"\"\n",
    # D401: plurals of blacklisted nouns whose stem is a verb's (calculations, references, authorities) are reported; utilities, apples and readies are not
    "D401-blacklist-plurals-and-stem-lookalikes.py": "def f0():\n    \"\"\"Calculations for x.\"\"\"\n\n\ndef f1():\n"
                                                     "    \"\"\"Collections of x.\"\"\"\n\n\ndef f2():\n"
                                                     "    \"\"\"Implementations of x.\"\"\"\n\n\ndef f3():\n"
                                                     "    \"\"\"References to x.\"\"\"\n\n\ndef f4():\n"
                                                     "    \"\"\"Authorities for x.\"\"\"\n\n\ndef f5():\n"
                                                     "    \"\"\"Utilities for x.\"\"\"\n\n\ndef f6():\n"
                                                     "    \"\"\"Apples are red.\"\"\"\n\n\ndef f7():\n"
                                                     "    \"\"\"Readies x.\"\"\"\n\n\ndef f8():\n"
                                                     "    \"\"\"Validities of x.\"\"\"\n\n\ndef f9():\n"
                                                     "    \"\"\"Normalities x.\"\"\"\n",
    # D402: the name as the 14th and 15th call-like word of the line, past the earlier 14-candidate limit
    "D402-name-after-many-calls.py": "def foo():\n"
                                     "    \"\"\"a(1) b(2) c(3) d(4) e(5) f(6) g(7) h(8) i(9) j(10) k(11) l(12) m(13) foo(14)\"\"\"\n"
                                     "\n\ndef bar():\n"
                                     "    \"\"\"a(1) b(2) c(3) d(4) e(5) f(6) g(7) h(8) i(9) j(10) k(11) l(12) m(13) n(14) bar(15)\"\"\"\n",
    # D403: single letters, trailing and inner punctuation, curly apostrophe, ligature, and Unicode spaces ending the first word agree with Ruff
    "D403-word-boundaries-and-punctuation.py": "def f0():\n    \"\"\"x\"\"\"\n\n\ndef f1():\n    \"\"\"x.\"\"\"\n\n\n"
                                               "def f2():\n    \"\"\"iPhone is\"\"\"\n\n\ndef f3():\n"
                                               "    \"\"\"a's\"\"\"\n\n\ndef f4():\n    \"\"\"foo,\"\"\"\n\n\n"
                                               "def f5():\n    \"\"\"foo:\"\"\"\n\n\ndef f6():\n    \"\"\"foo’s\"\"\"\n"
                                               "\n\ndef f7():\n    \"\"\"ﬁx it\"\"\"\n\n\ndef f8():\n"
                                               "    \"\"\"foo?.\"\"\"\n\n\ndef f9():\n    \"\"\"foo...bar\"\"\"\n\n\n"
                                               "def f10():\n    \"\"\"foo...\"\"\"\n\n\ndef f11():\n    \"\"\"foo.\n"
                                               "    bar\"\"\"\n\n\ndef f12():\n    \"\"\"foo!?!\"\"\"\n\n\ndef f13():\n"
                                               "    \"\"\"foo'\"\"\"\n\n\ndef f14():\n    \"\"\"é\"\"\"\n\n\n"
                                               "def f15():\n    \"\"\"foo bar\"\"\"\n\n\ndef f16():\n"
                                               "    \"\"\"foo bar\"\"\"\n\n\ndef f17():\n    \"\"\" foo\"\"\"\n\n\n"
                                               "def f18():\n    \"\"\"foo(x)\"\"\"\n\n\ndef f19():\n"
                                               "    \"\"\"foo\tbar\"\"\"\n\n\ndef f20():\n    \"\"\"fooBar baz\"\"\"\n"
                                               "\n\ndef f21():\n    \"\"\"_foo\"\"\"\n\n\ndef f22():\n"
                                               "    \"\"\"1abc\"\"\"\n\n\ndef f23():\n    \"\"\"foo_bar\"\"\"\n\n\n"
                                               "def f24():\n    \"\"\"foo-bar\"\"\"\n",
    # D404: punctuation inside the word, no-break and ideographic spaces, fullwidth letters and dotted capital I agree with Ruff
    "D404-joined-and-non-ascii-first-pieces.py": "def f0():\n    \"\"\"Th-is is\"\"\"\n\n\ndef f1():\n"
                                                 "    \"\"\"T.H.I.S is\"\"\"\n\n\ndef f2():\n"
                                                 "    \"\"\"This's it\"\"\"\n\n\ndef f3():\n    \"\"\"This2 is\"\"\"\n"
                                                 "\n\ndef f4():\n    \"\"\"THİS is\"\"\"\n\n\ndef f5():\n"
                                                 "    \"\"\"thiſ is\"\"\"\n\n\ndef f6():\n    \"\"\"Ｔｈｉｓ is\"\"\"\n\n\n"
                                                 "def f7():\n    \"\"\"This is\"\"\"\n\n\ndef f8():\n"
                                                 "    \"\"\" This is\"\"\"\n\n\ndef f9():\n    \"\"\"This　is\"\"\"\n\n"
                                                 "\ndef f10():\n    \"\"\"This,is a\"\"\"\n\n\ndef f11():\n"
                                                 "    \"\"\"This_is a\"\"\"\n\n\ndef f12():\n    \"\"\"(this)\"\"\"\n\n"
                                                 "\ndef f13():\n    \"\"\"This\n\nmore\"\"\"\n",
    # D405: a lower-case first section right after a sentence-ending line is a section when no section precedes it, unlike later ones
    "D405-lowercase-first-section-after-sentence.py": "def f():\n    \"\"\"Do it.\n    returns:\n        int.\n"
                                                      "    \"\"\"\n\n\ndef g(x):\n    \"\"\"Do it.\n\n"
                                                      "    The value, see below.\n    parameters\n    x : int\n"
                                                      "    \"\"\"\n\n\ndef h():\n    \"\"\"Do it;\n    raises\n"
                                                      "    ------\n    E\n    \"\"\"\n",
    # D406: CRLF line endings: the carriage return after a section name is not a trailing suffix
    "D406-crlf-line-endings.py": "def f(x):\r\n    \"\"\"Do it.\r\n\r\n    Parameters\r\n    ----------\r\n"
                                 "    x : int\r\n        The x.\r\n\r\n    returns\r\n    -----\r\n    int\r\n"
                                 "    \"\"\"\r\n\r\n\r\ndef g(x):\r\n    \"\"\"Do it.\r\n\r\n    Args:\r\n"
                                 "        x: thing.\r\n\r\n    Returns:\r\n        value.\r\n    \"\"\"\r\n",
    # D407: one NumPy-only and two Google-only sections make the docstring Google-style, so no underline is required
    "D407-style-by-section-counts.py": "def f(x):\n    \"\"\"Do it.\n\n    Receives\n    --------\n    x\n\n    Note\n"
                                       "    ----\n    a\n\n    Example\n    -------\n    b\n\n    Returns:\n    ---\n"
                                       "    c\n\n    Raises\n    E\n    \"\"\"\n",
    # D408: one and two blank lines between header and dashes, with NumPy-only and shared names
    "D408-blank-lines-before-dashes.py": "def f(x):\n    \"\"\"Do it.\n\n    Parameters\n\n    ----------\n"
                                         "    x : int\n\n    Returns\n\n\n    ---\n    int\n    \"\"\"\n",
    # D409: over-indented, tab-indented single-dash and over-long underlines
    "D409-indented-tab-and-long-underlines.py": "def f(x):\n    \"\"\"Do it.\n\n    Returns\n        ---------\n"
                                                "    int\n\n    Raises\n\t-\n    E\n\n    Notes\n"
                                                "    ----------------------------------------\n    n\n    \"\"\"\n",
    # D410: an underlined header after unpunctuated text is the reported section, and a lowercase section after a blank line resets the chain (the earlier port reported Returns instead)
    "D410-underlined-after-plain-text.py": "def f():\n    \"\"\"Compute the result.\n\n    Result\n    Returns\n"
                                           "    -------\n    int\n    Raises\n    ------\n    ValueError\n    \"\"\"\n"
                                           "\n\n\ndef g():\n    \"\"\"Do a thing.\n\n    Returns:\n        x.\n\n"
                                           "    notes:\n        n.\n    Raises:\n        y.\n    \"\"\"\n",
    # D411: a lowercase header right after the summary is a section when it is the first one; later lowercase headers after punctuation are not
    "D411-lowercase-first-section.py": "def f(x):\n    \"\"\"Do a thing.\n    returns:\n        the value.\n"
                                       "    raises:\n        nothing\n    \"\"\"\n\n\ndef g(x):\n"
                                       "    \"\"\"Do a thing.\n    args:\n        x: the x.\n    Returns:\n        y\n"
                                       "    \"\"\"\n",
    # D412: a lowercase first section after the summary with blank lines before content is reported, but not when the blank lines lead to the next header
    "D412-lowercase-first-section.py": "def f():\n    \"\"\"Do a thing.\n    returns:\n\n        int.\n    Raises:\n"
                                       "        x.\n    \"\"\"\n\n\ndef g():\n    \"\"\"Do a thing.\n    returns:\n\n"
                                       "    Raises:\n\n        x.\n    \"\"\"\n",
    # D413: a lowercase only section right after the summary, and one after a blank line in a method (indent 8), are the last section
    "D413-lowercase-first-section.py": "def f():\n    \"\"\"Do a thing.\n    returns:\n        int.\n    \"\"\"\n\n\n"
                                       "class K:\n    def m(self):\n        \"\"\"Do a thing.\n\n        returns:\n"
                                       "            int.\n        \"\"\"\n",
    # D414: an empty lowercase first section followed by one blank line and the next header, and one in a method at indent 8
    "D414-lowercase-first-section.py": "def f():\n    \"\"\"Do a thing.\n    returns:\n\n    Raises:\n        x.\n"
                                       "    \"\"\"\n\n\nclass K:\n    def m(self):\n        \"\"\"Do a thing.\n\n"
                                       "        returns:\n\n        \"\"\"\n",
    # D415: Pins lone carriage-return line endings, which Ruff's universal newlines split on (2 findings: a one-line summary and a summary followed by an underline line); Ruff's tests only use \n.
    "D415-lone-cr-line-endings.py": "def a():\r    \"\"\"Summary\"\"\"\r\r\rdef b():\r    \"\"\"Summary.\r\r    Detail\"\"\"\r\r\rdef c():\r    \"\"\"Summary\r    more.\r    \"\"\"\r\r\rdef d():\r    \"\"\"\r    Returns\r    \"\"\"\r\r\rdef e():\r    \"\"\"Summary\r    ---\r    more.\"\"\"\r\n",
    # D416: Pins Ruff's subsection rule for lowercase headers: `args` as the first section after a punctuated summary and `returns` after a blank line are reported, `returns` after `x: y.` and `error` in a list under `Args:` are not (2 findings, identical to live Ruff); Ruff's own D.py has no D416 finding.
    "D416-lowercase-subsection-words.py": "def f(x):\n    \"\"\"Summary.\n    args\n        x: y.\n    \"\"\"\n\n\n"
                                          "def g(x):\n    \"\"\"Summary\n    args\n        x: y.\n    \"\"\"\n\n\n"
                                          "def h(x):\n    \"\"\"Summary.\n\n    Args:\n        x: y.\n    returns\n"
                                          "        z.\n    \"\"\"\n\n\ndef i(x):\n    \"\"\"Summary.\n\n    Args:\n"
                                          "        x: y.\n\n    returns\n        z.\n    \"\"\"\n\n\n"
                                          "def log(level, msg):\n    \"\"\"Log a message.\n\n    Args:\n"
                                          "        level: Log level, one of:\n            debug,\n            info,\n"
                                          "            warning,\n            error\n        msg: The message.\n\n"
                                          "    Returns:\n        None.\n    \"\"\"\n",
    # D418: Pins the import forms through which `overload` resolves (module attribute, module alias, from-import alias, typing_extensions, _typeshed); Ruff's own tests only use a plain `from typing import overload`. 7 findings, identical to live Ruff.
    "D418-overload-import-forms.py": "import typing\nimport typing as t\nimport typing_extensions\n"
                                     "import typing_extensions as te\nfrom typing import overload as ov\n"
                                     "from typing_extensions import overload as ov2\n"
                                     "from _typeshed import overload as ov3\n\n\n@typing.overload\n"
                                     "def a(x: int) -> int:\n    \"\"\"Doc.\"\"\"\n\n\n@t.overload\n"
                                     "def b(x: int) -> int:\n    \"\"\"Doc.\"\"\"\n\n\n@typing_extensions.overload\n"
                                     "def c(x: int) -> int:\n    \"\"\"Doc.\"\"\"\n\n\n@te.overload\n"
                                     "def d(x: int) -> int:\n    \"\"\"Doc.\"\"\"\n\n\n@ov\ndef e(x: int) -> int:\n"
                                     "    \"\"\"Doc.\"\"\"\n\n\n@ov2\ndef g(x: int) -> int:\n    \"\"\"Doc.\"\"\"\n\n\n"
                                     "@ov3\ndef h(x: int) -> int:\n    \"\"\"Doc.\"\"\"\n",
    # D419: Pins parenthesized docstrings (also around a comment and across lines) and that implicit concatenations are skipped, none of which Ruff's own D.py covers: 5 findings (c, d, f, g, h), identical to live Ruff.
    "D419-concatenated-and-parenthesized.py": "def a():\n    \"\" \"\"\n\n\ndef b():\n    \"\"\"\"\"\" \"\"\"  \"\"\"\n"
                                              "\n\ndef c():\n    (\"\")\n\n\ndef d():\n    ((\"  \"))\n\n\ndef e():\n"
                                              "    (\"\" \"\")\n\n\ndef f():\n    (  # comment\n        \"\"\n    )\n\n"
                                              "\ndef g():\n    (\"\"\"\n    \"\"\")\n\n\ndef h():\n    (\"\") \n",
    # E401: Pins statement spans across backslash continuations. A line continuation before a single name is not a second name. A `# noqa` on the last physical line suppresses a leading-comma continuation. Also covers a `;` line and a continued line at column 0. Ruff's fixture only has single-line cases.
    "E401-backslash-continuation.py": "import \\\n    os, \\\n    sys\nimport \\\n    a\nimport json \\\n"
                                      "    , re  # noqa: E401\nx = 1; import b, \\\n    c\nimport d,\\\ne as f\n",
    # EM101: A raised call that follows `raise` across a backslash continuation (bare or parenthesized) is still the raised exception; the old follows-raise check missed it.
    "EM101-backslash-continuation-after-raise.py": "def a():\n    raise \\\n"
                                                   "        ValueError(\"continued after raise\")\n\n\ndef b(x):\n"
                                                   "    raise \\\n        ValueError(f\"continued {x}\")\n\n\n"
                                                   "def c(x):\n    raise \\\n        ValueError(\"{}\".format(x))\n\n\n"
                                                   "def d(x):\n    raise \\\n"
                                                   "        (ValueError(\"parens after continuation\"))\n\n\n"
                                                   "def e(x):\n    raise ValueError(\n        \"x\") \\\n"
                                                   "        from x\n",
    # EM102: An f-string in a call raised across a backslash continuation after `raise` is reported; the old follows-raise check missed it.
    "EM102-backslash-continuation-after-raise.py": "def a():\n    raise \\\n"
                                                   "        ValueError(\"continued after raise\")\n\n\ndef b(x):\n"
                                                   "    raise \\\n        ValueError(f\"continued {x}\")\n\n\n"
                                                   "def c(x):\n    raise \\\n        ValueError(\"{}\".format(x))\n\n\n"
                                                   "def d(x):\n    raise \\\n"
                                                   "        (ValueError(\"parens after continuation\"))\n\n\n"
                                                   "def e(x):\n    raise ValueError(\n        \"x\") \\\n"
                                                   "        from x\n",
    # EM103: A parenthesized `.format` callee is still a format call on a literal (Ruff unwraps the parentheses), and the span starts at the callee's opening parenthesis; format_map and a subscript receiver stay silent.
    "EM103-parenthesized-format-callee.py": "def a(x):\n    raise ValueError((\"{}\".format)(x))\n\n\ndef b(x):\n"
                                            "    raise ValueError((\"{}\"\n                      .format)(x))\n\n\n"
                                            "def c(x):\n    raise ValueError(((\"{}\").format)(x))\n\n\ndef d(x):\n"
                                            "    raise ValueError((((\"{}\")).format(x)))\n\n\ndef e(x):\n"
                                            "    raise ValueError(\"{}\".format_map(x))\n\n\ndef f(x):\n"
                                            "    raise ValueError(\"{}\"[0].format(x))\n",
    # ERA001: Pins tokenizer and parse details Ruff's fixture does not cover: None as a name in the consecutive-name check, prefix-only allowlist, Unicode digits after '#', call argument order, '=' and ':' only where the grammar allows them, numeric underscores, f-string fields, lambda defaults, type parameters, lazy import and bare (yield).
    "ERA001-parse-shapes.py": "# else: None foo\n# ruff: disabled = 1\n# x = 1  #٣\n# f(x=1, 2)\n# x = (a = 1)\n"
                              "# x = [1:2]\n# x = 1__0\n# x = f\"{\"\n# x = f\"{d[\"k\"]}\"\n# f = lambda x=[]: x\n"
                              "# def f[T](x: T) -> T: return x\n# lazy import os\n# x = (yield)\n",
    # FA100: A parenthesized `if (TYPE_CHECKING):` test still makes the block a typing context, so Optional/Union with a quoted element is reported there (Ruff at py39 reports lines 5 and 7 only); silent at the default target.
    "FA100-parenthesized-type-checking.py": "from typing import TYPE_CHECKING, Optional, Union\nimport typing\n\n"
                                            "if (TYPE_CHECKING):\n    a: Optional[\"Foo\"]\n"
                                            "if ((typing.TYPE_CHECKING)):\n    b: Union[int, \"Foo\"]\n"
                                            "if not TYPE_CHECKING:\n    c: Optional[\"Foo\"]\n"
                                            "if TYPE_CHECKING and True:\n    d: Optional[\"Foo\"]\n",
    # FA102: Union chains starting with a subscript, where the grammar mis-groups the operands: Ruff at py37 reports every inner `|` prefix span (34 findings), which the regex prefix clauses reproduce; silent at the default target.
    "FA102-union-chain-prefix-spans.py": "a1: list[int] | str | None = None\na3: list[int] | None | str = None\n"
                                         "a7: list[int] | str | float | None = None\n"
                                         "a12: list[int] | None | str | bytes = None\n"
                                         "b1: list[int] | None | dict[str, int] = None\n"
                                         "b2: dict[str, list[int]] | (str | None) | None = None\n"
                                         "b3: list[int] | str | dict[str, int] | None = None\nb4: (\n"
                                         "    list[int]  # c\n    | str\n    | None\n) = None\n"
                                         "b5: list[int] | Literal[\"a|b\", 'c'] | None = None\n",
    # FAST001: Pins type identity through parentheses, trailing commas and an empty tuple slice (`List[(Item)]`, `Dict[str, Item,]`, `List[Item,]` vs `List[Item]`, `Tuple[()]`), which Ruff's FAST001 fixture never spells.
    "FAST001-parentheses-and-trailing-commas.py": "from typing import List, Dict, Tuple\nfrom fastapi import FastAPI\n"
                                                  "\napp = FastAPI()\n\n\nclass Item: ...\n\n\n"
                                                  "@app.get(\"/a\", response_model=List[(Item)])\n"
                                                  "def a() -> List[Item]: ...\n\n\n"
                                                  "@app.get(\"/b\", response_model=Dict[str, Item,])\n"
                                                  "def b() -> Dict[str, Item]: ...\n\n\n"
                                                  "@app.get(\"/c\", response_model=(Item))\ndef c() -> Item: ...\n\n\n"
                                                  "@app.get(\"/d\", response_model=Item)\ndef d() -> (Item): ...\n\n\n"
                                                  "@app.get(\"/e\", response_model=List[Item,])\n"
                                                  "def e() -> List[Item]: ...\n\n\n"
                                                  "@app.get(\"/f\", response_model=Tuple[()])\n"
                                                  "def f() -> Tuple[()]: ...\n\n\n"
                                                  "@app.get(\"/g\", response_model=List[List[Item]])\n"
                                                  "def g() -> List[List[(Item)]]: ...\n\n\n"
                                                  "@app.get(\"/h\", response_model=Dict[str, Item])\n"
                                                  "def h() -> Dict[(str), Item]: ...\n",
    # FAST002: Pins marker resolution through an enclosing function's parameter, a class-body rebinding and a later module rebinding seen by a nested route (Ruff's deferred function bodies), none of which Ruff's FAST002 fixtures exercise.
    "FAST002-marker-shadowing-and-scopes.py": "from fastapi import FastAPI, Query, Depends\n\napp = FastAPI()\n\n\n"
                                              "def outer(Query):\n    @app.get(\"/a\")\n"
                                              "    def a(q: int = Query(None), d: int = Depends(dep)): ...\n\n\n"
                                              "class K:\n    Depends = staticmethod(lambda f: f)\n\n"
                                              "    @app.get(\"/b\")\n"
                                              "    def b(self, d: int = Depends(dep), q: int = Query()): ...\n\n\n"
                                              "@app.get(\"/c\")\ndef c(q: int = Query(None)): ...\n\n\nQuery = None\n\n"
                                              "\ndef later():\n    @app.get(\"/d\")\n"
                                              "    def d(q: int = Query(None), e: int = Depends(dep)): ...\n",
    # FAST003: Pins Annotated element positions across a comment, the count of resolvable Depends (a lambda one does not count), the default Depends taking precedence over Annotated metadata, and __init__ skipping its first non-positional-only parameter; Ruff's FAST003 fixture covers none of these.
    "FAST003-annotated-dependency-resolution.py": "from typing import Annotated\n"
                                                  "from fastapi import FastAPI, Depends, Path\n\napp = FastAPI()\n\n\n"
                                                  "def with_id(id): ...\ndef without(other): ...\n\n\nclass Init:\n"
                                                  "    def __init__(self, /, id, other): ...\n\n\n"
                                                  "@app.get(\"/a/{id}\")\ndef a(x: Annotated[str,  # comment\n"
                                                  "    Path(alias=\"id\")]): ...\n\n\n@app.get(\"/b/{id}\")\n"
                                                  "def b(x: Annotated[int, Depends(lambda: 0), Depends(without)]): ...\n"
                                                  "\n\n@app.get(\"/c/{id}\")\n"
                                                  "def c(x: Annotated[int, Depends(without), Depends(without)]): ...\n"
                                                  "\n\n@app.get(\"/d/{id}\")\n"
                                                  "def d(x: Annotated[int, Depends(with_id)] = Depends(without)): ...\n"
                                                  "\n\n@app.get(\"/e/{id}\")\n"
                                                  "def e(x: Annotated[int, Depends(with_id)] = Depends(lambda: 1)): ...\n"
                                                  "\n\n@app.get(\"/f/{id}\")\ndef f(x=Depends(Init)): ...\n",
    # FBT001: Pins how Union and Optional slices count members, including tuple versus single item, parenthesized and trailing-comma slices, typing and typing_extensions attribute heads, nested generics, and Union or Optional mixed with `|` (which tree-sitter parses as union_type over generic_type).
    "FBT001-union-optional-slices.py": "from typing import Optional, Union\nimport typing\n"
                                       "import typing_extensions as te\n\n\n"
                                       "def f1(a: Union[bool, int], b: Union[bool], c: Union[(bool, int)], d: Union[bool,], e: Union[(bool)]):\n"
                                       "    pass\n\n\n"
                                       "def f2(a: Optional[bool], b: Optional[bool, int], c: Optional[(bool)], d: Optional[(bool,)], e: Optional[\"bool\"]):\n"
                                       "    pass\n\n\n"
                                       "def f3(a: typing.Union[int, bool], b: typing.Optional[bool], c: te.Optional[bool], d: te.Union[str, \"bool\"]):\n"
                                       "    pass\n\n\n"
                                       "def f4(a: int | bool, b: (bool | None), c: None | int | bool, d: int | str, e: bool & int, f: Union[int, Optional[bool]]):\n"
                                       "    pass\n\n\n"
                                       "def f5(a: Union[(bool, int), str], b: Union[((bool, int))], c: list[bool], d: dict[str, bool], e: \"bool | None\"):\n"
                                       "    pass\n\n\n"
                                       "def f6(a: Union[bool, int] | None, b: None | Optional[bool], c: typing.Optional[bool] | int, d: Optional[Optional[int]], e: Union[Union[bool, int]]):\n"
                                       "    pass\n",
    # FBT002: Pins the setter exemption: only an `@<own name>.setter` decorator counts, with the object read through parentheses. Another object's setter, a three-part chain, a parenthesized attribute object, a called setter, `@x.setter` on a function named setter, and a getter still report.
    "FBT002-setter-decorator-forms.py": "class C:\n    @property\n    def foo(self) -> bool:\n        return True\n\n"
                                        "    @foo.setter\n    def foo(self, value: bool = False) -> None:\n"
                                        "        pass\n\n    @(foo).setter\n"
                                        "    def foo(self, value: bool = True) -> None:\n        pass\n\n"
                                        "    @((foo)).setter\n    def foo(self, value: bool = True) -> None:\n"
                                        "        pass\n\n    @bar.setter\n"
                                        "    def foo(self, value: bool = False) -> None:\n        pass\n\n"
                                        "    @self.foo.setter\n    def foo(self, value: bool = True) -> None:\n"
                                        "        pass\n\n    @(foo.bar).setter\n"
                                        "    def foo(self, value: bool = True) -> None:\n        pass\n\n"
                                        "    @foo.setter()\n    def foo(self, value: bool = True) -> None:\n"
                                        "        pass\n\n    @x.setter\n"
                                        "    def setter(self, value: bool = True) -> None:\n        pass\n\n"
                                        "    @foo.getter\n    def foo(self, value: bool = True) -> None:\n"
                                        "        pass\n",
    # FBT003: Pins calls inside type-definition strings: exact inner spans, whole-string spans for escaped and concatenated strings, nested and cast/TypeVar strings, silence in Literal and plain strings, and the name, setter and multiprocessing.Value exemptions inside the parsed string.
    "FBT003-calls-in-string-annotations.py": "from multiprocessing import Value\n"
                                             "from typing import Annotated, List, Literal, TypeVar, cast\n\n\n"
                                             "def g(*a):\n    return a\n\n\na: \"g(True, False)\"\n"
                                             "b: \"Annotated[int, g(\\x54rue)]\"\nc: \"Annotated[int, \" \"g(True)]\"\n"
                                             "d: \"List['g(True)']\"\ne: \"Literal['g(True)']\"\n"
                                             "f = cast(\"Annotated[int, g(True)]\", 1)\n"
                                             "T = TypeVar(\"T\", bound=\"Annotated[int, g(False)]\")\n"
                                             "h: \"Annotated[int, obj.get(True), obj.set_x(False), g((True))]\"\n"
                                             "i: \"\"\"Annotated[int,\n    g(True)]\"\"\"\n"
                                             "j = \"Annotated[int, g(True)]\"\nk: List[\"Annotated[int, g(True)]\"]\n"
                                             "v: \"Annotated[int, Value('b', True, False)]\"\n",
    # FIX001: Only the first tag across `#` segments counts: FIXME after TODO is silent, and FIXME after non-tag segments reports at the later segment. The scan runs past a noqa segment, but a blanket `# noqa` still suppresses the finding while `# noqa:E501` does not.
    "FIX001-first-directive-across-hash-segments.py": "#  #  FIXME\n# TODO # FIXME\n# x # y # z # FIXME\n"
                                                      "# fixmes #fixme\n# noqa # FIXME\n# noqa:E501 # FIXME\n"
                                                      "# FIXME # noqa: FIX001 # TODO\n",
    # FIX002: Unicode letters and numerics extend the word and silence it, `→` ends it, and U+3000 and U+0085 count as whitespace (Rust's White_Space) while U+FEFF does not, with byte spans after multibyte characters.
    "FIX002-unicode-word-boundaries.py": "# éTODO\n# TODOé\n# TODO→ arrow\n# TODO²\n#　TODO ideographic\n"
                                         "#TODO next line\n#﻿TODO byte order mark\n# 日本 TODO\n# 日本 # TODO\n",
    # FIX003: The tag must be a whole alphanumeric word in any case: a longer word or trailing digits are silent, a hyphen ends it, and a non-tag first segment hands over to the next `#` segment.
    "FIX003-tag-word-boundaries.py": "# XXXX\n# xXx\n# XXX123\n# xxx-yyy\n# xxxx # xxx\n",
    # FIX004: A longer word is silent, while a hyphen or underscore (not alphanumeric) ends the tag, and a non-tag first word hands over to the next `#` segment.
    "FIX004-tag-word-boundaries.py": "# HACKY\n# hack-around\n# HACK_\n# hacker # HACK\n",
    # FLY002: Pins join calls inside strings Ruff parses as type definitions (annotations, List slices, cast arguments, concatenated and escaped annotations; not Literal or plain strings), plus the raw-string bail that counts the joiner only with two or more elements; live Ruff reports 8.
    "FLY002-string-annotations-and-one-element-raw-bail.py": "from typing import List, Literal, cast\na = b = None\n"
                                                             "x1: \"''.join([a, b]) | '-'.join((a, 'x'))\" = None\n"
                                                             "x2: List[\"''.join([a, b])\"] = []\n"
                                                             "x3 = cast(\"''.join(['p', 'q'])\", a)\n"
                                                             "x4: Literal[\"''.join([a, b])\"] = None\n"
                                                             "x5 = \"''.join([a, b])\"\n"
                                                             "x6: \"''.join(\" \"[a, b])\" = None\n"
                                                             "x7: \"''.join([a, \\\"'\\\", '\\\"'])\" = None\n"
                                                             "y1 = '\\n'.join([r'a\"\"\"' \"'''\"])\n"
                                                             "y2 = \"'''\\n\".join([r'a\"\"\"'])\n"
                                                             "y3 = \"'''\\n\".join([r'a\"\"\"', 'b'])\n"
                                                             "y4 = '\"\"\"'.join([r'''a\n''' \"'''\"])\n"
                                                             "y5 = '\"\"\"'.join([r'''a\n''', \"'''\"])\n",
    # G001: Pins which receivers count as literals (bytes, numbers, None/True/False/Ellipsis, implicit concatenation) and which do not (f-string parts, unary minus, names, format_map, method chained after format).
    "G001-literal-receivers.py": "import logging\n\nx = 1\nlogging.info(b\"{}\".format(x))\n"
                                 "logging.info(1 .format(x))\nlogging.info(None.format(x))\n"
                                 "logging.info(True.format(x))\nlogging.info(False.format(x))\n"
                                 "logging.info(....format(x))\nlogging.info(1.5.format(x))\n"
                                 "logging.info(1j.format(x))\nlogging.info(\"a\" \"b{}\".format(x))\n"
                                 "logging.info(f\"{x}\".format(x))\nlogging.info(x.format(x))\n"
                                 "logging.info(\"a\".join(x))\nlogging.info((\"a\" f\"b\").format(x))\n"
                                 "logging.info(-1 .format(x))\nlogging.info(str.format(\"{}\", x))\n"
                                 "logging.info(u\"{}\".format(x))\nlogging.info(r\"{}\".format(x))\n"
                                 "logging.info(\"\"\"{}\n\"\"\".format(x))\nlogging.info(Rb'{}'.format(x))\n"
                                 "logging.info(\"{}\".format)\nlogging.info(\"{}\".format(x).upper())\n"
                                 "logging.info(\"{}\".format(x) + \"y\")\nlogging.info(0x10.format(x))\n"
                                 "logging.info(\"{}\".fmt(x))\nlogging.info(\"{}\".format_map({}))\n",
    # G002: Pins when an imported name stops being an import for the logger-name heuristic: conditional del and del in an except handler keep the import, del in a loop unbinds, an except-as name restores the shadowed import or is unbound, a bare annotation in a function falls through to the import, global keeps it, and nonlocal to an assignment does not.
    "G002-binding-forms.py": "from foo import log1, log2, log3, log4, log5, log6, log7\n\nx = 1\nif x:\n    del log1\n"
                             "log1.info(\"%s\" % x)\nfor _ in []:\n    del log2\nlog2.info(\"%s\" % x)\ntry:\n"
                             "    pass\nexcept Exception:\n    del log3\nlog3.info(\"%s\" % x)\ntry:\n    pass\n"
                             "except Exception as log4:\n    pass\nlog4.info(\"%s\" % x)\ntry:\n    pass\n"
                             "except Exception as log_err:\n    pass\nlog_err.info(\"%s\" % x)\n\n\ndef f():\n"
                             "    log5: int\n    log5.info(\"%s\" % x)\n\n\ndef g():\n    global log6\n"
                             "    log6.info(\"%s\" % x)\n\n\ndef h():\n    log7 = None\n\n    def inner():\n"
                             "        nonlocal log7\n        log7.info(\"%s\" % x)\n",
    # G003: Pins the scope of the logger-name heuristic: function bodies see the module's final binding (a later reassignment re-enables it, a later import disables it), class-body reads see the class assignment, and methods and inner comprehensions skip the class scope while a comprehension's first iterable does not.
    "G003-deferred-and-class-scope.py": "from foo import log1, log3\n\nx = 1\n\n\ndef f():\n"
                                        "    log1.info(\"a\" + str(x))\n\n\nlog1 = None\nlog2 = None\n\n\ndef g():\n"
                                        "    log2.info(\"a\" + str(x))\n\n\nfrom foo import log2\n\n\nclass C:\n"
                                        "    log3 = None\n    log3.info(\"a\" + str(x))\n\n    def m(self):\n"
                                        "        log3.info(\"a\" + str(x))\n\n"
                                        "    inner = [log3.info(\"a\" + str(x)) for _ in range(1)]\n"
                                        "    first = [y for y in log3.info(\"a\" + str(x))]\n",
    # G004: Pins the message slot: msg= beats the positional, a starred argument hides later positionals, .log reads index 1, **{'msg': ...} and generator arguments pass nothing.
    "G004-message-slot.py": "import logging\n\nx = 1\nargs = ()\nkw = {}\nlogging.info(*args)\n"
                            "logging.info(*args, \"a\" + str(x))\nlogging.log(*args, \"%s\" % x)\n"
                            "logging.log(10, *args)\nlogging.log(f\"{x}\")\nlogging.log(\"a\" + \"b\", 10)\n"
                            "logging.info(\"a\", \"b\" + str(x))\nlogging.info(\"a\" + str(x), msg=\"x\")\n"
                            "logging.info(x, msg=\"a\" + str(x))\nlogging.info(**{\"msg\": \"a\" + str(x)})\n"
                            "logging.log(level=10, msg=f\"{x}\")\nlogging.log(msg=\"%s\" % x)\n"
                            "logging.info(msg=\"{}\".format(x), extra={})\nlogging.info(\"{}\".format(x), *args)\n"
                            "logging.info(f\"{x}\", **kw)\nlogging.log(10, f\"{x}\", *args)\n"
                            "logging.log(10, msg=(\"a\" + \"b\"))\nlogging.info(f\"{x}\" for x in args)\n"
                            "logging.info()\nlogging.log(*args, msg=\"a\" + \"b\")\n",
    # G010: Pins that a class-scope import is visible to the class body but not to its methods, which fall to the name heuristic or stay unresolved.
    "G010-class-scope-visibility.py": "class C:\n    from foo import logger\n    logger.warn(\"a\")\n\n"
                                      "    def m(self):\n        logger.warn(\"b\")\n\n    LOG = make()\n\n"
                                      "    def n(self):\n        LOG.warn(\"c\")\n\n    from logging import warn\n\n"
                                      "    def p(self):\n        warn(\"d\")\n\n    warn(\"e\")\n",
    # G101: Pins that `dict(...)` read before a module-level `dict = ...` is still the builtin, while one after it or in a function body is not.
    "G101-builtin-dict-shadow-order.py": "import logging\nlogging.info(\"x\", extra=dict(name=1))\ndict = MyDict\n"
                                         "logging.info(\"x\", extra=dict(name=2))\n\n\ndef f():\n"
                                         "    logging.info(\"x\", extra=dict(msg=1))\n",
    # G201: Pins that class bodies clear the handler flag, lambdas and comprehensions in them included, while decorators, defaults and return annotations keep it, and one-line def and class bodies clear it too.
    "G201-handler-flag-cleared-by-class-body.py": "import logging\ntry:\n    pass\nexcept:\n    class K:\n"
                                                  "        f = lambda: logging.error(\"a\", exc_info=True)\n"
                                                  "        g = [logging.exception(i, exc_info=True) for i in x]\n\n"
                                                  "    @dec(lambda: logging.error(\"b\", exc_info=True))\n"
                                                  "    class L:\n        pass\n\n"
                                                  "    def h(x=lambda: logging.exception(\"c\", exc_info=True)):\n"
                                                  "        pass\n\n"
                                                  "    def k() -> logging.error(\"d\", exc_info=True):\n        pass\n"
                                                  "\n    def one(): logging.error(\"e\", exc_info=True)\n"
                                                  "    class Two: logging.exception(\"f\", exc_info=True)\n",
    # G202: Pins which exc_info values count: parenthesized `True`, and `sys.exc_info()` through any import form, alias or parenthesized callee and with any argument count. Other truthy values, a bare `sys.exc_info`, a conditional and a `**` spread do not count.
    "G202-exc-info-value-forms.py": "import logging\nimport sys\nimport sys as s\nfrom sys import exc_info as ei\n\n"
                                    "try:\n    pass\nexcept:\n    logging.error(\"a\", exc_info=(True))\n"
                                    "    logging.exception(\"a\", exc_info=(True))\n"
                                    "    logging.error(\"b\", exc_info=(sys.exc_info()))\n"
                                    "    logging.exception(\"b\", exc_info=(sys.exc_info()))\n"
                                    "    logging.error(\"c\", exc_info=s.exc_info())\n"
                                    "    logging.exception(\"c\", exc_info=s.exc_info())\n"
                                    "    logging.error(\"d\", exc_info=ei())\n"
                                    "    logging.exception(\"d\", exc_info=ei())\n"
                                    "    logging.error(\"e\", exc_info=1)\n    logging.exception(\"e\", exc_info=1)\n"
                                    "    logging.error(\"f\", exc_info=e)\n    logging.exception(\"f\", exc_info=e)\n"
                                    "    logging.error(\"g\", exc_info=None)\n"
                                    "    logging.exception(\"g\", exc_info=None)\n"
                                    "    logging.error(\"h\", exc_info=not False)\n"
                                    "    logging.exception(\"h\", exc_info=not False)\n"
                                    "    logging.error(\"i\", exc_info=sys.exc_info)\n"
                                    "    logging.exception(\"i\", exc_info=sys.exc_info)\n"
                                    "    logging.error(\"j\", exc_info=True if x else False)\n"
                                    "    logging.exception(\"j\", exc_info=True if x else False)\n"
                                    "    logging.error(\"k\", exc_info=(sys.exc_info)())\n"
                                    "    logging.exception(\"k\", exc_info=(sys.exc_info)())\n"
                                    "    logging.error(\"l\", exc_info=sys.exc_info(1))\n"
                                    "    logging.exception(\"l\", exc_info=sys.exc_info(1))\n"
                                    "    logging.error(\"m\", **{\"exc_info\": True})\n"
                                    "    logging.exception(\"m\", **{\"exc_info\": True})\n"
                                    "    logging.error(\"n\", True)\n    logging.exception(\"n\", True)\n",
    # ICN001: Pins NFKC-normalized qualified names on both the import and from-import side (reported when the source name differs from the alias), the from-import module-name position and relative imports staying silent, and conventional aliases on NFKC-spelled modules staying silent.
    "ICN001-non-ascii-module-spellings.py": "import ｐandas\nfrom matplotlib import ｐyplot\nimport numpy.ｔyping as nt\n"
                                            "import ｐandas as pd\nfrom ｍatplotlib import pyplot as plt\n"
                                            "from ｎumpy import array\nfrom ｍatplotlib.pyplot import plot\n"
                                            "from ｍatplotlib import (cm, pyplot as pp)\nfrom . import ｐandas\n"
                                            "import ｍatplotlib, os\n",
    # INT001: Pins f-strings in translation calls inside string type definitions: a plain annotation, an aliased import, nested `List[...]`, a `cast` argument and an escaped (complex) return annotation, reported at the inner argument or the whole string. Also pins three cases that are not reported: inside `Literal[...]`, in an f-string host, and as a second argument.
    "INT001-string-annotations.py": "from typing import List, Literal, TypeVar, cast\nimport gettext as g\n\na = 1\n"
                                    "x1: \"_(f'{a}')\" = 1\nx2: \"g.gettext(f'{a}' 'b')\" = 1\n"
                                    "x3: \"List[_(f'{a}')]\" = 1\nx4: \"Literal['_(f\\\"{a}\\\")']\" = 1\n"
                                    "x5: f\"_('{a}')\" = 1\nT = TypeVar(\"T\", bound=\"_(a, f'{a}')\")\n"
                                    "y = cast(\"ngettext(f'{a}', 'x', 1)\", 1)\n"
                                    "def f() -> \"_(\\\"%s\\\" f'{a}')\": ...\n",
    # INT002: Pins `format` calls in translation calls inside string type definitions: plain, nested in `List[...]` (whole-string range), a parenthesized receiver and a parameter annotation. Also pins four cases that are not reported: an object named `format`, an uncalled `.format`, `Literal[...]`, and a second argument in `cast`.
    "INT002-string-annotations.py": "from typing import List, Literal, cast\na = 1\nt = \"{}\"\n"
                                    "x1: \"_(t.format(a))\" = 1\nx2: \"List['gettext(t.format(a))']\" = 1\n"
                                    "x3: \"_((t).format(a))\" = 1\nx4: \"_(format.join(a))\" = 1\n"
                                    "x5: \"_(t.format)\" = 1\nx6: \"Literal['_(t.format(a))']\" = 1\n"
                                    "y = cast(\"_(a, t.format(a))\", 1)\n"
                                    "def f(p: \"ngettext(t.format(a), '', 1)\") -> None: ...\n",
    # ISC001: Every overlapping pair of a six-literal run; a run whose first gap breaks the line through a comment that contains quotes; an empty literal before a triple-quoted one; a literal with no space before the next; escaped braces in an f-string mid-run
    "ISC001-pairs-across-long-and-broken-runs.py": "x = \"a\" \"b\" \"c\" \"d\" \"e\" \"f\"\n"
                                                   "y = (\"a\"  # \"q\" \"r\"\n     \"b\" \"c\" \"d\" \"e\")\n"
                                                   "z = \"\" \"\"\"t\"\"\" ''\"u\"\nw = f\"{{\" \"a}\" \"b\"\n",
    # ISC003: Pins that only a line break between the left literal's end and the right literal's start counts (not one before the left literal inside its parentheses), that implicit-concatenation operands are read through parentheses, and that the kinds must match (bytes/str and t/str are not reported).
    "ISC003-parenthesized-operand-breaks.py": "_ = ((\n    \"a\") + \"b\")\n_ = ((\"a\"\n      ) + \"b\")\n"
                                              "_ = (\"a\" + (\n    \"b\"))\n_ = ((\"a\" \"b\") +\n     \"c\")\n"
                                              "_ = ((\"a\"\n      \"b\") + \"c\")\n_ = (b\"a\" +\n     \"b\")\n"
                                              "_ = (rb\"a\" +\n     Br\"b\")\n_ = (f\"a{x}\" +\n     U\"b\")\n"
                                              "_ = (t\"a\" +\n     t\"b\")\n_ = (t\"a\" +\n     \"b\")\n",
    # ISC004: Pins annotation subscripts (tree-sitter generic_type/type_parameter, not subscript) with multiple or trailing-comma indices vs a single index, the `print >> f, ...` tuple (tree-sitter print_statement), the subscripted-value exclusion with a comment before `[`, and the parenthesized with-items that are not a tuple.
    "ISC004-annotation-subscripts-and-print-chevron.py": "import sys\nfrom typing import Literal\n\n\n"
                                                         "def f(x: Literal[\"a\" \"b\", \"c\"], y: list[\"a\" \"b\"] = ()) -> tuple[\"a\" \"b\", int]: ...\n"
                                                         "\n\nv: Literal[\"a\" \"b\",] = \"ab\"\n"
                                                         "type T = dict[str, \"a\" \"b\"] | Literal[\"a\" \"b\", \"c\"]\n"
                                                         "print >> sys.stderr, \"a\" \"b\"\n"
                                                         "_ = (\"a\" \"b\"  # comment\n     [0, 1])\n"
                                                         "with (\"a\" \"b\", 1):\n    pass\n",
    # LOG004: Pins which strings Ruff parses as annotations (local and class annotations, TypeAlias values, cast/TypeVar/NewType arguments, starred parameters) and which it leaves alone (Annotated metadata, type comments, a non-type cast argument); Ruff's fixtures have no string annotations.
    "LOG004-string-annotation-positions.py": "import logging\nimport typing\n"
                                             "from typing import Annotated, NewType, TypeAlias, TypeVar, cast\n\n"
                                             "logger = logging.getLogger(__name__)\n\n\ndef f():\n"
                                             "    local: \"logger.exception('local annotation')\" = 1\n"
                                             "    local2: \"logger.info('local annotation', exc_info=True)\"\n"
                                             "    local3: \"logger.exception('local annotation', exc_info=False)\" = 1\n"
                                             "\n\na1: Annotated[int, \"logger.exception('annotated metadata')\"] = 1\n"
                                             "a2: Annotated[\"logger.exception('annotated type')\", 1] = 1\n"
                                             "A3: TypeAlias = \"logger.exception('typealias value')\"\n"
                                             "A4: TypeAlias = \"logger.info('typealias value', exc_info=True)\"\n"
                                             "A5: TypeAlias = \"logger.exception('typealias value', exc_info=None)\"\n"
                                             "N = NewType(\"N\", \"logger.exception('newtype')\")\n"
                                             "T = TypeVar(\"T\", \"logger.exception('constraint')\", \"logger.info('c', exc_info=True)\")\n"
                                             "c1 = cast(typ=\"logger.exception('cast kw')\", val=1)\n"
                                             "c2 = typing.cast(\"logger.exception('cast', exc_info=False)\", val=1)\n"
                                             "c3 = cast(1, \"logger.exception('cast second')\")\n\n\nclass K:\n"
                                             "    attr: \"logger.exception('class annotation')\"\n"
                                             "    attr2: \"logger.info('class annotation', exc_info=True)\"\n\n\n"
                                             "if typing.TYPE_CHECKING:\n"
                                             "    tc: \"logger.exception('type checking')\" = 1\n\n\n"
                                             "def g(*args: \"logger.exception('star')\", **kw: \"logger.exception('kwargs', exc_info=0)\"):\n"
                                             "    pass\n\n\nx = 1  # type: \"logger.exception('type comment')\"\n",
    # LOG007: Pins Ruff's empty-f-string reading: `{{`/`}}` and escapes are content, raw f-strings, nested and conditional interpolations, empty format specs and conversions, and Ruff's quirk that `f"{'a' f''}"` counts as empty; Ruff's fixture has no f-strings.
    "LOG007-fstring-emptiness-edges.py": "import logging\n\nlogger = logging.getLogger(__name__)\n\n"
                                         "logger.exception(\"x\", exc_info=f\"{{}}\")\n"
                                         "logger.exception(\"x\", exc_info=f\"{{\")\n"
                                         "logger.exception(\"x\", exc_info=f\"}}\")\n"
                                         "logger.exception(\"x\", exc_info=f\"\\n\")\n"
                                         "logger.exception(\"x\", exc_info=f\"\\x00\")\n"
                                         "logger.exception(\"x\", exc_info=f\"\\N{DIGIT ZERO}\")\n"
                                         "logger.exception(\"x\", exc_info=rf\"{''}\")\n"
                                         "logger.exception(\"x\", exc_info=fr\"{''}{''}\")\n"
                                         "logger.exception(\"x\", exc_info=f\"{'' if x else ''}\")\n"
                                         "logger.exception(\"x\", exc_info=f\"{(y := '')}\")\n"
                                         "logger.exception(\"x\", exc_info=f\"{'a' f''}\")\n"
                                         "logger.exception(\"x\", exc_info=f\"{'' f'{x}'}\")\n"
                                         "logger.exception(\"x\", exc_info=f\"{f'{x}'}\")\n"
                                         "logger.exception(\"x\", exc_info=f\"{''}\" f\"\")\n"
                                         "logger.exception(\"x\", exc_info=f\"{u''}\")\n"
                                         "logger.exception(\"x\", exc_info=f\"{r''}\")\n"
                                         "logger.exception(\"x\", exc_info=f\"{''''''}\")\n"
                                         "logger.exception(\"x\", exc_info=f'''''')\n"
                                         "logger.exception(\"x\", exc_info=f\"\"\"\n\"\"\")\n"
                                         "logger.exception(\"x\", exc_info=f\"{''!a}\")\n"
                                         "logger.exception(\"x\", exc_info=f\"{'':}\")\n"
                                         "logger.exception(\"x\", exc_info=f\"{'':{''}}\")\n"
                                         "logger.exception(\"x\", exc_info=f\"{'' '' ''}\")\n"
                                         "logger.exception(\"x\", exc_info=f\"{b'' b''}\")\n"
                                         "logger.exception(\"x\", exc_info=f\"{('', )}\")\n",
    # LOG014: Pins the callee set (imported `log`/`warn`/`exception` aliases in, `fatal` and attribute `log` out), the reported keyword range across parentheses, comments, spacing and line continuations, chained and nested logging calls, and `# noqa` placement on multi-line calls; Ruff's fixtures only use `logger.info(..., exc_info=True)` on one line.
    "LOG014-callees-and-keyword-ranges.py": "import logging\n"
                                            "from logging import log as L, fatal, warn as w, exception as ex, getLogger as gl\n"
                                            "\nlogger = gl(__name__)\n\nL(10, \"x\", exc_info=True)\n"
                                            "fatal(\"x\", exc_info=True)\nw(\"x\", exc_info=True)\n"
                                            "ex(\"x\", exc_info=True)\nlogger.fatal(\"x\", exc_info=True)\n"
                                            "logger.log(10, \"x\", exc_info=True)\n"
                                            "logging.log(10, \"x\", exc_info=True)\n"
                                            "logging.fatal(\"x\", exc_info=True)\nlogging.warn(\"x\", exc_info=True)\n"
                                            "gl().warn(\"x\", exc_info=True)\n(logger.info)(\"x\", exc_info=True)\n"
                                            "((logger).info)(\"x\", exc_info=True)\n"
                                            "logger.info(\"x\", exc_info=(True))\nlogger.info(\"x\", exc_info=(  # c\n"
                                            "    True))\nlogger.info(\"x\", exc_info= True)\n"
                                            "logger.info(\"x\", exc_info =True)\nlogger.info(\"x\", exc_info\\\n"
                                            "=True)\nlogger.info(\"x\", exc_info=True,)\n"
                                            "logger.info(\"x\", exc_info=b\"\" b\"a\")\n"
                                            "logger.info(\"x\", exc_info=\"\" \"\")\n"
                                            "logger.info(\"x\", exc_info=0 if 1 else 1)\n"
                                            "logger.info(\"x\", exc_info=True)  # noqa: LOG007\n"
                                            "logger.info(\"x\", exc_info=True)  # noqa:LOG014\n"
                                            "logger.info(\"x\", exc_info=True)  # NOQA\nlogger.info(\n    \"x\",\n"
                                            "    exc_info=True,  # noqa: LOG014\n)\nlogger.info(  # noqa: LOG014\n"
                                            "    \"x\",\n    exc_info=True,\n)\n"
                                            "logger.info(\"x\", exc_info=True).info(\"y\", exc_info=True)\n"
                                            "foo(logger.info(\"x\", exc_info=True))\nlogger.info(foo(exc_info=True))\n"
                                            "logger.info(logger.debug(\"x\", exc_info=1), exc_info=2)\n",
    # N801: PEP 695 generic class headers: the name is still the only reported span (Ruff reports my_list, pair and Box_, not MyList).
    "N801-pep695-generic-classes.py": "class my_list[T]:\n    pass\n\n\nclass MyList[T]:\n    pass\n\n\n"
                                      "class pair[K, V](dict[K, V]):\n    pass\n\n\nclass Box_[T: int]:\n    pass\n",
    # N802: visit_/do_ exemption lookups as Ruff does them: a base bound in an enclosing class body is invisible (visit_Nested reported), an `Outer.Inner` attribute base reaches the nested NodeVisitor or BaseHTTPRequestHandler subclass (visit_Attr, do_GET, do_POST exempt), and a def under if in the class body is not a direct method (do_PUT reported); Ruff's fixtures cover none of these.
    "N802-visitor-class-scope-and-attribute-bases.py": "import ast\nimport http.server\n\n\nclass Outer:\n"
                                                       "    class Inner(ast.NodeVisitor):\n        pass\n\n"
                                                       "    class Handler(http.server.BaseHTTPRequestHandler):\n"
                                                       "        pass\n\n    class V2(Inner):\n"
                                                       "        def visit_Nested(self, node): ...\n\n\n"
                                                       "class V(Outer.Inner):\n    def visit_Attr(self, node): ...\n\n"
                                                       "\nclass H(Outer.Handler):\n    def do_GET(self): ...\n\n\n"
                                                       "class H2(Outer.Handler):\n    def do_POST(self): ...\n\n"
                                                       "    if True:\n        def do_PUT(self): ...\n\n"
                                                       "    def DoIt(self): ...\n",
    # N803: The override exemption holds only for a def in a class scope (including under if in the class body): nested, module-level and in-function overrides and a def inside an override method are still reported, which Ruff's fixtures only touch with one module-level case.
    "N803-override-is-class-scope-only.py": "from typing import override\n\n\nclass A:\n    @override\n"
                                            "    def good(self, X, Y=1, *Z, **W): ...\n\n    @override\n"
                                            "    @staticmethod\n    def static(X): ...\n\n    @staticmethod\n"
                                            "    @override\n    def static2(X): ...\n\n    def bad(self, X): ...\n\n"
                                            "    if True:\n        @override\n        def cond(self, X): ...\n\n"
                                            "    def method(self):\n        @override\n        def nested(X): ...\n\n"
                                            "        class Inner:\n            @override\n"
                                            "            def inner(self, X): ...\n\n"
                                            "            def innerbad(self, X): ...\n\n        return nested, Inner\n\n"
                                            "\n@override\ndef module_level(X): ...\n\n\ndef func():\n    @override\n"
                                            "    def in_func(X): ...\n\n    class K:\n        @override\n"
                                            "        def in_class_in_func(self, X): ...\n\n"
                                            "        def bad(self, X): ...\n",
    # N804: Metaclass detection on bases Ruff's fixture lacks: an `Outer.Inner` attribute base to a nested metaclass, `abc.ABCMeta[int]` (not a metaclass), `type[int]`, `builtins.type`, `enum.EnumType`, a `type(...)` call base before or after `type`, plus implicit and decorated dunders, posonly/keyword-only/star first parameters, and annotated defaults with three nested brackets, a quoted `=` and a call with keyword arguments in the annotation.
    "N804-attribute-bases-and-annotated-defaults.py": "import abc\nimport enum\nimport builtins\n\n\nclass Outer:\n"
                                                      "    class Inner(type):\n        pass\n\n\n"
                                                      "class M(Outer.Inner):\n    def f(this): ...\n\n\n"
                                                      "class M2(abc.ABCMeta[int]):\n    def f(this): ...\n\n\n"
                                                      "class M3(type[int]):\n    def f(this): ...\n\n\n"
                                                      "class M4(builtins.type):\n    def f(this): ...\n\n\n"
                                                      "class M5(enum.EnumType):\n    def f(this): ...\n\n\n"
                                                      "class M6(type(abc.ABC)):\n    def f(this): ...\n\n\n"
                                                      "class M7(type(abc.ABC), type):\n    def f(this): ...\n\n\n"
                                                      "class M8(abc.ABCMeta, metaclass=abc.ABCMeta):\n"
                                                      "    def f(this): ...\n\n    def __new__(mcs, *a): ...\n\n"
                                                      "    def __init_subclass__(this): ...\n\n    @staticmethod\n"
                                                      "    def __class_getitem__(this): ...\n\n    @classmethod\n"
                                                      "    def __new__(this): ...\n\n    async def af(this): ...\n\n"
                                                      "    def g(this, /, b): ...\n\n    def h(*, this): ...\n\n"
                                                      "    def i(*this): ...\n\n    def j(): ...\n\n"
                                                      "    def k(this: int = 3, b=2): ...\n\n    def l(this=3): ...\n\n"
                                                      "    def m(this: dict[str, list[tuple[int, int]]] = {}): ...\n\n"
                                                      "    def n(this: \"x=y\" = 3): ...\n\n"
                                                      "    def o(this: Annotated[int, Field(default=dict(a=1))] = 3): ...\n"
                                                      "\n    def p(cls): ...\n\n    def q(setUp): ...\n",
    # N805: Pins the span of `name: annotation = default` first parameters whose annotation contains `=`, `==`, a lambda default or a string with `=`.
    "N805-annotation-with-equals.py": "from typing import Annotated\n\n\nclass A:\n"
                                      "    def a(x: Annotated[int, Field(default=1)] = 2): ...\n"
                                      "    def b(x: \"a=b\" = 1): ...\n"
                                      "    def c(x: Annotated[int, (lambda q=1: q)] = 3): ...\n"
                                      "    def d(x: dict[str, int]={}): ...\n    def e(x: int  =  1, /): ...\n"
                                      "    def f(x: Annotated[int, f(a==1)] = 2): ...\n",
    # N806: Pins that a walrus in a comprehension's first iterable is a function-scope store (reported) while one in an inner comprehension's first iterable, a condition or a later iterable is not.
    "N806-comprehension-first-iterable-walrus.py": "def f(y):\n    a = [x for x in (Y1 := range(3))]\n"
                                                   "    b = {x: 1 for x in (Y2 := y)}\n"
                                                   "    c = [[z for z in (W := x)] for x in y]\n"
                                                   "    d = (x for x in y if (Y3 := x))\n"
                                                   "    e = [x for x in y for q in (Y4 := y)]\n"
                                                   "    return a, b, c, d, e\n",
    # N807: Pins NFKC normalization: a fullwidth low line still makes a dunder name, and a fullwidth module-level __getattr__ is exempt while a nested __dir__ is reported.
    "N807-nfkc-dunder-names.py": "def _＿x__(): ...\ndef __ｇｅｔａｔｔｒ__(name): ...\n\n\ndef f():\n    def __ｄｉｒ__(): ...\n",
    # N811: Pins that names NFKC expands to several characters (Ǆ, Ĳ, Ⅱ, Ŀ, Ǳ) lose the single-character CamelCase exemption while Ⅰ and Ⅴ keep it.
    "N811-nfkc-expanding-single-char.py": "from m import Ǆ as ǅ\nfrom m import Ĳ as Ij\nfrom m import Ⅱ as Two\n"
                                          "from m import Ŀ as Lx\nfrom m import Ǳ as dz\nfrom m import Ⅰ as One\n"
                                          "from m import Ⅴ as five\n",
    # N812: Dotted import, NFKC-normalised aliases and names (modifier capital, titlecase digraph, compatibility letters), default ignore-names, CJK alias, and a parenthesised multi-line import: all agree with Ruff 0.16.9.
    "N812-normalized-alias-and-ignored-names.py": "import pkg.sub.lower as Lower\nimport pkg.sub.lower as lower_again\n"
                                                  "from mod import abc as ᴬbc\nfrom mod import ǅx as ǅx\n"
                                                  "from mod import ǆx as ǅx\nfrom mod import foo as setUp\n"
                                                  "from mod import foo as maxDiff\nfrom mod import foo as 日本\n"
                                                  "from mod import foo as _\nfrom mod import (\n    Camel as camel,\n"
                                                  "    plain as Plain,  # reported\n)\nfrom mod import foo as ŉx\n"
                                                  "from mod import foo as ˤx\n",
    # N813: Dotted import, underscore names that are not CamelCase, mixedCase and uncased (CJK) names that are, a titlecase digraph name, default ignore-names, and a parenthesised multi-line import: all agree with Ruff 0.16.9.
    "N813-dotted-underscore-and-ignored-names.py": "import pkg.sub.CamelCase as camelcase\n"
                                                   "import pkg.sub.Camel_Case as camel_case\n"
                                                   "from mod import mixedCase as mixedcase\n"
                                                   "from mod import setUp as setup\n"
                                                   "from mod import maxDiff as maxdiff\nfrom mod import 日本 as x\n"
                                                   "from mod import ǅx as dzx\nfrom mod import (\n    First as first,\n"
                                                   "    Second as SECOND,\n    Third as Third,\n)\n",
    # N814: Single-character aliases that NFKC-normalise to two or more characters (Ǆ, Ⅻ, Ŀ) are reported like Ruff, while one that stays one character (Ⅰ, C) is skipped as ambiguous.
    "N814-single-char-alias-nfkc-length.py": "from m import Camel as Ǆ\nfrom m import Camel as Ⅻ\n"
                                             "from m import Camel as Ŀ\nfrom m import Camel as Ⅰ\n"
                                             "from m import Camel as C\nfrom m import Camel as CA\n",
    # N815: Pins the class-scope edges (parameter and return annotations, type alias value, first comprehension iterable versus the rest), parenthesised and starred with targets, or-pattern captures, a `Outer.CondTD` nested TypedDict base and a doubly subscripted TypedDict base, all agreeing with Ruff 0.16.9.
    "N815-scope-edges-nested-typeddict-and-with-targets.py": "from typing import TypedDict\n\n\nclass Outer:\n"
                                                             "    if True:\n        class CondTD(TypedDict):\n"
                                                             "            x: int\n\n\nclass FromNested(Outer.CondTD):\n"
                                                             "    nestedCase: int\n\n\n"
                                                             "class DoubleSub(TypedDict[int][str]):\n"
                                                             "    doubleCase: int\n\n\nclass A:\n"
                                                             "    def f(self, x: (aB := int) = (cD := 1)) -> (eF := int):\n"
                                                             "        pass\n"
                                                             "    xs = [x for x in (gH := range(3)) if (iJ := x) for y in (kL := xs)]\n"
                                                             "    type mN = (oP := int)\n    with ctx() as (qR): pass\n"
                                                             "    with ctx() as (sT, *uV): pass\n    match xs:\n"
                                                             "        case [wX, (yZ | yZ)]: pass\n",
    # N816: Pins walrus scope across nested comprehensions (only first-iterable chains reach module scope) and parenthesised augmented targets at module scope versus function/class scope.
    "N816-nested-comprehension-and-paren-augassign.py": "zs = [[y for y in (fooA := x)] for x in [[1]]]\n"
                                                        "gs = {k: v for k, v in (fooB := {}).items()}\n"
                                                        "ss = {[z for z in (fooC := w)][0] for w in [\"a\"]}\n"
                                                        "ts = (a for a in [b for b in (fooD := [1])])\n"
                                                        "us = [x for x in [y for y in (fooE := [1])]]\n"
                                                        "vs = [x for x in [(fooF := y) for y in [1]]]\n(fooN) += 1\n"
                                                        "((fooM)) -= 1\ndef f():\n    global fooP\n    (fooP) += 1\n"
                                                        "class K:\n    (fooQ) *= 2\nif x:\n    (fooR) //= 2\n"
                                                        "(_fooS) += 1\n(FooT) += 1\n",
    # N817: Pins the ET convention through spaced and continued dotted paths, and near-miss module paths (cElementTree, ElementTree.ElementTree, from xml.etree.ElementTree) still reported.
    "N817-import-convention-near-misses.py": "import xml . etree . ElementTree as ET\n"
                                             "from xml . etree import ElementTree as ET\nimport xml.etree\\\n"
                                             ".ElementTree as ET\nfrom xml.etree.ElementTree import ElementTree as ET\n"
                                             "import xml.etree.cElementTree as ET\nfrom xml import etree as ET\n"
                                             "import xml.etree.ElementTree.ElementTree as ET\n",
    # N818: Pins NFKC-equivalent spellings of the Error suffix and of Exception/...Error bases (fullwidth, mathematical, modifier and roman-numeral letters).
    "N818-nfkc-suffix-and-base.py": "class A(Ｅxception): ...\nclass B(𝐄xcept𝐢on): ...\nclass C(Fooｅrror): ...\n"
                                    "class Dｅrror(Exception): ...\nclass Ｅrror(Exception): ...\nclass F(ⅹError): ...\n"
                                    "class G(Exceptⅰon): ...\nclass H(ᴱrror): ...\nclass I(Exceptionⅰ): ...\n",
    # NPY001: Pins reports inside a string annotation and inside a match-case class pattern, neither of which Ruff's own NPY001 fixture covers.
    "NPY001-string-annotation-and-case-pattern.py": "from typing import Optional\n\nimport numpy as np\n\n\n"
                                                    "def f(value: \"Optional[np.float]\") -> None:\n    match value:\n"
                                                    "        case np.int():\n            pass\n",
    # NPY002: A parenthesized callee is reported at the inner attribute while an uncalled reference is not; Ruff's own fixture only has plain calls.
    "NPY002-parenthesized-callee.py": "import numpy as np\n\n(np.random.rand)()\ndraw = np.random.rand\n",
    # NPY003: An aliased from-import and uncalled references; Ruff's fixture only has unaliased from-imports and calls.
    "NPY003-aliased-import.py": "import numpy as np\nfrom numpy import alltrue as every\n\nevery([True])\n"
                                "np.round_(1.5, 1)\nvalues = [np.product, every]\n",
    # NPY201: Only a handler's own type or a direct element of its tuple counts as catching AttributeError; a nested tuple, a call, an `as` name or a subscript do not, and Ruff's fixtures never vary the handler type.
    "NPY201-handled-exception-forms.py": "import numpy as np\nimport builtins\n\ntry:\n    np.all([1])\n"
                                         "except (ValueError, (AttributeError, KeyError)):\n    np.alltrue([1])\ntry:\n"
                                         "    np.any([1])\nexcept type(AttributeError):\n    np.sometrue([1])\ntry:\n"
                                         "    np.prod([1])\nexcept Exception as AttributeError:\n    np.product([1])\n"
                                         "try:\n    np.round(1)\nexcept [AttributeError][0]:\n    np.round_(1)\ntry:\n"
                                         "    np.cumprod(1)\nexcept (builtins.AttributeError,):\n    np.cumproduct(1)\n",
    # PD002: Pins Ruff's deferral of annotations under `from __future__ import annotations` (aliased): parameter and class-body annotations above the only pandas import, a star import inside a later function, are reported.
    "PD002-future-annotations-deferred.py": "from __future__ import annotations as ann\n\n\n"
                                            "def f(x: frame.drop(inplace=True)) -> None:\n    pass\n\n\nclass C:\n"
                                            "    a: frame.rename(inplace=True)\n\n\ndef g():\n"
                                            "    from pandas import *\n",
    # PD012: Pins the decoded string value Ruff compares: a named escape, an 8-digit unicode escape and a backslash continuation equal a comma, while a raw `\x2c`, `", "`, `" ,"` and `"\,"` do not.
    "PD012-separator-escapes.py": "import pandas as pd\n\npd.read_table(\"a\", sep=\"\\N{COMMA}\")\n"
                                  "pd.read_table(\"a\", sep=\",\")\npd.read_table(\"a\", sep=\"\\U0000002c\")\n"
                                  "pd.read_table(\"a\", sep=r\"\\x2c\")\npd.read_table(\"a\", sep=\",\\\n\")\n"
                                  "pd.read_table(\"a\", sep=\", \")\npd.read_table(\"a\", sep=\" ,\")\n"
                                  "pd.read_table(\"a\", sep=\"\\,\")\n",
    # PD101: Pins Ruff's binding kinds: module-scope `__all__` and a chained `__all__ = x = ...` target are exports, a function-local `__all__` is an assignment, and a comprehension target in a `with` header is a with-item variable while one in an expression statement is relevant.
    "PD101-dunder-all-and-with-header-comprehension.py": "import pandas as pd\n\n__all__ = x = pd.Series([1])\n"
                                                         "__all__.nunique() == 1\nx.nunique() == 1\n\n\ndef f():\n"
                                                         "    __all__ = pd.Series([1])\n"
                                                         "    return __all__.nunique() == 1\n\n\n"
                                                         "with ctx([s.nunique() == 1 for s in frames]):\n    pass\n"
                                                         "[t.nunique() == 1 for t in frames]\n",
    # PERF101: Pins Ruff's statement-level mutation test (a parenthesized call counts, a call inside a tuple statement or a `del` of a tuple display does not), a bare tuple value, def, import and comprehension-walrus rebindings in the loop, and a nonlocal name; none is in Ruff's fixture.
    "PERF101-statement-mutations.py": "a = 1, 2\nfor i in list(a):\n    pass\ndef f1():\n    x = [1]\n"
                                      "    for i in list(x):\n        (x.append(1))\ndef f2():\n    x = [1]\n"
                                      "    for i in list(x):\n        x.append(1), 2\ndef f3():\n    x = [1]\n"
                                      "    for i in list(x):\n        del (x[0], i)\ndef f4():\n    x = [1]\n"
                                      "    for i in list(x):\n        del (x[0]), i\ndef f5():\n    x = [1]\n"
                                      "    for i in list(x):\n        def x():\n            pass\ndef f6():\n"
                                      "    x = [1]\n    for i in list(x):\n        [x := 5 for _ in range(3)]\n"
                                      "def f7():\n    x = [1]\n    for i in list(x):\n        import x.y\ndef f8():\n"
                                      "    x = [1]\n    def inner():\n        nonlocal x\n        for i in list(x):\n"
                                      "            pass\n",
    # PERF102: Pins `(d.items)()` (reported on `d.items`, inside the parentheses), reads copied forward from earlier and later bindings, and an empty-tuple element counting as unused; none is in Ruff's fixture.
    "PERF102-parenthesized-callee.py": "def f1(d):\n    for k, v in (d.items)():\n        print(k)\ndef f2(d):\n"
                                       "    for k, v in ((d).items)(x=1):\n        print(v)\ndef f3(d):\n    v = 1\n"
                                       "    print(v)\n    for k, v in d.items():\n        print(k)\ndef f4(d):\n"
                                       "    for k, () in d.items():\n        print(k)\ndef f5(d):\n"
                                       "    for k, v in d.items():\n        print(k)\n    v = 2\n    print(v)\n",
    # PERF203: Pins the handler's end position against trailing, own-line, nested and line-continuation comments and `#` inside strings, which tree-sitter keeps inside the except clause; Ruff's fixture has none.
    "PERF203-trailing-comments.py": "def f(xs):\n    for x in xs:\n        try:\n            g(x)\n"
                                    "        except ValueError:\n            h(\"a # b\")  # trailing\n"
                                    "    for x in xs:\n        try:\n            g(x)\n        except ValueError:\n"
                                    "            s = 'it''s # x'\n            # own line\n            # another\n"
                                    "    for x in xs:\n        try:\n            g(x)\n        except ValueError:\n"
                                    "            h(x)  # don't \"quote\"\n    for x in xs:\n        try:\n"
                                    "            g(x)\n        except ValueError:\n            h(x); y = 1  # c\n"
                                    "    for x in xs:\n        try:\n            g(x)\n        except ValueError:\n"
                                    "            h(\n                x,  # inner\n            )\n            # after\n"
                                    "    for x in xs:\n        try:\n            g(x)\n        except ValueError:\n"
                                    "            t = \"\"\"\n            # not a comment\n            \"\"\"\n"
                                    "    for x in xs:\n        try:\n            g(x)\n        except ValueError:\n"
                                    "            if x:\n                pass  # deep\n            # tail\n\n"
                                    "    for x in xs:\n        try:\n            g(x)\n        except ValueError:\n"
                                    "            y = 1 + \\\n                2  # cont\n    for x in xs:\n"
                                    "        try:\n            g(x)\n        except ValueError: pass  # one line\n"
                                    "    for x in xs:\n        try:\n            g(x)\n        except ValueError:\n"
                                    "            h(r\"\\\"#\")  # raw\n",
    # PERF401: Pins the with-item and def return-annotation list bindings, a doubly parenthesized target with a trailing semicolon, and a later nested `nonlocal` of the target suppressing the report.
    "PERF401-binding-forms-and-nonlocal.py": "from typing import List\n\n\ndef made() -> List[int]:\n    return []\n\n"
                                             "\ndef f(items):\n    with list() as res:\n        for i in items:\n"
                                             "            res.append(i + 1)\n    for i in items:\n"
                                             "        made.append(i + 1)\n    out = []\n"
                                             "    for ((i)) in items: out.append(i + 1);\n    acc = []\n"
                                             "    for j in items:\n        acc.append(j + 1)\n    def g():\n"
                                             "        nonlocal j\n        j = 0\n    return g\n",
    # PERF402: Pins the with-item and def return-annotation list bindings, a doubly parenthesized target with a trailing semicolon, no report under an if, and a report despite the target being read after the loop.
    "PERF402-binding-forms.py": "from typing import List\n\n\ndef made() -> List[int]:\n    return []\n\n\n"
                                "def f(items, c):\n    with list() as res:\n        for i in items:\n"
                                "            res.append(i)\n    for i in items:\n        made.append(i)\n    out = []\n"
                                "    for ((i)) in items: out.append(i);\n    acc = []\n    for j in items:\n"
                                "        if c:\n            acc.append(j)\n    print(i)\n",
    # PERF403: Pins parenthesized target elements compared with key and value, with-item and def return-annotation dict bindings, a trailing semicolon, and a later nested `nonlocal` of a target name suppressing the report.
    "PERF403-parenthesized-elements-and-nonlocal.py": "def made() -> dict:\n    return {}\n\n\ndef f(items):\n"
                                                      "    with dict() as res:\n        for k, v in items:\n"
                                                      "            res[k] = v\n    for (k), (v) in items:\n"
                                                      "        made[k] = v\n    out = {}\n"
                                                      "    for ((k)), ((v, w)) in items: out[k] = v, w;\n    acc = {}\n"
                                                      "    for a, b in items:\n        acc[a] = b\n    def g():\n"
                                                      "        nonlocal b\n        b = 0\n    return g\n",
    # PGH003: Several ignore markers in one comment: each blank one is its own diagnostic at its own `#`, a tagged one between them or an invalid tag does not count; Ruff's fixtures have at most two per line.
    "PGH003-repeated-markers.py": "x = 1  # type: ignore # type: ignore\nx = 1  # type: ignore # pyright: ignore\n"
                                  "x = 1  # type: ignore[a] # foo # type: ignore\nx = 1  # foo # pyright: ignore\n"
                                  "x = 1  #type:ignore#\nx = 1  # type: ignore[]\n"
                                  "x = 1  # type: ignore [a, b] # pyright: ignore[c]\n"
                                  "x = 1  # type: ignoreme # type: ignore\n"
                                  "x = 1  # type: ignore[a]  # type: ignore[b]\n"
                                  "x = 1  # type: ignore # pyright: ignore # type: ignore\n",
    # PGH004: File-level directives: only the first `ruff:`/`flake8:` marker of a comment decides (blanket, coded, blanket is reported; coded, blanket is not), own-line versus trailing versus indented or bracketed, case and spacing variants, invalid forms; Ruff's tests have one marker per comment.
    "PGH004-file-directive-first-marker.py": "# ruff: noqa  # ruff: noqa: F401  # ruff: noqa\n"
                                             "# ruff: noqa: F401 # ruff: noqa\n#ruff:noqa\n#  flake8  :  NOQA\n"
                                             "# Ruff: noqa\n# ruff: noqa F401\n# ruff: noqa:F401 hello\n"
                                             "# ruff: noqa # noqa\n# hello # ruff: noqa\n# ruff: noqa:\ndef f():\n"
                                             "    # ruff: noqa\n    pass\nx = [\n    # flake8: noqa\n    1,\n]\n"
                                             "x = 1  # ruff: noqa\n# ruffx: noqa\n# ruff:: noqa\n# ruff: noqa : F401\n"
                                             "# ruff: noqa # ruff: noqa: F401\n",
    # PGH005: Parenthesised callee, a backslash continuation between `assert` and its test, comments inside parentheses and semicolon-separated statements, none of which Ruff's fixture has.
    "PGH005-continuation-and-parenthesized-callee.py": "(  # c\n    m.assert_called  # c2\n)\nassert (  # c\n"
                                                       "    m.not_called()  # c2\n)\nassert(m.not_called())\n"
                                                       "assert(m.not_called)\nassert (m.not_called)  # x\n"
                                                       "assert m.not_called() , \"x\" \"y\"\n"
                                                       "assert m.not_called() if 1 else 2\n"
                                                       "assert a.not_called() is b.not_called\n"
                                                       "assert m.not_called() \\\n    , \"msg\"\nassert \\\n"
                                                       "    m.not_called()\nm.\\\n    assert_called\n(m.\n"
                                                       " assert_called)\nx = 1; m.assert_called; y = 2\n"
                                                       "if x: m.assert_called; m.assert_called_once\n"
                                                       "assert (m.not_called)()\nassert (m.not_called)(1, 2)  # c\n",
    # PT001: Pins the decorator span end when a form feed (or tab) separates the closing parenthesis from a trailing comment, which the first version extended to the end of the comment.
    "PT001-form-feed-before-trailing-comment.py": "import pytest\n\n\n\f@pytest.fixture()\f  # a\ndef a(): ...\n\n\n"
                                                  "@pytest.fixture(\"x\")\f# b\ndef b(): ...\n\n\nclass K:\n"
                                                  "\t@pytest.fixture(scope=\"function\")\n\tdef c(self): ...\n\n"
                                                  "\t@pytest.fixture()\t# d\n\tdef d(self): ...\n",
    # PT002: Pins the decorator span end when an empty string argument is directly followed by a comment or a backslash continuation and the decorator has a trailing comment containing a parenthesis; the first version could not tokenize these and ended the span after the comment.
    "PT002-empty-string-before-comment-or-continuation.py": "import pytest\n\n\n@pytest.fixture(\"\"#c )\n)  # a )\n"
                                                            "def a(): ...\n\n\n@pytest.fixture(''\\\n)  # b )\n"
                                                            "def b(): ...\n\n\n@pytest.fixture(\"\", \"\"#)\n"
                                                            ", \"\")  # c )\ndef c(): ...\n\n\n"
                                                            "@pytest.fixture(\"x\"#)\n)#d\ndef d(): ...\n",
    # PT003: Pins the first-fixture-decorator look-back across aliases and yield_fixture, the string-literal forms of scope (implicit concatenation, escapes, bytes, f-string), keywords that are not top-level arguments, non-ASCII text before the keyword and a backslash continuation inside the call.
    "PT003-first-decorator-aliases-and-scope-literals.py": "import pytest\nimport pytest as pt\n"
                                                           "from pytest import fixture as fx\n"
                                                           "from pytest import yield_fixture as yf\n\n\n@pt.fixture\n"
                                                           "@pytest.fixture(scope=\"function\")\n"
                                                           "def first_decorator_is_bare_alias(): ...\n\n\n@yf\n"
                                                           "@fx(scope=\"function\")\n"
                                                           "def first_decorator_is_bare_yield_fixture(): ...\n\n\n"
                                                           "@pytest.fixture(scope=\"function\")\n"
                                                           "@fx(scope=\"function\")\n"
                                                           "def first_decorator_is_reported(): ...\n\n\n"
                                                           "@pytest.fixture(scope=\"function\", name=\"a)#b\")  # c\n"
                                                           "def with_trailing_comment(): ...\n\n\n"
                                                           "@pytest.fixture(scope=(\"func\" \"tion\"))\n"
                                                           "def implicit_concatenation(): ...\n\n\n"
                                                           "@pytest.fixture(scope=\"\\x66unction\")\n"
                                                           "def escaped_literal(): ...\n\n\n"
                                                           "@pytest.fixture(scope=b\"function\")\n"
                                                           "def bytes_is_not_a_string(): ...\n\n\n"
                                                           "@pytest.fixture(scope=f\"function\")\n"
                                                           "def fstring_is_not_a_string(): ...\n\n\n"
                                                           "@pytest.fixture(scope=\"function\" if True else \"module\")\n"
                                                           "def conditional(): ...\n\n\n"
                                                           "@pytest.fixture(name=\"é😀\", scope=\"function\")\n"
                                                           "def non_ascii_before_keyword(): ...\n\n\n"
                                                           "@pytest.fixture(**{\"scope\": \"function\"}, name=dict(scope=\"function\"))\n"
                                                           "def not_a_keyword_argument(): ...\n\n\n@pytest.fixture(\n"
                                                           "    \"x\", *args,\n    scope=\\\n"
                                                           "        \"function\"  # c\n    , **kwargs\n)\n"
                                                           "def messy_arguments(): ...\n",
    # PT006: Pins Ruff's find_argument_value lookup for argnames/argvalues: keyword in either order, keyword beating a positional, *args ending the positional lookup, and no finding unless both arguments exist.
    "PT006-argument-lookup-keyword-star-missing.py": "import pytest\n\n\n"
                                                     "@pytest.mark.parametrize(argvalues=[1], argnames=\"a,b\")\n"
                                                     "def t1(a): ...\n\n\n"
                                                     "@pytest.mark.parametrize(\"a,b\", argvalues=[1])\n"
                                                     "def t2(a): ...\n\n\n@pytest.mark.parametrize(argnames=\"a,b\")\n"
                                                     "def t3(a): ...\n\n\n@pytest.mark.parametrize(\"a,b\")\n"
                                                     "def t4(a): ...\n\n\n@pytest.mark.parametrize(\"a,b\", *rows)\n"
                                                     "def t5(a): ...\n\n\n@pytest.mark.parametrize(*args)\n"
                                                     "def t6(a): ...\n\n\n"
                                                     "@pytest.mark.parametrize(*args, \"a,b\", [1])\ndef t7(a): ...\n\n"
                                                     "\n@pytest.mark.parametrize(\"a,b\", [1], *rest)\ndef t8(a): ...\n"
                                                     "\n\n@pytest.mark.parametrize(\"a,b\", **kw)\ndef t9(a): ...\n\n\n"
                                                     "@pytest.mark.parametrize(**kw)\ndef t10(a): ...\n\n\n"
                                                     "@pytest.mark.parametrize(\"a,b\", argnames=\"x,y\", argvalues=[1])\n"
                                                     "def t11(a): ...\n\n\n"
                                                     "@pytest.mark.parametrize([\"a\", \"b\"], argvalues=[1], ids=[\"x\"])\n"
                                                     "def t12(a): ...\n\n\n@pytest.mark.parametrize(\n"
                                                     "    argvalues=[1],\n    argnames=(\"a\", ),\n    ids=[\"x\"],\n"
                                                     ")\ndef t13(a): ...\n\n\n"
                                                     "@pytest.mark.parametrize(\"a,b\", [1], **kw)\ndef t14(a): ...\n\n"
                                                     "\n"
                                                     "@pytest.mark.parametrize(\"a,b\", argnames=[\"x\"], argvalues=[1])\n"
                                                     "def t15(a): ...\n\n\n"
                                                     "@pytest.mark.parametrize(*a, argnames=\"a,b\", argvalues=[1])\n"
                                                     "def t16(a): ...\n\n\n"
                                                     "@pytest.mark.parametrize(\"a,b\", [1], \"extra\")\n"
                                                     "def t17(a): ...\n\n\n"
                                                     "@pytest.mark.parametrize(\"a,b\", values=[1])\ndef t18(a): ...\n",
    # PT007: Pins Ruff's is_multi_named: only a plain string literal with at most one name makes the call single-named, so names that are variables, calls, f-strings, bytes or sequences still make list rows a finding.
    "PT007-names-not-a-single-string-literal.py": "import pytest\n\nNAMES = \"a,b\"\n\n\n"
                                                  "@pytest.mark.parametrize(NAMES, [[1, 2]])\ndef t1(a): ...\n\n\n"
                                                  "@pytest.mark.parametrize(f\"a\", [[1, 2]])\ndef t2(a): ...\n\n\n"
                                                  "@pytest.mark.parametrize(b\"a\", [[1, 2]])\ndef t3(a): ...\n\n\n"
                                                  "@pytest.mark.parametrize(f\"a,b\", [[1, 2]])\ndef t4(a): ...\n\n\n"
                                                  "@pytest.mark.parametrize(get(), [[1, 2]])\ndef t5(a): ...\n\n\n"
                                                  "@pytest.mark.parametrize([], [[1, 2]])\ndef t6(a): ...\n\n\n"
                                                  "@pytest.mark.parametrize((), [[1, 2]])\ndef t7(a): ...\n\n\n"
                                                  "@pytest.mark.parametrize(\"a\" f\"b\", [[1, 2]])\ndef t8(a): ...\n\n"
                                                  "\n@pytest.mark.parametrize(r\"a\", [[1, 2]])\ndef t9(a): ...\n\n\n"
                                                  "@pytest.mark.parametrize(u\"a\", [[1, 2]])\ndef t10(a): ...\n\n\n"
                                                  "@pytest.mark.parametrize(\"a\" if x else \"b\", [[1, 2]])\n"
                                                  "def t11(a): ...\n\n\n@pytest.mark.parametrize(1, [[1, 2]])\n"
                                                  "def t12(a): ...\n\n\n@pytest.mark.parametrize(None, [[1, 2]])\n"
                                                  "def t13(a): ...\n\n\n"
                                                  "@pytest.mark.parametrize((\"a\" \"b\"), [[1, 2]])\ndef t14(a): ...\n"
                                                  "\n\n@pytest.mark.parametrize(((\"a\")), [[1, 2]])\ndef t15(a): ...\n"
                                                  "\n\n@pytest.mark.parametrize((\"a,b\"), (()))\ndef t16(a): ...\n\n\n"
                                                  "@pytest.mark.parametrize(\"\", [[1, 2]])\ndef t17(a): ...\n\n\n"
                                                  "@pytest.mark.parametrize(\",\", [[1, 2]])\ndef t18(a): ...\n\n\n"
                                                  "@pytest.mark.parametrize(\" , \", [[1, 2]])\ndef t19(a): ...\n",
    # PT012: Pins that a trailing `;` is not a second statement, that `..., 1` in a `for` body is a tuple and not a lone ellipsis, and that the finding ends at the last statement before same-line and own-line trailing comments, quotes in comments included.
    "PT012-semicolons-ellipsis-tuple-trailing-comments.py": "import pytest\n\n\n"
                                                            "def test_semicolons_and_ellipsis_tuples():\n"
                                                            "    with pytest.raises(E):\n        a();\n\n"
                                                            "    with pytest.raises(E): a();\n\n"
                                                            "    with pytest.raises(E):\n        for x in y: ...;\n\n"
                                                            "    with pytest.raises(E):\n        for x in y:\n"
                                                            "            ..., 1\n\n    with pytest.raises(E):\n"
                                                            "        for x in y:\n            ...,\n\n"
                                                            "    with pytest.raises(E):\n        with ctx(): a(); b()\n"
                                                            "\n\ndef test_trailing_comments():\n"
                                                            "    with pytest.raises(E):\n        a()\n"
                                                            "        b()  # don't\n        # trailing at body indent\n"
                                                            "            # deeper\n\n    with pytest.raises(E):\n"
                                                            "        a()\n        b(\"x # y\")  # it's\n\n"
                                                            "    with pytest.raises(E):\n        a()\n"
                                                            "        x = \"\"\"\ny\"\"\" + z(\"#\")  # z's\n\n"
                                                            "    with pytest.raises(E):\n        a()\n        if x:\n"
                                                            "            b()\n            # inner trailing\n"
                                                            "        # outer trailing\n    c()\n",
    # PT014: Ruff flattens a and b and c into one BoolOp and a < b < c into one Compare, so (a and b) and c and (a < b) < c differ from the unparenthesized forms while a + b + c equals (a + b) + c; Ruff's fixture has none of these
    "PT014-parenthesized-boolop-and-compare-chains.py": "import pytest\na = b = c = 1\n\n"
                                                        "@pytest.mark.parametrize(\"x\", [(a and b) and c, a and b and c, a and (b and c), (a or b) or c, a or b or c, a or (b or c)])\n"
                                                        "def t1(x): ...\n\n"
                                                        "@pytest.mark.parametrize(\"x\", [(a < b) < c, a < b < c, a < (b < c), (a < b) < c])\n"
                                                        "def t2(x): ...\n\n"
                                                        "@pytest.mark.parametrize(\"x\", [not not a, not (not a), - - a, -(-a), a + b + c, (a + b) + c, a + (b + c)])\n"
                                                        "def t3(x): ...\n",
    # PT015: Ruff checks the builtin binding as of the assert, so an assert before a later module-level list = 1 is reported and one after it is not
    "PT015-builtin-rebound-later-in-scope.py": "assert list()\nlist = 1\nassert list()\n",
    # PT017: lambda parameter names (positional, keyword-only, starred, defaults' names) are not Name expressions but lambda bodies and default values are, as Ruff's visitor walks them
    "PT017-lambda-parameters-vs-body-names.py": "try:\n    pass\nexcept Exception as e:\n    assert lambda e: 0\n"
                                                "    assert lambda *, e: 0\n    assert lambda x=e: 0\n"
                                                "    assert lambda e=1: e\n    assert lambda *e: 0\n"
                                                "    assert lambda **e: 0\n    assert lambda e, /: 0\n"
                                                "    assert lambda x=lambda e: 0: 0\n",
    # PT019: Pins that an argnames= keyword wins over a positional first argument, that a starred argument hides the positional argnames, and that a bare name passed as argnames silences the function only when it is the argument actually used.
    "PT019-argnames-keyword-and-starred.py": "import pytest\n\n\n@pytest.mark.parametrize(\"_x\", argnames=\"_a\")\n"
                                             "def test_a(_a, _x):\n    pass\n\n\n"
                                             "@pytest.mark.parametrize(argvalues=[1], argnames=\"_a\")\n"
                                             "def test_b(_a):\n    pass\n\n\n@pytest.mark.parametrize(*[\"_a\"], [1])\n"
                                             "def test_c(_a):\n    pass\n\n\n@pytest.mark.parametrize(*x, \"_a\")\n"
                                             "def test_d(_a):\n    pass\n\n\n"
                                             "@pytest.mark.parametrize(**{\"argnames\": \"_a\"})\ndef test_e(_a):\n"
                                             "    pass\n\n\n@pytest.mark.parametrize(names, argnames=\"_a\")\n"
                                             "def test_f(_a):\n    pass\n\n\n"
                                             "@pytest.mark.parametrize(\"_a\", argnames=names)\ndef test_g(_a):\n"
                                             "    pass\n",
    # PT020: Pins that only the first fixture or yield_fixture decorator of a function is examined: a yield_fixture after a fixture is silent, and only the first of two yield_fixtures is reported.
    "PT020-first-fixture-decorator-wins.py": "import pytest\n\n\n@pytest.fixture\n@pytest.yield_fixture\ndef a():\n"
                                             "    pass\n\n\n@pytest.yield_fixture\n@pytest.fixture\ndef b():\n"
                                             "    pass\n\n\n@pytest.yield_fixture\n@pytest.yield_fixture\ndef c():\n"
                                             "    pass\n\n\n@pytest.mark.foo\n@pytest.yield_fixture()\n"
                                             "@pytest.mark.bar\ndef d():\n    pass\n\n\n@pytest.fixture()\n"
                                             "@pytest.mark.foo\n@pytest.yield_fixture()\ndef e():\n    pass\n",
    # PT021: Pins Ruff's visiting order: an annotation call is visited after the assigned value, type-parameter bounds after the alias value, and a call inside a subscripted annotation is not reached.
    "PT021-annotation-walk-order.py": "import pytest\n\n\n@pytest.fixture\ndef annotated(request):\n"
                                      "    x: request.addfinalizer(one) = request.addfinalizer(two)\n\n\n"
                                      "@pytest.fixture\ndef alias(request):\n"
                                      "    type X[T: request.addfinalizer(one), U: request.addfinalizer(two)] = request.addfinalizer(three)\n"
                                      "\n\n@pytest.fixture\ndef subscripted(request):\n"
                                      "    x: list[request.addfinalizer(one)] = request.addfinalizer(two)\n",
    # PT022: Pins that a yield in a nested def's decorator is skipped (Ruff skips the whole FunctionDef) while one in a nested class's decorator is counted.
    "PT022-nested-def-decorator-yield.py": "import pytest\n\n\ndef deco(x):\n    return lambda f: f\n\n\n"
                                           "@pytest.fixture\ndef nested_def_decorator_yield_skipped():\n"
                                           "    @deco((yield))\n    def inner():\n        pass\n    yield inner\n\n\n"
                                           "@pytest.fixture\ndef nested_class_decorator_yield_counted():\n"
                                           "    @deco((yield))\n    class C:\n        pass\n    yield C\n",
    # PT023: Pins that a name imported from pytest.mark is a marker but one imported from a submodule under it (four segments) is not.
    "PT023-deeper-import-not-marker.py": "from pytest.mark import foo\nfrom pytest.mark.sub import bar\n\n\n@foo()\n"
                                         "def marker_import_flagged(): ...\n\n\n@bar()\n"
                                         "def deeper_import_not_a_marker(): ...\n",
    # PT024: Pins the reported end when a triple-quoted argument holds a quote and a # that is not a comment.
    "PT024-triple-quoted-hash-span.py": "import pytest\n\n\n@pytest.mark.asyncio(reason='''it's # here''')  # c\n"
                                        "@pytest.fixture\nasync def triple_single():\n    pass\n\n\n@pytest.fixture\n"
                                        "@pytest.mark.asyncio(reason=\"\"\"a\"b # c\"\"\")\n"
                                        "async def triple_double():\n    pass\n",
    # PT025: Pins the decorator end position when a Python 3.12 nested same-quote f-string holds a # (which tree-sitter reads as a comment), doubled braces and nested format specs, with a trailing comment.
    "PT025-nested-fstring-span.py": "import pytest\n\n\n@pytest.mark.usefixtures(f\"{\"#\"}\", f'{'#'}')  # trailing\n"
                                    "@pytest.fixture\ndef a(): ...\n\n\n"
                                    "@pytest.mark.usefixtures(f\"{{#}}\")  # trailing\n@pytest.fixture\ndef b(): ...\n"
                                    "\n\n@pytest.mark.usefixtures(rf\"{\"a\"}#\", F'{x!r:>{w}}')  # c\n"
                                    "@pytest.fixture\ndef c(): ...\n",
    # PT026: Pins Ruff's asymmetry (bare attribute reported, bare imported name not, empty call of either reported), classes never reported, and exact end positions with a trailing comment.
    "PT026-bare-name-and-trailing-comment.py": "import pytest\nfrom pytest import mark\n"
                                               "from pytest.mark import usefixtures\n\n\n"
                                               "@pytest.mark.usefixtures()  # it's \"empty\" # really\n"
                                               "def test_a(): ...\n\n\n@mark.usefixtures  # bare attribute\n"
                                               "def test_b(): ...\n\n\n@usefixtures\n"
                                               "def test_c(): ...  # a bare imported name is not reported\n\n\n"
                                               "@usefixtures(  # empty call of an imported name\n)\ndef test_d(): ...\n"
                                               "\n\n@pytest.mark.usefixtures()\nclass TestE: ...\n\n\n"
                                               "@(pytest.mark.usefixtures)\nasync def test_f(): ...\n",
    # PT028: Pins the scope rule (class with __init__, nested non-Test class, function in function, nested Test class), the inner-expression span of parenthesized defaults (walrus, tuple, comment inside the parentheses) and variadic parameters being skipped.
    "PT028-scope-and-parenthesized-defaults.py": "class TestWithInit:\n    def __init__(self): ...\n\n"
                                                 "    def test_a(self, a=1): ...\n\n\nclass TestPlain:\n"
                                                 "    def test_b(self, a=(1), b: int = ((2))):\n"
                                                 "        def test_inner(c=3): ...\n\n    class Helper:\n"
                                                 "        def test_c(self, a=4): ...\n\n    class TestNested:\n"
                                                 "        def test_d(self, a=(x := 6)): ...\n\n\nclass Plain:\n"
                                                 "    def test_e(self, a=1): ...\n\n\nif True:\n\n"
                                                 "    def test_f(a=(x := 1), *args, b=(1, 2), **kwargs): ...\n\n\n"
                                                 "def testament(a=(  # comment\n    5\n)): ...\n",
    # PT030: Which f-string `match=` values Ruff's is_empty_or_null_string calls empty: escaped braces, `=` debug, conversions, format specs, nested and concatenated parts.
    "PT030-fstring-match-forms.py": "import pytest\n\n\ndef test():\n    pytest.warns(Warning, match=f\"{{}}\")\n"
                                    "    pytest.warns(Warning, match=f\"{''=}\")\n"
                                    "    pytest.warns(Warning, match=f\"{''!r}\")\n"
                                    "    pytest.warns(Warning, match=f\"{'':>3}\")\n"
                                    "    pytest.warns(Warning, match=f\"{'',}\")\n"
                                    "    pytest.warns(Warning, match=f\"{('')}\")\n"
                                    "    pytest.warns(Warning, match=f\"{f''}\")\n"
                                    "    pytest.warns(Warning, match=f\"{'' ''}\")\n"
                                    "    pytest.warns(Warning, match=f\"\" \"\")\n"
                                    "    pytest.warns(Warning, match=\"\" f\"{None}\")\n"
                                    "    pytest.warns(Warning, match=f\"{None!s:}\")\n"
                                    "    pytest.warns(Warning, match=F\"\")\n    pytest.warns(Warning, match=fr\"\")\n"
                                    "    pytest.warns(Warning, match=f'''{''}''')\n"
                                    "    pytest.warns(Warning, match=f\"{x}\")\n"
                                    "    pytest.warns(Warning, match=f\"{'a'}\")\n"
                                    "    pytest.warns(Warning, match=f\"{''}{None}{f'{None}'}\")\n"
                                    "    pytest.warns(Warning, match=f\"{'' if x else ''}\")\n"
                                    "    pytest.warns(Warning, match=f\"{b''}\")\n"
                                    "    pytest.warns(Warning, match=f\"\\n\")\n"
                                    "    pytest.warns(Warning, match=f\"{'':{''}}\")\n"
                                    "    pytest.warns(Warning, match=f\"{ '' }\")\n"
                                    "    pytest.warns(Warning, match=f\"{\n        ''\n    }\")\n",
    # PT031: End of the reported range when the last statement is followed by comments or `;` and holds `#` inside string literals of every quote style.
    "PT031-trailing-comments-and-strings.py": "import pytest\n\n\ndef t():\n    with pytest.warns(A):\n        foo()\n"
                                              "        bar(\"#x\")\n    with pytest.warns(A):\n        foo()\n"
                                              "        bar('#')  # c\n    with pytest.warns(A):\n        foo()\n"
                                              "        bar(\"\"\"a\n#b\"\"\")  # c\n    with pytest.warns(A):\n"
                                              "        foo()\n        bar(\"a\\\"#\")  # c ; x\n"
                                              "    with pytest.warns(A):\n        foo()\n        bar('a\\'#') ;  # c\n"
                                              "    with pytest.warns(A):\n        foo()\n        bar(f\"{'#'}\")  # c\n"
                                              "    with pytest.warns(A):\n        foo()\n"
                                              "        bar(r\"\\\\\")  # \"x\"\n    with pytest.warns(A):\n"
                                              "        foo()\n        bar(b'#', u\"#\")\n    with pytest.warns(A):\n"
                                              "        foo()\n        x = '''#''' + \"\"\"#\"\"\"  # '''\n",
    # PTH100: Pins aliased and parenthesized callees, the path keyword, a lone generator argument, and the argument shapes Ruff leaves alone (two arguments, starred, double-starred, unknown keyword, shadowed os).
    "PTH100-aliased-parenthesized-callee.py": "import os\nfrom os import path as pp\n\npp.abspath(\"a\")\n"
                                              "(os.path.abspath)(\"b\")\nos.path.abspath(path=\"c\")\n"
                                              "os.path.abspath(x for x in [\"d\"])\nos.path.abspath(\"e\", \"f\")\n"
                                              "os.path.abspath(*[\"g\"])\nos.path.abspath(**{\"path\": \"h\"})\n"
                                              "os.path.abspath(name=\"i\")\n\n\ndef shadowed(os):\n"
                                              "    os.path.abspath(\"j\")\n",
    # PTH101: Pins the latest class-scope binding (nested if, loop, augmented and later rebindings cancel; chained and tuple targets count; a rebound inner class does not), two-level parenthesized access, and Ruff's int inference (1 + int(s), 2 ** -1, 1 @ 2, bool arms, with item).
    "PTH101-class-latest-binding-and-int-inference.py": "import os\n\n\nclass A:\n    r = s = 7\n    p, q = 1, \"a\"\n"
                                                        "    if c:\n        t = 3\n    u = 3\n    for u in range(2):\n"
                                                        "        pass\n    v = 3\n    v += 1\n    w = 3\n    if c:\n"
                                                        "        w = \"x\"\n    class B:\n        y = 3\n"
                                                        "    def m(self) -> int:\n        return 1\n\n\nclass O:\n"
                                                        "    class B:\n        y = 3\n    B = None\n\n\n"
                                                        "k1 = 1 + int(\"3\")\nk2 = 3 if c else True\nk3 = 2 ** -1\n"
                                                        "k4 = 1 @ 2\nk5 = True if c else False\n\nwith 3 as w1:\n"
                                                        "    pass\n\nos.chmod(A.r, 1)\nos.chmod(A.s, 1)\n"
                                                        "os.chmod(A.p, 1)\nos.chmod(A.q, 1)\nos.chmod(A.t, 1)\n"
                                                        "os.chmod(A.u, 1)\nos.chmod(A.v, 1)\nos.chmod(A.w, 1)\n"
                                                        "os.chmod((A.B).y, 1)\nos.chmod(A.m, 1)\nos.chmod(O.B.y, 1)\n"
                                                        "os.chmod(k1, 1)\nos.chmod(k2, 1)\nos.chmod(k3, 1)\n"
                                                        "os.chmod(k4, 1)\nos.chmod(k5, 1)\nos.chmod(w1, 1)\n",
    # PTH102: Pins that only a non-None dir_fd keyword exempts the call (a parenthesized None does not), and that argument-less, starred and **kwargs calls are still reported.
    "PTH102-dir-fd-default.py": "import os\nfrom os import mkdir\n\nmkdir(\"a\", dir_fd=(None))\n"
                                "mkdir(\"b\", dir_fd=3)\n(os.mkdir)(\"c\", 0o700)\n"
                                "os.mkdir(\"d\", dir_fd=None, **{\"x\": 1})\nos.mkdir(*[\"e\"])\nos.mkdir()\n"
                                "os.mkdir(\"f\", 0o700, dir_fd=fd)\n",
    # PTH103: Pins that every os.makedirs call is reported whatever its arguments (none, starred, double-starred, an unexpected dir_fd), through an alias and a parenthesized callee, but not through a shadowed os.
    "PTH103-any-arguments.py": "import os\nfrom os import makedirs as mk\n\nmk(\"a\", exist_ok=True)\n"
                               "(os.makedirs)(\"b\", 0o700, True)\nos.makedirs(*[\"c\"])\n"
                               "os.makedirs(**{\"name\": \"d\"})\nos.makedirs()\nos.makedirs(\"e\", dir_fd=3)\n\n\n"
                               "def shadowed(os):\n    os.makedirs(\"f\")\n",
    # PTH104: Pins that either dir_fd keyword with a non-None value exempts the call, a parenthesized None does not, and starred, **kwargs and argument-less calls are still reported.
    "PTH104-parenthesized-dir-fd.py": "import os\nfrom os import rename as mv\n\nmv(\"a\", \"b\", src_dir_fd=(None))\n"
                                      "mv(\"c\", \"d\", dst_dir_fd=3)\n"
                                      "os.rename(\"e\", \"f\", src_dir_fd=None, dst_dir_fd=(None))\n"
                                      "os.rename(\"g\", \"h\", src_dir_fd=None, dst_dir_fd=fd)\n"
                                      "os.rename(*[\"i\", \"j\"])\nos.rename(\"k\", \"l\", **{\"src_dir_fd\": 3})\n"
                                      "os.rename()\n",
    # PTH105: Pins that either dir_fd keyword with a non-None value exempts the call, a parenthesized None does not, and starred, **kwargs and argument-less calls are still reported.
    "PTH105-parenthesized-dir-fd.py": "import os\nfrom os import replace as mv\n\nmv(\"a\", \"b\", src_dir_fd=(None))\n"
                                      "mv(\"c\", \"d\", dst_dir_fd=3)\n"
                                      "os.replace(\"e\", \"f\", src_dir_fd=None, dst_dir_fd=(None))\n"
                                      "os.replace(\"g\", \"h\", src_dir_fd=None, dst_dir_fd=fd)\n"
                                      "os.replace(*[\"i\", \"j\"])\nos.replace(\"k\", \"l\", **{\"src_dir_fd\": 3})\n"
                                      "os.replace()\n",
    # PTH106: calls in simple, nested-quoted and complex (concatenated or escaped) string annotations, a forward reference to a later function-local import, and the argument limits inside a string
    "PTH106-string-annotation-calls.py": "import os\np = \"x\"\nz: \"os.rmdir(p)\" = 1\n"
                                         "def f(b: \"os.rmdir(path=p)\" = 1) -> \"list['os.rmdir(p)']\":\n"
                                         "    k: \"os.rmdir(p)\"\n    import os\nw: \"os.rmdir(p, p)\"\n"
                                         "m: \"os.rmdir\" \"(p)\"\nn: \"os.rmdir(1)\" \"\"\no: \"os.rmdir(\\x70)\"\n",
    # PTH107: calls in simple, nested-quoted and complex (concatenated or escaped) string annotations, a forward reference to a later function-local import, and the argument limits inside a string
    "PTH107-string-annotation-calls.py": "import os\np = \"x\"\nz: \"os.remove(p)\" = 1\n"
                                         "def f(b: \"os.remove(path=p)\" = 1) -> \"list['os.remove(p)']\":\n"
                                         "    k: \"os.remove(p)\"\n    import os\nw: \"os.remove(p, p)\"\n"
                                         "m: \"os.remove\" \"(p)\"\nn: \"os.remove(1)\" \"\"\no: \"os.remove(\\x70)\"\n",
    # PTH108: calls in simple, nested-quoted and complex (concatenated or escaped) string annotations, a forward reference to a later function-local import, and the argument limits inside a string
    "PTH108-string-annotation-calls.py": "import os\np = \"x\"\nz: \"os.unlink(p)\" = 1\n"
                                         "def f(b: \"os.unlink(path=p)\" = 1) -> \"list['os.unlink(p)']\":\n"
                                         "    k: \"os.unlink(p)\"\n    import os\nw: \"os.unlink(p, p)\"\n"
                                         "m: \"os.unlink\" \"(p)\"\nn: \"os.unlink(1)\" \"\"\no: \"os.unlink(\\x70)\"\n",
    # PTH109: getcwd and getcwdb calls in simple, nested-quoted and complex (concatenated or escaped) string annotations, and a forward reference to a later function-local import
    "PTH109-string-annotation-calls.py": "import os\np = \"x\"\nz: \"os.getcwd()\" = 1\n"
                                         "def f(b: \"os.getcwdb()\" = 1) -> \"list['os.getcwd()']\":\n"
                                         "    k: \"os.getcwd()\"\n    import os\nw: \"os.getcwd(p, os)\"\n"
                                         "m: \"os.getcwd\" \"(p)\"\nn: \"os.getcwd()\" \"\"\no: \"os.getcwd(\\x29)\"\n",
    # PTH110: calls in simple, nested-quoted and complex (concatenated or escaped) string annotations, a forward reference to a later function-local import, and the argument limits inside a string
    "PTH110-string-annotation-calls.py": "import os\np = \"x\"\nz: \"os.path.exists(p)\" = 1\n"
                                         "def f(b: \"os.path.exists(path=p)\" = 1) -> \"list['os.path.exists(p)']\":\n"
                                         "    k: \"os.path.exists(p)\"\n    import os\nw: \"os.path.exists(p, p)\"\n"
                                         "m: \"os.path.exists\" \"(p)\"\nn: \"os.path.exists(1)\" \"\"\n"
                                         "o: \"os.path.exists(\\x70)\"\n",
    # PTH111: calls in simple, nested-quoted and complex (concatenated or escaped) string annotations, a forward reference to a later function-local import, and the argument limits inside a string
    "PTH111-string-annotation-calls.py": "import os\np = \"x\"\nz: \"os.path.expanduser(p)\" = 1\n"
                                         "def f(b: \"os.path.expanduser(path=p)\" = 1) -> \"list['os.path.expanduser(p)']\":\n"
                                         "    k: \"os.path.expanduser(p)\"\n    import os\n"
                                         "w: \"os.path.expanduser(p, p)\"\nm: \"os.path.expanduser\" \"(p)\"\n"
                                         "n: \"os.path.expanduser(1)\" \"\"\no: \"os.path.expanduser(\\x70)\"\n",
    # PTH112: Pins Ruff's single-argument rule: keyword spelled s (not path), starred and double-starred arguments, a lone generator expression, a parenthesized aliased callee and a comment inside the call.
    "PTH112-argument-shapes.py": "import os\nfrom os import path as osp\n\np = \"x\"\nos.path.isdir(p)\n"
                                 "os.path.isdir(s=p)\nos.path.isdir(path=p)\nos.path.isdir(p, p)\nos.path.isdir(p,)\n"
                                 "os.path.isdir(*[p])\nos.path.isdir(**{\"s\": p})\nos.path.isdir(x for x in [p])\n"
                                 "(osp.isdir)(p)\nos.path.isdir(\n    # why\n    p\n)\n",
    # PTH113: Pins Ruff's single-argument rule: keyword spelled path (not s), starred and double-starred arguments, an aliased from-import, a parenthesized callee and a lone generator expression.
    "PTH113-argument-shapes.py": "import os\nfrom os.path import isfile as is_file\n\np = \"x\"\nos.path.isfile(p)\n"
                                 "os.path.isfile(path=p)\nos.path.isfile(s=p)\nos.path.isfile(p, p)\n"
                                 "os.path.isfile(*[p])\nos.path.isfile(**{\"path\": p})\nis_file(p)\n"
                                 "(os.path.isfile)(p)\nos.path.isfile(x for x in [p])\n",
    # PTH114: Pins Ruff's single-argument rule through an import os.path as alias: keyword spelled path (not s), starred and double-starred arguments, a parenthesized callee, a lone generator expression and a trailing comma.
    "PTH114-argument-shapes.py": "import os.path as osp\n\np = \"x\"\nosp.islink(p)\nosp.islink(path=p)\n"
                                 "osp.islink(s=p)\nosp.islink(p, p)\nosp.islink(*[p])\nosp.islink(**{\"path\": p})\n"
                                 "(osp.islink)(p)\nosp.islink(x for x in [p])\nosp.islink(p,)\n",
    # PTH115: Pins that a dir_fd keyword even set to None silences the rule (Ruff requires exactly one argument), which Ruff's own fixture covers only with dir_fd=1, plus the path keyword spelling and starred forms.
    "PTH115-dir-fd-and-keywords.py": "import os\n\np = \"x\"\nos.readlink(p)\nos.readlink(path=p)\n"
                                     "os.readlink(p, dir_fd=None)\nos.readlink(p, dir_fd=3)\nos.readlink(dir_fd=None)\n"
                                     "os.readlink(s=p)\nos.readlink(*[p])\nos.readlink(**{\"path\": p})\n"
                                     "(os.readlink)(p)\nos.readlink(x for x in [p])\n",
    # PTH116: Pins Ruff's class-scope lookup of the latest binding (rebinding in an if, augmented assignment, tuple unpack, nested class, decorated def) and its structural int inference (condition with a call ignored, bool & bool is int, @ and ** with a negative exponent).
    "PTH116-class-latest-binding-and-inferred-int.py": "import os\nc = input()\nclass A:\n    fd: int = 1\n    if c:\n"
                                                       "        fd = \"x\"\nclass B:\n    fd = \"x\"\n    if c:\n"
                                                       "        fd = 1\nclass C:\n    fd = 1\n    fd += 1\nclass D:\n"
                                                       "    fd, other = 1, \"a\"\nclass S:\n    class T:\n"
                                                       "        fd: int = 1\n    @property\n"
                                                       "    def gd(self) -> int: ...\nos.stat(A.fd)\nos.stat(B.fd)\n"
                                                       "os.stat(C.fd)\nos.stat(D.fd)\nos.stat(S.T.fd)\nos.stat(S.gd)\n"
                                                       "n1 = 1 if c.strip() else 2\nos.stat(n1)\nn2 = True & False\n"
                                                       "os.stat(n2)\nn3 = 1 @ 2\nos.stat(n3)\nn4 = 2 ** -1\n"
                                                       "os.stat(n4)\n",
    # PTH117: Pins Ruff's single-argument rule: keyword spelled s (not path), starred and double-starred arguments, a from-import, a parenthesized callee and a lone generator expression.
    "PTH117-argument-shapes.py": "import os\nfrom os.path import isabs\n\np = \"x\"\nos.path.isabs(p)\n"
                                 "os.path.isabs(s=p)\nos.path.isabs(path=p)\nos.path.isabs(p, p)\nos.path.isabs(*[p])\n"
                                 "os.path.isabs(**{\"s\": p})\nisabs(p)\n(os.path.isabs)(p)\n"
                                 "os.path.isabs(x for x in [p])\n",
    # PTH118: Parenthesized callee, lone generator argument, starred and double-starred arguments and an empty call all fire; a shadowed os parameter and a bare attribute reference do not.
    "PTH118-join-forms.py": "import os\nfrom os import path as p\n\n(os.path.join)(\"a\", \"b\")\n"
                            "os.path.join(x for x in [\"a\", \"b\"])\nos.path.join(*[\"a\"], **{})\np.join()\n\n\n"
                            "def shadowed(os):\n    return os.path.join(\"a\", \"b\")\n\n\nos.path.join.__name__\n",
    # PTH119: Only the one-argument forms fire (keyword p=, lone generator, trailing comma and comment); path=, starred, two-argument and p= plus **kwargs calls do not.
    "PTH119-keyword-forms.py": "import os\n\nos.path.basename(p=\"a\")\nos.path.basename(path=\"a\")\n"
                               "os.path.basename(*[\"a\"])\nos.path.basename(\"a\", \"b\")\n"
                               "os.path.basename(x for x in \"a\")\nos.path.basename(p=\"a\", **{})\n"
                               "os.path.basename(\n    \"a\",  # trailing comma\n)\n",
    # PTH120: A callee separated from its arguments by a line continuation, a parenthesized callee with p=, and a keyword split from its = across lines all fire; path= and an empty call do not.
    "PTH120-continued-callee.py": "import os.path as osp\n\nosp.dirname \\\n    (\"a\")\n(osp.dirname)(p=\"a\")\n"
                                  "osp.dirname(p\n            = \"a\")\nosp.dirname(path=\"a\")\nosp.dirname()\n",
    # PTH121: Ruff lints calls inside string annotations (two in one return annotation, each reported at its own column) and an argument-less call, but not an ordinary string assigned to a variable.
    "PTH121-string-annotation.py": "import os\n\n\n"
                                   "def f(a: \"os.path.samefile('a', 'b')\") -> \"os.path.samefile(a, a) or os.path.samefile(a, a)\":\n"
                                   "    return os.path.samefile()\n\n\nx = \"os.path.samefile('a', 'b')\"\n",
    # PTH122: Parenthesized and aliased callees, a line-continued callee and an empty call fire; calling a plain variable that was assigned the function does not.
    "PTH122-parenthesized-callee.py": "import os\nfrom os.path import splitext as se\n\n(os.path.splitext)(\"a\")\n"
                                      "(((se)))(\"a\")\nos.path.splitext \\\n    (\"a\")\nos.path.splitext()\n"
                                      "splitext = os.path.splitext\nsplitext(\"a\")\n",
    # PTH123: Ruff's int inference over or/and and int-or-bool conditionals, the keyword winning over the positional slot for file and closefd, and the latest class-scope binding of an attribute (augmented rebinding reported; an int bound in an if block or a with item skipped).
    "PTH123-fd-latest-binding.py": "c = 0\na = 1 or 2\nb = 1 if c else True\ns = True if c else False\nopen(a)\n"
                                   "open(b)\nopen(s)\nopen(3, file=\"x.txt\")\n"
                                   "open(\"x.txt\", \"r\", -1, None, None, None, False, closefd=True)\n\n\n"
                                   "class Settings:\n    fd = 3\n    fd += 1\n    if c:\n        handle = 4\n"
                                   "    with int(\"5\") as wd:\n        pass\n\n\nopen(Settings.fd)\n"
                                   "open(Settings.handle)\nopen(Settings.wd)\n",
    # PTH124: A call in a string nested inside a string annotation and one with a doubly parenthesized callee fire, while an Annotated metadata string does not; the original port missed both.
    "PTH124-nested-string-annotation.py": "import py\nfrom typing import Annotated, List\n\n"
                                          "a: \"List['py.path.local(1)']\"\nb: Annotated[int, \"py.path.local()\"]\n"
                                          "c: \"((py.path.local))()\"\n",
    # PTH201: Current-directory calls in a string nested inside a string annotation, in the first Annotated argument and with a doubly parenthesized callee fire, while Annotated metadata does not; the original port missed nested strings and parenthesized callees there.
    "PTH201-nested-string-annotation.py": "from pathlib import Path\nfrom typing import Annotated, List\n\n"
                                          "a: \"\"\"List['Path(\".\")']\"\"\"\n"
                                          "b: Annotated[\"Path('')\", \"Path('.')\"]\nc: \"((Path))('.')\"\n",
    # PTH202: A call in a string nested inside a string annotation and one with a doubly parenthesized callee fire at their exact positions, while Literal and Annotated metadata strings do not; the original port missed nested strings.
    "PTH202-nested-string-annotation.py": "import os\nfrom typing import Annotated, List, Literal\n\n"
                                          "a: \"List['os.path.getsize(1)']\"\nb: \"Literal['os.path.getsize(1)']\"\n"
                                          "c: Annotated[int, \"os.path.getsize('f')\"]\n"
                                          "d: \"((os.path.getsize))('f')\"\n"
                                          "e: \"List['List[\\\"os.path.getsize(filename=1)\\\"]']\"\n",
    # PTH203: A call in a string nested inside a string annotation and one with a doubly parenthesized callee fire at their exact positions, while Literal and Annotated metadata strings do not; the original port missed nested strings.
    "PTH203-nested-string-annotation.py": "import os\nfrom typing import Annotated, List, Literal\n\n"
                                          "a: \"List['os.path.getatime(1)']\"\nb: \"Literal['os.path.getatime(1)']\"\n"
                                          "c: Annotated[int, \"os.path.getatime('f')\"]\n"
                                          "d: \"((os.path.getatime))('f')\"\n"
                                          "e: \"List['List[\\\"os.path.getatime(filename=1)\\\"]']\"\n",
    # PTH204: A call in a string nested inside a string annotation and one with a doubly parenthesized callee fire at their exact positions, while Literal and Annotated metadata strings do not; the original port missed nested strings.
    "PTH204-nested-string-annotation.py": "import os\nfrom typing import Annotated, List, Literal\n\n"
                                          "a: \"List['os.path.getmtime(1)']\"\nb: \"Literal['os.path.getmtime(1)']\"\n"
                                          "c: Annotated[int, \"os.path.getmtime('f')\"]\n"
                                          "d: \"((os.path.getmtime))('f')\"\n"
                                          "e: \"List['List[\\\"os.path.getmtime(filename=1)\\\"]']\"\n",
    # PTH205: A call in a string nested inside a string annotation and one with a doubly parenthesized callee fire at their exact positions, while Literal and Annotated metadata strings do not; the original port missed nested strings.
    "PTH205-nested-string-annotation.py": "import os\nfrom typing import Annotated, List, Literal\n\n"
                                          "a: \"List['os.path.getctime(1)']\"\nb: \"Literal['os.path.getctime(1)']\"\n"
                                          "c: Annotated[int, \"os.path.getctime('f')\"]\n"
                                          "d: \"((os.path.getctime))('f')\"\n"
                                          "e: \"List['List[\\\"os.path.getctime(filename=1)\\\"]']\"\n",
    # PTH206: Parenthesized callees, parenthesized separators, comments and a line continuation around a reported split, next to the maxsplit and starred forms Ruff skips.
    "PTH206-parenthesized-callee-and-trivia.py": "import os\n\np = \"a/b\"\n(p.split)(os.sep)\n"
                                                 "((p).split)(sep=(os.sep))\np.split(  # leading\n"
                                                 "    sep=os.sep  # trailing\n)\nq = p \\\n    .split(os.sep)\n"
                                                 "p.split(os.sep, 1)\np.split(sep=os.sep, maxsplit=1)\n"
                                                 "p.split(*[os.sep])\n",
    # PTH207: dir_fd is exempt only as a non-None keyword: None in parentheses, **kwargs and starred arguments are not read as dir_fd, and an aliased from-import is still resolved.
    "PTH207-dir-fd-none-forms.py": "import glob\nfrom glob import iglob as search\n\nglob.glob(\"*.py\", dir_fd=None)\n"
                                   "glob.glob(\"*.py\", dir_fd=(None))\nsearch(\"*.py\", dir_fd=((None)))\n"
                                   "glob.glob(\"*.py\", dir_fd=0)\nsearch(\"*.py\", dir_fd=-1)\n"
                                   "glob.glob(\"*.py\", **{\"dir_fd\": 1})\nglob.glob(\"*.py\", *[1, 2, 3])\n"
                                   "glob.glob(dir_fd=None, pathname=\"*.py\", recursive=True)\n",
    # PTH208: Ruff's type inference rather than value folding (bool operands, or-chains with floats, comparisons, division by zero, walrus of int()), with-item targets, starred-only arguments, and class attributes bound in a method or by an int-returning method.
    "PTH208-type-inference-and-class-scope.py": "import os\n\na = True or 1\nb = 1 or 2.5\nc = True & True\n"
                                                "d = 1 if True else 2.0\ne = (1 < 2) + 1\nf = 1 // 0\n"
                                                "g = (n := int(\"3\"))\nos.listdir(a)\nos.listdir(b)\nos.listdir(c)\n"
                                                "os.listdir(d)\nos.listdir(e)\nos.listdir(f)\nos.listdir(g)\n"
                                                "with 3 as h, int(3) as (i, j):\n    os.listdir(h)\n    os.listdir(i)\n"
                                                "os.listdir(*paths)\n\n\nclass C:\n    def m(self):\n        fd = 1\n\n"
                                                "    def meth(self) -> int: ...\n\n\nos.listdir(C.fd)\n"
                                                "os.listdir(C.meth)\n",
    # PTH210: A with-item unpacked into a tuple is not a path binding while a plain name target is, and parenthesized Path annotations on parameters and annotated assignments still type the name.
    "PTH210-with-target-and-annotation-forms.py": "from pathlib import Path\n\n\ndef f(p: (Path)):\n"
                                                  "    p.with_suffix(\"py\")\n\n\nwith Path() as s, Path() as (t, u):\n"
                                                  "    s.with_suffix(\"py\")\n    t.with_suffix(\"py\")\n"
                                                  "r: (Path) = x\nr.with_suffix(\"py\")\n",
    # PTH211: dir_fd is exempt only as a non-None keyword: None in parentheses, **kwargs, starred arguments and a missing argument list do not exempt, and an aliased from-import and a parenthesized callee are still resolved.
    "PTH211-dir-fd-keyword-forms.py": "import os\nfrom os import symlink as link\n\n"
                                      "os.symlink(\"a\", \"b\", dir_fd=None)\nos.symlink(\"a\", \"b\", dir_fd=(None))\n"
                                      "link(\"a\", \"b\", dir_fd=((None)))\nos.symlink(\"a\", \"b\", dir_fd=0)\n"
                                      "os.symlink(\"a\", \"b\", dir_fd=-1, target_is_directory=True)\n"
                                      "os.symlink(\"a\", \"b\", **{\"dir_fd\": 1})\n"
                                      "os.symlink(\"a\", \"b\", *[True, 3])\n(os.symlink)(src=\"a\", dst=\"b\")\n"
                                      "os.symlink()\n",
    # PYI001: Pins aliased and from-imports of the three typing modules, plus tuple, chained, annotated, underscore-prefixed and parenthesized targets (7 findings, identical to Ruff).
    "PYI001-alias-and-target-forms.pyi": "import typing as t\nimport typing_extensions as te\n"
                                         "from typing import TypeVar as TV, ParamSpec\n\n"
                                         "U = t.TypeVar(\"U\", bound=int)\nP = te.ParamSpec(\"P\")\nV = TV(\"V\")\n"
                                         "Q = ParamSpec(\"Q\")\n_S = t.TypeVar(\"_S\")\n"
                                         "W: t.TypeVar = t.TypeVar(\"W\")\nA = B = t.TypeVar(\"A\")\n"
                                         "E, F = t.TypeVar(\"E\")\n(C) = t.TypeVar(\"C\")\nD = (t.TypeVar(\"D\"))\n",
    # PYI002: Pins how Ruff flattens a same-operator chain and reports nested or parenthesized boolean operands whole, parenthesized left operands, and that elif tests are skipped (7 findings, identical to Ruff).
    "PYI002-nested-boolean-chains.pyi": "import sys\n"
                                        "if sys.version_info >= (3, 8) and sys.platform == \"linux\" and sys.maxsize > 5: ...\n"
                                        "if sys.version_info >= (3, 8) or sys.platform == \"linux\" and sys.maxsize > 5: ...\n"
                                        "if (sys.version_info >= (3, 8) and sys.platform == \"linux\") and sys.maxsize > 5: ...\n"
                                        "if (sys.version_info >= (3, 8) or sys.maxsize) and (sys.platform == \"linux\"): ...\n"
                                        "if sys.version_info >= (3, 8) and (sys.platform == \"a\" and sys.platform == \"b\"): ...\n"
                                        "if (sys.platform) == \"linux\" and (sys.version_info)[0] == 3: ...\n"
                                        "if sys.version_info >= (3, 8) and not sys.maxsize: ...\n"
                                        "elif sys.maxsize: ...\n",
    # PYI003: Pins that a comment holding a colon inside a `[:2]` slice and a comment between the operator and the comparator change nothing, while a string comparator and a step still report (2 findings, identical to Ruff).
    "PYI003-comments-inside-slice-and-after-operator.pyi": "import sys\nif sys.version_info[:  # a:b\n"
                                                           "    2] >= (3, 8): ...\nif sys.version_info[:2  # c:\n"
                                                           "    :] >= (3, 8): ...\nif (sys.version_info >=  # c\n"
                                                           "    (3, 8)): ...\nif (sys.version_info[0] ==  # c\n"
                                                           "    3): ...\nif (sys.version_info >=  # c\n"
                                                           "    \"3\"): ...\nif sys.version_info[:2:  # x:y\n"
                                                           "    1] >= (3, 8): ...\n",
    # PYI004: Pins that a comment holding a colon inside a `[:2]` slice and a parenthesized `[:(1)]` bound still report, while a step `[:2:1]` stays silent (3 findings, identical to Ruff).
    "PYI004-comment-colon-in-slice.pyi": "import sys\nif sys.version_info[:  # a:b\n    2] >= (3, 8, 1): ...\n"
                                         "if sys.version_info[:2  # c:\n    :] >= (3, 8, 1): ...\n"
                                         "if sys.version_info[:(1)] >= (3, 8, 1): ...\n"
                                         "if sys.version_info[:2:1] >= (3, 8, 1): ...\n",
    # PYI005: Stub-only rule, so this is a .pyi: aliased import, parenthesized operands, one-level and/or flattening, elif and negation and chained comparison skipped, [:2:] and [:(1)] slice forms; Ruff reports 6.
    "PYI005-boolop-paren-alias-elif.pyi": "import sys\nfrom sys import version_info as vi\n\n"
                                          "if (vi[:1] == (2, 7)) and sys.platform: ...\n"
                                          "elif sys.version_info[:2] == (3,): ...\n"
                                          "if sys.platform or sys.version_info[:2] == (3,) or vi[:1] == (): ...\n"
                                          "if sys.platform and (sys.version_info[:2] == (3,)): ...\n"
                                          "if sys.platform and (sys.platform or sys.version_info[:2] == (3,)): ...\n"
                                          "if not sys.version_info[:2] == (3,): ...\n"
                                          "if sys.version_info[:2] == (3,) == (3,): ...\n"
                                          "if sys.version_info[:2:] == (3,): ...\n"
                                          "if sys.version_info[:(1)] == ((2, 7)): ...\n",
    # PYI006: elif tests, one-level and/or flattening, parenthesized operand, aliased import, and look-alikes Ruff skips (subscripted left, reversed operands, chained comparison, while, assignment); Ruff reports 3.
    "PYI006-elif-boolop-alias.py": "import sys\nfrom sys import version_info as vi\n\n"
                                   "if sys.version_info >= (3, 9): ...\nelif vi > (3, 8): ...\n"
                                   "elif vi == (3, 7) or sys.platform: ...\nif sys.platform and (vi <= (3, 6)): ...\n"
                                   "if sys.platform and (sys.platform or vi != (3, 5)): ...\nif vi[0] > 3: ...\n"
                                   "if (3, 8) < vi: ...\nif vi > (3, 8) > (3, 7): ...\nwhile vi > (3, 8): ...\n"
                                   "x = vi > (3, 8)\n",
    # PYI007: Stub-only rule, so this is a .pyi: f-string, bytes and mixed f-string concatenation operands are reported while a parenthesized implicit concatenation is not, plus an or-chain operand, an elif skipped and a reversed comparison; Ruff reports 5.
    "PYI007-operand-kinds-boolop.pyi": "import sys\nfrom sys import platform as plat\n\nif plat == f\"linux\": ...\n"
                                       "if plat == b\"linux\": ...\nif plat == (\"lin\" \"ux\"): ...\n"
                                       "if plat == \"lin\" f\"ux\": ...\n"
                                       "if sys.platform == \"linux\" or sys.platform in (\"darwin\",): ...\n"
                                       "elif sys.platform > \"a\": ...\nif \"linux\" == sys.platform: ...\n"
                                       "if plat == (3): ...\n",
    # PYI008: Stub-only rule, so this is a .pyi: the literal is judged on its decoded value, so octal, hex and implicit-concatenation spellings of linux or win32 stay silent, while an unknown escape keeps its backslash, a raw string is not decoded, and named-escape and NUL-suffixed values are reported; Ruff reports 4.
    "PYI008-escapes-decoded.pyi": "import sys\nif sys.platform == \"\\linux\": ...\n"
                                  "if sys.platform == \"\\154inux\": ...\nif sys.platform == \"\\x6c\" \"inux\": ...\n"
                                  "if sys.platform == r\"\\x6cinux\": ...\nif sys.platform == \"\" \"linux\": ...\n"
                                  "if sys.platform == \"win\\x33\\x32\": ...\n"
                                  "if sys.platform == \"\\N{DIGIT THREE}\": ...\n"
                                  "if sys.platform == \"linux\\x00\": ...\n",
    # PYI009: Pins two-statement semicolon bodies (not reported), line-continued and trailing-semicolon single bodies (reported); stub rule, so the probe must keep its .pyi extension.
    "PYI009-pass-semicolon-and-continuation.pyi": "def a(): pass; x = 1\ndef b(): \\\n    pass\ndef c(): pass;\n"
                                                  "class D: pass; pass\nclass E:\n    pass  # trailing comment\n",
    # PYI010: Pins the end of a compound sole statement whose last code line has `#` inside a string, triple-quoted string or f-string before a trailing comment, and a backslash continuation before a comment line; stub rule, so the probe must keep its .pyi extension.
    "PYI010-hash-in-string-before-trailing-comment.pyi": "def a():\n    if x:\n        y = \"#\"  # c\ndef b():\n"
                                                         "    if x:\n        y = '''\n#b'''\n        # own\ndef c():\n"
                                                         "    if x:\n        y = f\"{z:{w}}#{'#'}\"  # c\ndef d():\n"
                                                         "    if x:\n        y \\\n          # weird\n",
    # PYI011: Pins math/sys constants used before their module-level import (accepted, -math.nan still reported) and a parameter named like the later-imported module (reported, as in Ruff); stub rule, so the probe must keep its .pyi extension.
    "PYI011-forward-import.pyi": "def p1(a: float = math.pi, b: float = -math.tau, c: object = sys.stdout, d: float = -math.nan) -> None: ...\n"
                                 "def p2(math, a: float = math.pi) -> None: ...\nimport math\nimport sys\n",
    # PYI012: Pins one-line semicolon class bodies (each pass reported, a lone pass left to PYI009); stub rule, so the probe must keep its .pyi extension.
    "PYI012-semicolon-forms.pyi": "class A: x: int; pass\nclass B: pass; x: int\nclass C: pass; pass\nclass D: pass\n"
                                  "class E: ...; pass\n",
    # PYI013: Pins that tuple statements starting with an ellipsis (`..., x`, `...,`, `(...,)`) are not reported.
    "PYI013-ellipsis-tuple-statement.pyi": "class A:\n    ..., x\n    y: int\nclass B:\n    ...,\n    y: int\n"
                                           "class C:\n    (...,)\n    y: int\n",
    # PYI014: Pins that a backslash continuation inside a 10-element list is not counted as an eleventh element.
    "PYI014-line-continuation-in-ten-element-list.pyi": "def f(a=[1, 2, 3, 4, 5, 6, 7, 8, 9, \\\n"
                                                        "    10], b=(1, 2, 3, 4, 5, 6, 7, 8, 9, \\\n"
                                                        "    10, 11)) -> None: ...\n",
    # PYI015: Pins that parenthesized `__all__`/`__slots__`/`__match_args__` targets and parenthesized Final/TypeAlias annotations are exempt like the bare forms.
    "PYI015-parenthesized-special-target-and-annotation.pyi": "from typing import Final, TypeAlias\n"
                                                              "(__all__) = foo + bar\nx: (Final) = foo\n"
                                                              "X: (TypeAlias) = foo()\nclass A:\n"
                                                              "    (__slots__) = a + b\n"
                                                              "    (__match_args__): tuple = foo()\n",
    # PYI016: Pins Union tuple slices: flattened when the parenthesized tuple is the whole slice, one member beside other members or with a trailing comma, in annotations and expressions.
    "PYI016-union-tuple-slice.py": "from typing import Union\na: Union[(int, int)]\nb: Union[(int, int),]\n"
                                   "c: Union[((int, int))]\nd: Union[(int, int), str]\n"
                                   "e: Union[(int, int), (int, int)]\nf: Union[(int, str), int]\ng: Union[  # c\n"
                                   "    (int, int)\n]\na = Union[(int, int)]\nb = Union[(int, int),]\n"
                                   "c = Union[((int, int))]\nd = Union[(int, int), str]\n"
                                   "e = Union[(int, int), (int, int)]\nf = Union[(int, str), int]\n"
                                   "g = Union[((int, int)), str]\nh = Union[(((int, int)),)]\n",
    # PYI017: chains mixing names with attribute, item and parenthesized targets report once per statement
    "PYI017-chained-attribute-and-item-targets.pyi": "a = b.c = 1\na[0] = b = 1\na = b[0] = 1\na = (b) = 1\n",
    # PYI018: locals() in the function itself, in a nested function, and a bare locals reference
    "PYI018-locals-calls.py": "from typing import TypeVar, ParamSpec, TypeVarTuple\ndef f():\n"
                              "    _A = TypeVar(\"_A\")\n    x = _A\n    _A = TypeVar(\"_A\")\n    return locals()\n"
                              "def g():\n    _B = TypeVar(\"_B\")\n    def h():\n        return locals()\n"
                              "    return h\ndef k():\n    _C = TypeVar(\"_C\")\n    locals\n",
    # PYI019: report end with parenthesized, doubly parenthesized and spaced return annotations, colons in defaults and strings
    "PYI019-parenthesized-return-ranges.pyi": "from typing import TypeVar, Callable, Literal\n_S = TypeVar(\"_S\")\n"
                                              "class A:\n    def a(self: _S, x: \"a)b\") -> _S: ...\n"
                                              "    def b(self: _S, x=lambda: 1) -> _S: ...\n"
                                              "    def c(self: _S, x: dict[str, int] = {\"a\": 1}) -> _S: ...\n"
                                              "    def d(self: _S) -> (_S): ...\n    def e(self: _S) -> (\n        _S\n"
                                              "    ): ...\n    def f(self: _S) -> ((_S)): ...\n"
                                              "    def g(self: _S) -> (_S) | (int): ...\n"
                                              "    def h(self: _S) -> ((int) | _S): ...\n    def i(\n"
                                              "        self: _S,\n    ) -> _S: ...\n"
                                              "    def j(self: _S) -> Callable[[int], _S]: ...\n"
                                              "    def k(self: _S) -> \"Callable[[int], _S]\": ...\n"
                                              "    def l(self: _S): ...\n    def m(self: _S) -> Literal[\"a:b\"]: ...\n"
                                              "    def n(self: _S, x=(1)): ...\n"
                                              "    def o(self: _S) -> (_S | None): ...\n"
                                              "    def p(self: _S) -> ( _S ) : ...\n",
    # PYI020: escaped, raw, bytes and line-continued quoted annotations
    "PYI020-escaped-and-prefixed-strings.pyi": "from typing import Annotated, Callable, Literal, TypeVar, cast, Final, Any\n"
                                               "import typing_extensions as te\na: \"int\"\nb: rb\"int\"\n"
                                               "c: Rb\"int\"\nd: R\"int\"\ne: \"int\\\n\"\n",
    # PYI021: Pins which string forms count as a docstring (plain, raw, u prefix, implicit concatenation, parenthesized, one-line bodies) and which do not (b, f prefixes, concatenation with an f-string, tuples, binary operations); Ruff's own fixtures cover only plain triple-quoted strings. Must stay a .pyi file: the extension is what makes the rule apply.
    "PYI021-docstring-forms.pyi": "def a() -> None:\n    r\"\"\"raw doc\"\"\"\ndef b() -> None:\n"
                                  "    u\"\"\"unicode doc\"\"\"\ndef c() -> None:\n    R'''raw single'''\n"
                                  "def d() -> None:\n    b\"\"\"bytes not doc\"\"\"\ndef e() -> None:\n"
                                  "    f\"\"\"fstring not doc\"\"\"\ndef g(x: int) -> None:\n    f\"{x}\"\n"
                                  "def h() -> None:\n    \"implicit\" \"concat\"\ndef i() -> None:\n"
                                  "    \"implicit\" f\"fconcat\"\ndef j() -> None:\n    rb\"bytes raw\"\n"
                                  "def k() -> None:\n    U\"upper unicode\"\ndef l() -> None:\n    f\"\" \"x\"\n"
                                  "def m() -> None:\n    '''a''' '''b'''\n    ...\ndef a() -> None:\n"
                                  "    (\"\"\"parenthesized doc\"\"\")\ndef b() -> None:\n    ((\"double paren\"))\n"
                                  "def c() -> None:\n    (\"implicit\"\n     \"concat\")\ndef d() -> None:\n    (\n"
                                  "        # comment inside\n        \"doc with comment\"\n    )\ndef e() -> None:\n"
                                  "    (\"a\", \"b\")\ndef f() -> None:\n    (\"a\" + \"b\")\ndef g() -> None:\n"
                                  "    (\"\"\"x\"\"\").upper()\ndef a() -> None: \"\"\"one-liner doc\"\"\"\n"
                                  "class B: \"\"\"one-liner class doc\"\"\"\ndef c() -> None: \"\"\"doc\"\"\"; ...\n"
                                  "def d() -> None: ...; \"\"\"not doc\"\"\"\nclass E: x: int; \"\"\"not doc\"\"\"\n"
                                  "def f(): \"single quote doc\"\n",
    # PYI024: Pins attribute references reported in delete, store and augmented contexts while bare-name augmented targets are not, starred attributes inside lists and expression lists, and a match value pattern reported on its collections.namedtuple prefix; Ruff's fixtures cover none of these.
    "PYI024-store-del-splat-case.py": "import collections\nfrom collections import namedtuple\n"
                                      "del collections.namedtuple\ncollections.namedtuple += 1\nnamedtuple += 1\n"
                                      "(namedtuple) -= 1\nx = 1\nx += namedtuple\n"
                                      "a = [*collections.namedtuple(\"a\", \"b\")._fields]\n"
                                      "b = *collections.namedtuple, 1\nmatch v:\n"
                                      "    case collections.namedtuple.attr.deep:\n        pass\n"
                                      "    case collections.namedtuple2:\n        pass\n",
    # PYI025: Pins that a plain import statement aliasing collections.abc.Set is not a from-import binding and stays silent, beside from-import aliases that are reported; Ruff's fixtures have no import statement of the member.
    "PYI025-import-statement.py": "import collections.abc.Set as S\nimport collections.abc.Set\n"
                                  "from collections.abc import Set as S2\n"
                                  "from collections.abc import Set as AbstractSet\n",
    # PYI026: Pins the enum exemption for `X = None` through parenthesized bases, `(Enum)`, `(Enum)[int]` and `(enum.Flag)`, which Ruff sees through and the first port did not, plus a union still reported in an enum body. Must stay a .pyi file.
    "PYI026-parenthesized-enum-base.pyi": "from enum import Enum\nimport enum\nclass A((Enum)):\n    X = None\n"
                                          "class B((Enum)[int]):\n    Y = None\nclass C((enum.Flag)):\n    Z = None\n"
                                          "    W = int | None\nclass D((Base)):\n    V = None\n",
    # PYI029: Ruff counts only positional parameters and keyword-only ones: *args/**kwargs and a no-parameter method are still reported, a keyword-only or second positional parameter is not.
    "PYI029-varargs-kwonly-and-positional-only.pyi": "class A:\n    def __str__(self, *args) -> str: ...\n"
                                                     "    def __repr__(self, **kw) -> str: ...\nclass B:\n"
                                                     "    def __str__() -> str: ...\n"
                                                     "    def __repr__(*args: int, **kw: str) -> str: ...\nclass C:\n"
                                                     "    def __str__(self, *, k) -> str: ...\n"
                                                     "    def __repr__(self, /) -> str: ...\n"
                                                     "    def __str__(self, /, x) -> str: ...\n"
                                                     "    def __repr__(self, *args, k) -> str: ...\n",
    # PYI030: Escaped and concatenated string annotations are reported at the whole string only when the last resolvable name is Literal, as Ruff's shared-range name resolution does.
    "PYI030-complex-string-annotations.py": "from typing import Literal\na: \"Literal['a'] | Literal[\\\"b\\\"]\"\n"
                                            "b: \"Literal[1] \" \"| Literal[2]\"\n"
                                            "c: \"Literal[1] | Literal[2] | int\" \"\"\n"
                                            "d: \"list[int] | Literal[1]\" \"\"\ne: \"Literal[1] | Literal[2]\"\n",
    # PYI032: Only regular parameters count: with posonly parameters before `/`, the method is reported only when exactly two regular parameters remain; keyword-only parameters do not matter.
    "PYI032-positional-only-and-keyword-only-counting.py": "from typing import Any\nclass A:\n"
                                                           "    def __eq__(x, y, /, self, o: Any) -> bool: ...\n"
                                                           "    def __ne__(x, /, o: Any) -> bool: ...\n"
                                                           "    def __eq__(self, o: Any, /, p) -> bool: ...\n"
                                                           "    def __ne__(self, *, o: Any) -> bool: ...\n"
                                                           "    def __eq__(*, self, o: Any) -> bool: ...\n"
                                                           "    def __ne__(self, o: Any): pass\n",
    # PYI033: Pins Ruff's two regexes on edge forms: trailing whitespace, a bare `#` after type:, ignore-prefixed words, case and spacing variants.
    "PYI033-type-comment-regex-edges.pyi": "x = 1  # type: \ny = 2  # type: #foo\nz = 3  # type: ignored\n"
                                           "w = 4  # type: ignorance\nv = 5  #type:int\nu = 6  # type:ignore[x] # c\n"
                                           "t = 7  # type: int # type: ignore\ns = 8  # type:\tint\n"
                                           "r = 9  # Type: int\nq = 0  # type :int\np = 1  # type:#\n"
                                           "o = 2  #   type:  i\nn = 3  # type: ignore  \nm = 4  # type:  \t \n"
                                           "l = 5  # type:ig\nk = 6  # type: IGNORE\nj = 7  # type: ignore#x\n",
    # PYI034: Metaclass and Iterator inheritance through a subscripted same-file base class (Meta[int], It[int]), which Ruff follows via map_subscript.
    "PYI034-subscripted-same-file-bases.py": "from typing import Generic, Iterator, TypeVar\nT = TypeVar(\"T\")\n"
                                             "class Meta(type, Generic[T]): ...\nclass A(Meta[int]):\n"
                                             "    def __enter__(self) -> A: ...\nclass It(Iterator[T]): ...\n"
                                             "class B(It[int]):\n    def __iter__(self) -> Iterator[int]: ...\n",
    # PYI035: __slots__ and __match_args__ under if/else in a class, in a method body (not reported), and in a nested class; __all__ inside a method and a class.
    "PYI035-special-variable-scopes.pyi": "class Foo:\n    if True:\n        __slots__: tuple[str, ...]\n"
                                          "        __match_args__: tuple[str, ...]\n    else:\n"
                                          "        __slots__: tuple[str, ...] = ()\n    def method(self) -> None:\n"
                                          "        __slots__: tuple[str, ...]\n        __all__: list[str]\n"
                                          "    class Inner:\n        __slots__: tuple[str, ...]\n"
                                          "        __all__: list[str]\n",
    # PYI036: Attribute-form Union with None first, a single tuple slice Union[(X, None)], and trailing-comma slices (type[BaseException, ], Union[(X, None),]) that Ruff reads differently.
    "PYI036-union-attribute-and-tuple-slice.py": "import typing as t\nfrom typing import Union\n"
                                                 "from types import TracebackType\nclass A:\n"
                                                 "    def __exit__(self, a: t.Union[None, type[BaseException]], b: Union[(BaseException, None)], c: t.Union[None, TracebackType]) -> None: ...\n"
                                                 "class B:\n"
                                                 "    def __exit__(self, a: type[BaseException, ] | None, b: Union[(BaseException, None),], c: Union[(TracebackType, int)]) -> None: ...\n",
    # PYI041: Tuples inside a union: unpacked only as the sole slice of Union (also double-parenthesized), not after |, not among other Union arguments, not with a trailing comma.
    "PYI041-tuple-members.py": "from typing import Union\ndef f1(x: int | (float, complex)): ...\n"
                               "def f2(x: Union[int, (float, str)]): ...\ndef f3(x: Union[(int, float)]): ...\n"
                               "def f4(x: Union[(int, float),]): ...\ndef f5(x: Union[((int, float))]): ...\n",
    # PYI042: Aliased from-import and module alias, parenthesized target and annotation, generic PEP 695 name, an attribute target, a value-less annotation and a locally shadowed TypeAlias, none of which Ruff's fixtures cover.
    "PYI042-aliased-paren-generic-shadowed.py": "from typing import TypeAlias as TA\nimport typing_extensions as te\n\n"
                                                "snake_aliased: TA = int\n(snake_paren): te.TypeAlias = str\n"
                                                "snake_in_parens: (TA) = bytes\ntype snake_pep695[T] = list[T]\n"
                                                "type CamelPep695[T] = list[T]\n\n\nclass Holder:\n"
                                                "    snake_member: TA = int\n    self_attr: TA\n\n\ndef func():\n"
                                                "    TA = int\n    not_an_alias: TA = int\n",
    # PYI043: Non-ASCII letters before or in place of the lowercase letter (`_xéT`, `_ÉT`), digit suffixes 0 and 9, a digit before the `T` (`_0T`), a letter after the `T` (`_aTx`), a double-underscore prefix (`__aT`) and bare `_T`, none of which Ruff's fixtures cover.
    "PYI043-non-ascii-and-digit-suffixes.py": "from typing import TypeAlias\n\néclair: TypeAlias = int\n"
                                              "_éclair: TypeAlias = int\nΔx: TypeAlias = int\n_xéT: TypeAlias = int\n"
                                              "_ÉT: TypeAlias = int\n_zT0: TypeAlias = int\n_zT9: TypeAlias = int\n"
                                              "_0T: TypeAlias = int\n_aTx: TypeAlias = int\n__aT: TypeAlias = int\n"
                                              "_T: TypeAlias = int\n",
    # PYI044: Relative `__future__` imports at two dot levels and with a space after the dots (reported), a dotted relative module ending or continuing past `__future__` (not reported), and a statement followed by `;` (span stops before it); Ruff's fixture has only absolute imports.
    "PYI044-relative-future-levels.pyi": "from ..__future__ import annotations\nfrom . __future__ import annotations\n"
                                         "from .__future__.sub import annotations\n"
                                         "from ...pkg.__future__ import annotations\n"
                                         "from __future__ import annotations; import sys\n",
    # PYI045: A parenthesized inner subscript (`(Iterable[int])[str]`, `((AsyncIt)[int])[str]`, missed before review), a triple subscript (not reported), and names imported only after the class: a later from-import and alias (reported), a later import then rebound, and a later import of a different name (not reported).
    "PYI045-paren-subscripts-and-later-import.py": "class A:\n    def __iter__(self) -> (Iterable[int])[str]: ...\n"
                                                   "    def __aiter__(self) -> ((AsyncIt)[int])[str]: ...\n"
                                                   "    def __iter__(self) -> ((Iterable[int])[str])[bytes]: ...\n"
                                                   "    def __iter__(self) -> Rebound[int]: ...\n"
                                                   "    def __aiter__(self) -> NotIterable[int]: ...\n\n\n"
                                                   "from typing import Iterable\n"
                                                   "from collections.abc import AsyncIterable as AsyncIt\n"
                                                   "from collections.abc import Iterable as Rebound\nRebound = list\n"
                                                   "from typing import Iterator as NotIterable\n",
    # PYI046: Pins which class bases count: Protocol after Generic or another base and next to a metaclass keyword counts, while Protocol only as metaclass= or inside a starred base does not.
    "PYI046-bases-with-star-and-metaclass.py": "import typing\nfrom typing import Generic, Protocol, TypeVar\n\n"
                                               "T = TypeVar(\"T\")\nbases = ()\n\n\nclass Base: ...\n\n\n"
                                               "class M(type): ...\n\n\nclass _A(Generic[T], Protocol):\n    x: T\n\n\n"
                                               "class _B(Base, typing.Protocol[T]):\n    x: T\n\n\n"
                                               "class _C(*bases, Protocol):\n    x: int\n\n\n"
                                               "class _D(Protocol, metaclass=M):\n    x: int\n\n\n"
                                               "class _E(Base, metaclass=Protocol):\n    x: int\n\n\n"
                                               "class _F(*[Protocol]):\n    x: int\n",
    # PYI047: Pins Ruff's report of unused underscore-prefixed comprehension variables inside stub type alias values (public aliases, type statements, nested and repeated targets, class bodies, parenthesized targets) and its silence for attribute targets, plain annotations and walrus names.
    "PYI047-stub-alias-comprehension-variable.pyi": "from typing import TypeAlias\nimport typing\n\n"
                                                    "Pub: TypeAlias = list[[0 for _Q in ()]]\n"
                                                    "type T1 = [0 for _R in ()]\nx.y: TypeAlias = [0 for _S in ()]\n"
                                                    "Pub2: TypeAlias = [_U for _U in ()]\n"
                                                    "Pub3: TypeAlias = [0 for _V in () for _V in ()]\n"
                                                    "Pub4: TypeAlias = (lambda: [0 for _W in ()])\n"
                                                    "Pub5: int = [0 for _X in ()]\n"
                                                    "Pub6: TypeAlias = [0 for _Y, _Z in ()]\nclass K:\n"
                                                    "    Pub7: TypeAlias = {0: 1 for _A in ()}\n"
                                                    "    x = [0 for _B in ()]\n"
                                                    "Pub8: TypeAlias = [0 for q in () if (_C := 1)]\n"
                                                    "Pub9: TypeAlias = [[0 for _D in ()] for e in ()]\n"
                                                    "(Pub10): TypeAlias = [0 for _E in ()]\n",
    # PYI048: Pins that a column-zero comment between two body statements does not end the body, and that trailing and inline comments are not statements.
    "PYI048-comments-between-statements.pyi": "def a():\n    ...\n# column zero comment\n    ...\n\n"
                                              "def b():  # comment\n    ...\n    # trailing comment\n\n"
                                              "def c(): ...  # one\n\ndef d():\n        ...\n        # deep\n"
                                              "        ...\n",
    # PYI049: Pins Ruff's report of unused underscore-prefixed comprehension variables in a stub's functional TypedDict call and in a TypedDict class's decorators, keywords and type parameters, and its silence for class bodies, annotated and chained assignments and non-TypedDict classes.
    "PYI049-stub-typed-dict-comprehension-variable.pyi": "from typing import TypedDict\n\ndef deco(*a): ...\n\n"
                                                         "Pub = TypedDict(\"Pub\", {\"x\": int for _a in ()})\n"
                                                         "@deco([0 for _b in ()])\nclass Pub2(TypedDict):\n"
                                                         "    x = [0 for _c in ()]\n"
                                                         "class Pub3(TypedDict, **{k: 1 for _d in ()}):\n    y: int\n"
                                                         "Pub4: int = TypedDict(\"Pub4\", {\"x\": int for _e in ()})\n"
                                                         "Pub5 = Pub6 = TypedDict(\"Pub5\", {\"x\": int for _f in ()})\n"
                                                         "class Pub7(dict, x=[0 for _g in ()]): ...\n"
                                                         "class Pub8[T: [0 for _h in ()]](TypedDict): ...\n",
    # PYI050: implicitly concatenated, parenthesised, padded and escaped string annotations are parsed and reported like Ruff
    "PYI050-string-annotation-forms.pyi": "from typing import NoReturn\n"
                                          "def f(x: \"NoReturn\" \"\", y: \"(NoReturn)\", z: \"NoReturn  \", w: \"\\tNoReturn\"): ...\n",
    # PYI051: a parenthesised tuple is unpacked only when it is the whole Literal slice, never when nested among other values
    "PYI051-nested-literal-tuples.py": "from typing import Literal\nx1: Literal[(\"a\", \"b\"), \"c\"] | str\n"
                                       "x2: Literal[(\"a\", \"b\")] | str\nx3: Literal[\"a\",] | str\n"
                                       "x4: Literal[(\"a\",)] | str\nx5: Literal[((\"a\"))] | str\n",
    # PYI052: negated booleans, double negation and negated sys constants are not simple; parenthesised complex sums, None and ellipsis elements are
    "PYI052-container-element-forms.pyi": "import sys, math\na1 = [-True]\na2 = [--1]\na3 = [-sys.maxsize]\n"
                                          "a4 = (1, -math.inf)\na5 = -(-1)\na6 = [(1 + 2j)]\na7 = [(None), (...)]\n",
    # PYI053: NamedTuple keyword fields, legacy TypedDict keywords and extra_items are type definitions; TypedDict total= is not
    "PYI053-namedtuple-typeddict-keywords.pyi": "from typing import NamedTuple, TypedDict\n"
                                                "N1 = NamedTuple(\"N\", a=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\")\n"
                                                "N2 = NamedTuple(\"N\", [(\"a\", \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\")], b=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\")\n"
                                                "T1 = TypedDict(\"T\", {\"a\": \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"}, total=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\")\n"
                                                "T2 = TypedDict(\"T\", a=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\")\n"
                                                "T3 = TypedDict(\"T\", {\"a\": int}, extra_items=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\")\n",
    # PYI054: Escaped and implicitly concatenated string annotations (also nested ones) report every number at the whole string when it is over ten characters; raw strings and unknown escapes are plain.
    "PYI054-escaped-or-concatenated-string-annotation.pyi": "from typing import Literal\na: \"Literal[1]\" \"\"\n"
                                                            "b: r\"Literal[\\x31]\"\nc: \"Literal[\\q1]\"\n"
                                                            "d: u\"Literal[\\x31]\"\ne: \"1\" \"2\"\n"
                                                            "f: \"list['Literal[1]' '']\"\ng: (\"Literal[1]\"\n"
                                                            "    \"\")\nh: \"Literal[12345678901]\" \"\"\n",
    # PYI055: In escaped or concatenated string annotations only `|` unions are checked (Ruff cannot resolve `Union` there); a concatenated string nested in a plain one is also handled.
    "PYI055-escaped-or-concatenated-string-union.pyi": "from typing import Union\n"
                                                       "a: \"Union[type[int], type[str]]\" \"\"\n"
                                                       "b: \"type[int] | type[str]\" \"\"\n"
                                                       "c: \"Union[type[int], \\x74ype[str]]\"\n"
                                                       "d: \"type[int] | type[str] | None\" \"\"\n"
                                                       "e: \"Union[type[int] | type[str], int]\" \"\"\n"
                                                       "f: \"type[int] | Union[type[str], type[bytes]]\" \"\"\n"
                                                       "g: \"list['type[int]' '|type[str]']\"\n",
    # PYI056: The list_splat-callee grammar shape, a comment after the star, and calls inside string annotations report at the attribute.
    "PYI056-splat-callee-and-string-annotation.py": "x = [*(__all__).append(1)]\n"
                                                    "print(*__all__.append(1), *__all__.extend(2))\ny = [* # comment\n"
                                                    "     __all__.remove(1)]\nz: \"(__all__.remove)(1)\"\n"
                                                    "w: \"list['(__all__).extend(1)']\"\n",
    # PYI057: Union, mapping-key, keyword and value patterns, and dotted names that continue past ByteString, report the qualified-name prefix.
    "PYI057-match-pattern-positions.py": "import typing\nimport collections.abc\nmatch x:\n"
                                         "    case typing.ByteString | bytes: ...\n"
                                         "    case {typing.ByteString: 1}: ...\n    case C(x=typing.ByteString): ...\n"
                                         "    case typing.ByteString.x: ...\n"
                                         "    case collections.abc.ByteString.y(): ...\n",
    # PYI058: Bare tuple expression statements (`"a", "b"`, `..., ...`, `"x",`) and tuple returns after a yield, which Ruff treats as non-simple bodies.
    "PYI058-tuple-expression-statements.py": "from typing import Generator\n\nclass A:\n"
                                             "    def __iter__(self) -> Generator:\n        \"a\", \"b\"\n"
                                             "    def __iter__(self) -> Generator:\n        ..., ...\n"
                                             "    def __iter__(self) -> Generator:\n        yield 1, 2\n"
                                             "    def __iter__(self) -> Generator:\n        x: int\n"
                                             "    def __iter__(self) -> Generator:\n        yield 1\n"
                                             "        return None, None\n    def __iter__(self) -> Generator:\n"
                                             "        yield 1\n        return (None,)\n"
                                             "    def __iter__(self) -> Generator:\n        \"x\",\n",
    # PYI059: Generic hidden behind a star, call, conditional or walrus (not recognized) and bases mixed with keyword, **kwargs and trailing starred arguments.
    "PYI059-wrapped-and-keyword-bases.py": "from typing import Generic, TypeVar, Sized\nT = TypeVar(\"T\")\n\n"
                                           "class A(*Generic, Sized): ...\nclass B(f(Generic[T]), Sized): ...\n"
                                           "class C(Generic[T] if x else Sized, Sized): ...\n"
                                           "class D((G := Generic)[T], Sized): ...\n"
                                           "class E(Generic[T], Sized, metaclass=M, **kw): ...\n"
                                           "class F(Generic[T], *(), ): ...\nclass G(Generic[T], lambda: 1): ...\n"
                                           "class H(Sized, Generic[T], metaclass=M): ...\n"
                                           "class I(Generic[T], foo=1, *b): ...\n",
    # PYI061: Literal[None] inside string annotations nested two and three levels deep, with escaped quotes, which Ruff parses and its fixtures do not cover.
    "PYI061-nested-string-annotations.py": "from typing import Literal, Optional\n\n"
                                           "b: \"Optional[\\\"Optional['Literal[None]']\\\"]\"\n"
                                           "c: \"list['Literal[None, 1]']\"\nd: \"Literal['Literal[None]']\"\n"
                                           "e: \"Optional['''Optional[\\\"Literal[None]\\\"]''']\"\n"
                                           "f: \"Optional['''Optional['Literal[None, 1]']''']\"\n",
    # PYI062: Duplicate Literal members inside string annotations nested two and three levels deep, which Ruff parses and its fixtures do not cover.
    "PYI062-nested-string-annotations.py": "from typing import Literal, Optional\n\na: \"Optional['Literal[1, 1]']\"\n"
                                           "b: \"list['''Optional[\\\"Literal['x', 'x']\\\"]''']\"\n"
                                           "c: \"Literal['Literal[1, 1]', 2]\"\n",
    # PYI063: Compatibility low-line characters normalise to `_` in identifiers, so `_＿x` is a double-underscore name and `__x＿` ends with a double underscore; Ruff's fixtures use only ASCII names.
    "PYI063-nfkc-underscores.py": "def a(_＿x): ...\ndef b(__x_＿): ...\ndef c(_﹏x, y): ...\ndef d(_︳x): ...\n"
                                  "def e(_x＿): ...\ndef f(__x﹍_): ...\nclass A:\n    def m(self, _＿y): ...\n",
    # PYI064: Comments, line continuations and a trailing comma followed by a comment inside the brackets: a trailing comma makes Ruff see a tuple (not reported) and comments must not change the element count; Ruff's fixtures have only one such comment.
    "PYI064-comments-inside-subscript.py": "from typing import Final, Literal\n\nx: Final[\n    Literal[\n"
                                           "        1  # c\n    ]\n] = 1\n\ny: Final[Literal[1]] = (\n    1\n)\n\n"
                                           "z: Final[Literal[  # c1\n    \"a\"\n    # c2\n]] = \"a\"; w = 2\n\n"
                                           "v : Final [ Literal [ 1 ] ]   =   1  # trailing\nu: Final[Literal[1]] \\\n"
                                           "    = 1\n\nt: Final[Literal[1]]; s: Final[Literal[2]] = 2\n"
                                           "if True: r: Final[Literal[1]] = 1\nq: Final[Literal[1,  # c\n]] = 1\n",
    # PYI066: Ruff checks only the direct values of the test's top-level boolean operation: mixed and/or chains, parenthesised sub-chains and `not` must not reach nested comparisons; Ruff's fixtures have no boolean chains.
    "PYI066-mixed-boolean-chains.pyi": "import sys\nimport typing\n\n"
                                       "if typing.TYPE_CHECKING and sys.version_info < (3, 9):\n    def a(): ...\n"
                                       "else:\n    def a(x): ...\n\n"
                                       "if not typing.TYPE_CHECKING and sys.version_info < (3, 9):\n    def b(): ...\n"
                                       "else:\n    def b(x): ...\n\n"
                                       "if sys.version_info < (3, 9) and not typing.TYPE_CHECKING:\n    def c(): ...\n"
                                       "else:\n    def c(x): ...\n\n"
                                       "if (sys.version_info < (3, 9) and typing.TYPE_CHECKING) and sys.platform == \"x\":\n"
                                       "    def d(): ...\nelse:\n    def d(x): ...\n\n"
                                       "if sys.platform == \"x\" and sys.platform == \"y\" and sys.version_info < (3, 9) or sys.platform == \"z\":\n"
                                       "    def e(): ...\nelse:\n    def e(x): ...\n\n"
                                       "if (sys.version_info < (3, 9) or sys.platform == \"x\") and sys.platform == \"y\":\n"
                                       "    def f(): ...\nelse:\n    def f(x): ...\n\n"
                                       "if sys.platform == \"x\" and sys.version_info < (3, 9) and sys.platform == \"y\" and sys.version_info < (3, 8):\n"
                                       "    def g(): ...\nelse:\n    def g(x): ...\n",
    # RSE102: Pins Ruff's class-scope walk through three levels and through a parenthesized middle segment, a non-class middle member, and global and nonlocal declarations that hand the decision to the naming heuristic. Ruff's fixture covers none of these.
    "RSE102-class-walk-global-nonlocal.py": "class A:\n    class B:\n        class C:\n            def Foo(self):\n"
                                            "                pass\n\n            class bar(Exception):\n"
                                            "                pass\n\n        def Make(self):\n            pass\n\n"
                                            "    B2 = B\n\n\nraise A.B.C.Foo()\nraise A.B.C.bar()\nraise (A.B).Make()\n"
                                            "raise A.B2.Make()\n\n\nclass lowc(Exception):\n    pass\n\n\ndef Upfn():\n"
                                            "    pass\n\n\ndef uses_global():\n    global lowc, Upfn\n"
                                            "    raise lowc()\n    raise Upfn()\n\n\ndef outer():\n    def Fn():\n"
                                            "        pass\n\n    def inner():\n        nonlocal Fn\n"
                                            "        raise Fn()\n\n    def shadowing():\n        def Fn():\n"
                                            "            pass\n\n        raise Fn()\n\n    return inner, shadowing\n",
    # RUF001: pins that plain-string annotations (through parentheses and concatenation) are skipped after the future import while nested, union and non-annotation strings are still checked
    "RUF001-string-annotation-under-future-import.py": "from __future__ import annotations\n\nfrom typing import List\n"
                                                       "\nx: \"Typе\" = 1\ny: List[\"Fоo\"] = []\nz: (\"Pаren\") = 1\n"
                                                       "w: \"Cоn\" \"cat\" = 1\nt: \"Fоo\" | None = None\n"
                                                       "v = \"vаlue\"\n\n\n"
                                                       "def f(a: \"Pаram\", b: \"Dеf\" = \"dеfault\", *args: \"Stаr\", **kw: \"Kwаrg\") -> \"Rеt\":\n"
                                                       "    q: \"Lоcal\" = 1\n    return lambda: \"lаmbda\"\n\n\n"
                                                       "class K:\n    attr: \"Attrіbute\"\n",
    # RUF002: pins that a first statement that is a tuple of strings is not a docstring
    "RUF002-tuple-statement-is-not-docstring.py": "def f():\n    \"dоc\",\n\n\ndef g():\n    \"dоc\" \"е\", 1\n",
    # RUF003: pins that consecutive one-letter look-alike words in text with an unambiguous letter are each reported
    "RUF003-consecutive-one-letter-words.py": "# л а а а\n# л о-о-о\nx = 1  # л (а) [о]\n",
    # RUF005: Pins that a reducible `+` inside a string Ruff reads as a type (annotations, nested generic strings, cast, TypeVar bound, implicit concatenation, escapes) is reported at Ruff's span, one per `+`.
    "RUF005-string-type-definitions.py": "from typing import List, cast, TypeVar\n"
                                         "def f(a: \"foo + [1]\") -> \"[1] + foo\": ...\nx: \"foo + [1]\" = foo + [2]\n"
                                         "y: List[\"foo + [1]\"] = []\nz = cast(\"(1,) + foo\", w)\n"
                                         "T = TypeVar(\"T\", bound=\"[1] + foo\")\nv: \"[1] + foo | bar + (2,)\" = 1\n"
                                         "u: \"Literal['[1] + foo']\" = 1\ns: \"foo + 1\" = 1\n"
                                         "t: \"1 + [1] + foo\" = 1\nr: \"  [1] + foo\" = 1\nq: \"[1]\" \"+ foo\" = 1\n"
                                         "p: \"[1] + \\x66oo\" = 1\n",
    # RUF006: Pins that Ruff's comprehension-scope pass reports the task call once per unused comprehension variable in the assignment, even when the task variable is awaited, and twice when it is not.
    "RUF006-unused-comprehension-variable.py": "import asyncio\n\n\nasync def f(urls):\n"
                                               "    t = asyncio.create_task(asyncio.gather(*[fetch() for _ in urls]))\n"
                                               "    await t\n\n\nasync def g(urls):\n"
                                               "    t = asyncio.create_task(asyncio.gather(*(fetch(u) for u in urls)))\n"
                                               "    await t\n\n\nasync def h(urls):\n"
                                               "    asyncio.create_task(asyncio.gather(*[fetch() for _ in urls]))\n\n\n"
                                               "async def i(urls):\n"
                                               "    t = asyncio.create_task(asyncio.gather(*[fetch() for _ in urls]))\n",
    # RUF007: Pins the successor test across decimal carries (9 to 10, 99 to 100, 1099 to 1100) and non-decimal literals (0x9 to 0xa, 0o7 to 0o10, 0b1 to 0B10).
    "RUF007-start-carry.py": "zip(x[9:], x[10:])\nzip(x[19:], x[20:])\nzip(x[99:], x[100:])\nzip(x[999:], x[1000:])\n"
                             "zip(x[109:], x[110:])\nzip(x[1099:], x[1100:])\nzip(x[9:], x[0:])\nzip(x[10:], x[9:])\n"
                             "zip(x[9:], x[9:])\nzip(x[39:], x[40:])\nzip(x[49:], x[41:])\nzip(x[0x9:], x[0xa:])\n"
                             "zip(x[0o7:], x[0o10:])\nzip(x[0b1:], x[0B10:])\nzip(x[0X1F:], x[32:])\n"
                             "zip(x[1_9:], x[2_0:])\n",
    # RUF008: A parenthesized tuple slice of Union or Annotated is a tuple for Ruff's immutable-annotation check, and bare frozendict is not a builtin at the default target version; Ruff's tests cover neither.
    "RUF008-tuple-slice-and-frozendict.py": "from dataclasses import dataclass\nfrom typing import Annotated, Union\n\n"
                                            "\n@dataclass\nclass A:\n    a: Union[(int, str)] = []\n"
                                            "    b: Union[((int, str))] = []\n    c: Union[(int, list)] = []\n"
                                            "    d: Annotated[(int, 1)] = []\n    e: Annotated[(list, 1)] = []\n"
                                            "    f: frozendict = []\n",
    # RUF009: frozen= follows Ruff's syntactic truthiness (-1 and an opaque f-string do not freeze, [1] and list("a") do), parentheses before a subscript keep tuple immutable, and bare frozendict is not a builtin; Ruff's tests only use frozen=True.
    "RUF009-frozen-truthiness-literals.py": "from dataclasses import dataclass\n\n\n@dataclass(frozen=-1)\n"
                                            "class Negative:\n    x: int = 1\n\n\n@dataclass(frozen=[1])\n"
                                            "class NonEmptyList:\n    x: int = 1\n\n\n@dataclass(frozen=list(\"a\"))\n"
                                            "class ListCall:\n    x: int = 1\n\n\n@dataclass(frozen=f\"{x}\")\n"
                                            "class OpaqueFString:\n    x: int = 1\n\n\n@dataclass\nclass Holder:\n"
                                            "    a: Negative = Negative()\n    b: NonEmptyList = NonEmptyList()\n"
                                            "    c: ListCall = ListCall()\n    d: OpaqueFString = OpaqueFString()\n"
                                            "    e: tuple = (tuple)[int]()\n    f: list = frozendict()\n",
    # RUF010: A module-level rebinding of str after a function still shadows the builtin inside that function body (visited after the module), while repr stays builtin; only repr(a) is reported.
    "RUF010-deferred-function-shadowing.py": "def f(a):\n    return f\"{str(a)} {repr(a)}\"\n\n\nstr = lambda x: x\n",
    # RUF012: Union[(int, str)] and Annotated[(int, 1)] spread their parenthesized tuple slice like Ruff (all elements for Union, the first for Annotated), so these are immutable while list members are reported.
    "RUF012-parenthesized-tuple-union-annotated.py": "from typing import Union, Annotated\n\n\nclass A:\n"
                                                     "    a: Union[((int, str))] = []\n"
                                                     "    b: Annotated[((int, 1))] = []\n"
                                                     "    c: Union[(int, list)] = []\n"
                                                     "    d: Annotated[(list, 1)] = []\n    e: Union[(int,)] = []\n"
                                                     "    f: Annotated[(int,)] = []\n",
    # RUF013: Annotated[(int, 1)] reports int, Literal[(None, 1)] and Union[(int, None)] allow None and Literal[(1, 2)] or Union[(int, str)] are reported whole, as Ruff spreads a parenthesized tuple slice.
    "RUF013-parenthesized-tuple-slices.py": "from typing import Annotated, Literal, Union\n\n\ndef f(\n"
                                            "    a: Annotated[(int, 1)] = None,\n    b: Literal[(None, 1)] = None,\n"
                                            "    c: Literal[(1, 2)] = None,\n    d: Union[(int, str)] = None,\n"
                                            "    e: Union[(int, None)] = None,\n    f: Literal[(None)] = None,\n"
                                            "    g: Annotated[(int)] = None,\n    h: Annotated[((int, 1))] = None,\n"
                                            "): ...\n",
    # RUF015: Keyword arguments and a starred argument in the inner list(...) call are ignored as in Ruff, and whitespace or a comment between call, subscript and pop does not stop the match.
    "RUF015-inner-call-keywords-and-spacing.py": "x = range(3)\na = list (x) [0]\nb = list(x) .pop (0)\n"
                                                 "c = list(x)[ 0 ]\nd = list(x)[(0  # zero\n)]\ne = list(*x, k=1)[0]\n"
                                                 "f = list(x, *())[0]\ng = [ * x ][0]\n"
                                                 "h = list(ß)[0] if (ß := x) else None\n",
    # RUF016: Pins the tuple-index span through strings holding brackets or `#`, leading and trailing comments and deep nesting, plus index and slice checks inside string annotations.
    "RUF016-tuple-span-strings-comments-and-annotations.py": "a = [1][\"]\", 1,]\nb = [1][\"#\", [2, [3, [4]]],]\n"
                                                             "c = [1][  # lead [\n    1, 2,  # ]\n]\n"
                                                             "d = [1][\"\"\" ] \"\"\", 1,]\n"
                                                             "x: \"[1]['a'] | [2][1.5:'b':None]\" = 1\n"
                                                             "y: list[\"(1,)[1.0]\"] = []\n",
    # RUF017: Pins calls found inside string annotations: several per string, nested in a subscript, inside cast(), and concatenated (whole-string range), but not in Literal[...] or in plain strings.
    "RUF017-string-annotation-calls.py": "from typing import Literal, cast\nx = [[1]]\n"
                                         "a: \"sum(x, []) | sum(x, list())\" = 1\nb: \"list[sum(x, [])]\" = 1\n"
                                         "c: Literal[\"sum(x, [])\"] = 1\nd = print(\"sum(x, [])\")\n"
                                         "e = cast(\"sum(x, [])\", x)\nf: \"sum(x, \" \"[])\" = 1\n"
                                         "g = sum(x, start_=[])\n",
    # RUF018: Pins Ruff's copied reads: broken by an unconditional del and an except alias, kept by a conditional del, not taken from an enclosing function, and uses through nonlocal and __all__.
    "RUF018-copied-reads-and-chain-breaks.py": "a = 0\nprint(a)\ndel a\nassert (a := 1)\nb = 0\nprint(b)\nif c:\n"
                                               "    del b\nassert (b := 1)\ntry:\n    pass\nexcept E as e:\n"
                                               "    print(e)\nassert (e := 1)\ndef outer():\n    x = 1\n"
                                               "    def inner():\n        print(x)\n        assert (x := 2)\n"
                                               "    assert (y := 1)\n    def g():\n        nonlocal y\n"
                                               "assert (z := 1)\n__all__ = [\"z\"]\n",
    # RUF019: Tuple keys inside extra parentheses, a starred one-element key against `d[*x]`, a parenthesized empty-initializer callee, comments in an empty argument list and before bool's first argument, none of which Ruff's own fixture covers.
    "RUF019-paren-tuple-keys-and-empty-initializers.py": "d = {}\nx = y = 1\nif ((x, y)) in d and d[x, y]:\n    pass\n"
                                                         "if (*x,) in d and d[*x]:\n    pass\n"
                                                         "if (list)() in d and d[(list)()]:\n    pass\n"
                                                         "if list(  # empty\n) in d and d[list()]:\n    pass\n"
                                                         "z = bool(  # first argument\n    x in d and d[x]\n)\n",
    # RUF020: A `|` union whose left side is a subscripted generic in an annotation (a different tree shape), and a parenthesized member tuple with and without a trailing comma, none of which Ruff's own fixture covers.
    "RUF020-union-type-and-tuple-trailing-comma.py": "from typing import Never, Union\nimport typing\n\n"
                                                     "a: list[int] | Never\nb: list[int] | (typing.NoReturn) | str\n"
                                                     "c: Union[(Never, int),]\nd = Union[(Never, int),]\n"
                                                     "e: Union[(Never, int)]\nf = Union[(Never, int)]\n",
    # RUF021: And-under-or inside string annotations and f-string fields, mixed with parenthesized operands and and-over-or that must stay silent, none of which Ruff's own fixture covers.
    "RUF021-string-annotation-and-nesting.py": "from typing import cast\n\na, b, c, d = 1, 0, 2, 3\n"
                                               "x: \"a or b and c\" = 1\ny = cast(\"a and b or c\", 1)\n"
                                               "z = a or (b and c) or d and a\nw = f\"{a or b and c}\"\n"
                                               "v = a and (b or c and a)\nu = (a or b) and c\nt = a or not b and c\n",
    # RUF022: Pins the module-scope test (annotations, type parameters and type aliases are not module scope; a comprehension's first iterable is) and natord's handling of zero-led digit runs, white space inside digit runs and the order of characters above U+FFFF against U+E000 to U+FFFF
    "RUF022-scope-and-zero-led-digit-runs.py": "def f(x: __all__.extend([\"b\", \"a\"])) -> __all__.extend([\"d\", \"c\"]): pass\n"
                                               "def g[T](x=__all__.extend([\"f\", \"e\"])): pass\n"
                                               "type X = __all__.extend([\"h\", \"g\"])\n"
                                               "a = [x for x in __all__.extend([\"j\", \"i\"])]\n"
                                               "b = [x for y in z for x in __all__.extend([\"l\", \"k\"])]\n"
                                               "c = [x for x in [y for y in __all__.extend([\"n\", \"m\"])]]\n"
                                               "d = [[x for x in __all__.extend([\"p\", \"o\"])] for y in q]\n"
                                               "__all__ = [\"x01\", \"x0y\"]\n__all__ = [\"x0y\", \"x01\"]\n"
                                               "__all__ = [\"x012\", \"x09\"]\n__all__ = [\"x09\", \"x012\"]\n"
                                               "__all__ = [\"0 1\", \"0a\"]\n__all__ = [\"10\", \"1 0\"]\n"
                                               "__all__ = [\"\\U0001F600\", \"￿\"]\n"
                                               "__all__ = [\"￿\", \"\\U0001F600\"]\n",
    # RUF023: Pins natord's digit-by-digit comparison of zero-led runs, white space inside digit runs and code point order above U+FFFF, for lists, tuples, sets and dict keys
    "RUF023-zero-led-digit-runs-and-astral-order.py": "class A:\n    __slots__ = [\"x01\", \"x0y\"]\nclass B:\n"
                                                      "    __slots__ = [\"x0y\", \"x01\"]\nclass C:\n"
                                                      "    __slots__ = [\"x012\", \"x09\"]\nclass D:\n"
                                                      "    __slots__ = {\"x09\": 1, \"x012\": 2}\nclass E:\n"
                                                      "    __slots__ = [\"a00000001\", \"a0000000a\"]\nclass F:\n"
                                                      "    __slots__ = [\"0 1\", \"0a\"]\nclass G:\n"
                                                      "    __slots__ = {\"10\", \"1 0\"}\nclass H:\n"
                                                      "    __slots__ = [\"\\U0001F600\", \"￿\"]\nclass I:\n"
                                                      "    __slots__ = (\"￿\", \"\\U0001F600\")\n",
    # RUF024: Pins that Ruff strips exactly one subscript from the value's callee: `list[int]()` and `(list)[int]()` are mutable, `list[int][str]()` is not
    "RUF024-subscripted-constructor-depth.py": "keys = \"ab\"\ndict.fromkeys(keys, list[int]())\n"
                                               "dict.fromkeys(keys, (list)[int]())\n"
                                               "dict.fromkeys(keys, (list[int])())\n"
                                               "dict.fromkeys(keys, list[int][str]())\n"
                                               "dict.fromkeys(keys, (list[int])[str]())\n"
                                               "dict.fromkeys(keys, dict[str, list][int]())\n",
    # RUF026: Pins calls inside simple string annotations (parameter, return, variable, nested quotes, triple quotes, cast and TypeVar bound) reported at their inner position, and complex strings skipped.
    "RUF026-call-in-string-annotation.py": "from typing import cast, List, TypeVar\n"
                                           "from collections import defaultdict\n\n\n"
                                           "def f(a: \"defaultdict(default_factory=int)\") -> \"defaultdict(default_factory=list)\":\n"
                                           "    b: \"List['defaultdict(default_factory=int)']\" = 1\n"
                                           "    c: \"defaultdict\" \"(default_factory=int)\" = 1\n"
                                           "    d: \"defaultdict(default_factory=\\x69nt)\" = 1\n"
                                           "    e: \"\"\"defaultdict(default_factory=int)\"\"\" = 1\n"
                                           "    g = cast(\"defaultdict(default_factory=int)\", a)\n"
                                           "    T = TypeVar(\"T\", bound=\"defaultdict(default_factory=int)\")\n"
                                           "    h: List[\"defaultdict(default_factory=int)\"] = 1\n"
                                           "    return defaultdict\n",
    # RUF028: Pins how a statement holding a suppression comment hands a later trailing or indented own-line comment to its last expression, except when that expression is parenthesised.
    "RUF028-comment-attached-after-statement.py": "def f():\n    x = [  # fmt: off\n        1\n    ]\n    # fmt: on\n"
                                                  "    y = 1\n\n\ndef g():\n    x = (  # fmt: off\n        1\n    )\n"
                                                  "    # fmt: on\n    y = 1\n\n\nx = (  # fmt: off\n    1\n"
                                                  ")  # fmt: skip\nx = a + (  # fmt: off\n    b\n)  # fmt: skip\n"
                                                  "x = [  # fmt: off\n    1\n]\n# fmt: on\n",
    # RUF030: Pins that an assert inside a function body is judged after the whole module is bound, so a later module-level `print = ...` suppresses it.
    "RUF030-function-sees-later-module-rebinding.py": "def f():\n    assert x, print(\"a\")\n\n\nprint = log\n",
    # RUF032: Decimal float calls inside string annotations, nested quoted annotations and cast() type strings are reported at the span inside the string, while Literal strings and ordinary strings are not read.
    "RUF032-string-annotation.py": "from decimal import Decimal\nimport typing\n\nx: \"Decimal((1.5), 2.0)\" = 1\n"
                                   "y: \"list['Decimal(-+2.5)']\" = []\nz: \"Decimal(value=1.5)\" = 1\n"
                                   "w = typing.cast(\"Decimal(3.5)\", 1)\nv: typing.Literal[\"Decimal(1.5)\"] = 1\n"
                                   "print(\"Decimal(1.5)\")\n",
    # RUF033: The dataclass decorator resolves where the class is defined: a module-level class before the import is silent, while a class inside a function body sees the later module import and is reported.
    "RUF033-import-after-use.py": "@dataclass\nclass A:\n    def __post_init__(self, x=1): ...\n\n\ndef f():\n"
                                  "    @dataclass\n    class B:\n        def __post_init__(self, y=2): ...\n\n"
                                  "    return B\n\n\nfrom dataclasses import dataclass\n",
    # RUF034: A conditional inside a quoted string nested in a string annotation is reported, Literal and Annotated metadata strings and plain strings are not read, and a parenthesized boolean chain differs from the flat one.
    "RUF034-nested-string-annotation.py": "from typing import Literal, Annotated\n\nx: \"list['a if b else a']\" = []\n"
                                          "y: Literal[\"a if b else a\"] = 1\n"
                                          "z: Annotated[int, \"a if b else a\"] = 1\nprint(\"a if b else a\")\n"
                                          "w = (a and b and c) if t else ((a and b) and c)\n",
    # RUF036: Pins TypeVar's non-type keywords, TypedDict legacy keyword fields and total, starred TypeForm arguments, TypedDict[{...}] values, and generic slots inside store/del subscript targets.
    "RUF036-call-keywords-and-store-subscripts.py": "from typing import List, Optional, TypeVar, TypedDict\n"
                                                    "from typing_extensions import TypeForm\n\n"
                                                    "A = TypeVar(\"A\", covariant=None | int)\n"
                                                    "B = TypeVar(\"B\", default=None | int)\n"
                                                    "C = TypedDict(\"C\", x=None | int, total=None | int)\n"
                                                    "D = TypedDict(\"D\", {\"y\": int}, x=None | int)\n"
                                                    "E = TypeForm(*[None | int])\n"
                                                    "F = TypedDict[{\"a\": None | int, \"b\": f(None | int)}]\nd = {}\n"
                                                    "d[List[None | int]] = 1\ndel d[Optional[None | int]]\n"
                                                    "x = d[List[None | int]]\n",
    # RUF037: Pins calls inside string annotations, builtins.list() and a shadowed list, parenthesized callee and iterable= keyword.
    "RUF037-string-annotations-and-builtin-aliases.py": "import builtins\nimport collections as c\n"
                                                        "from collections import deque\n\na: \"deque([], 3)\" = None\n"
                                                        "b = c.deque(builtins.list())\nc2 = c.deque(iterable=())\n"
                                                        "d = deque(f\"{''}\" \"\")\ne = (deque)(set(), maxlen=2)\n\n\n"
                                                        "def shadow(list):\n    return deque(list())\n\n\n"
                                                        "def stays_silent():\n"
                                                        "    return deque(\"a\"), deque([1]), deque(maxlen=3), deque([], 1, 2), deque(*[[]])\n",
    # RUF040: Pins parenthesized and bytes literal messages and look-alike non-literals.
    "RUF040-parenthesized-and-bytes-messages.py": "x = 1\nassert x, (1)\nassert x, (\n    None  # why\n)\n"
                                                  "assert x, b\"a\" b\"b\"\nassert x, ((...))\nassert x, -1\n"
                                                  "assert x, 1 + 1\nassert x, (\"text\")\nassert (x, 1), 2\n"
                                                  "assert x, \"ok\"  # noqa: RUF040\nassert x, 3  # noqa: RUF040\n",
    # RUF041: Pins that only a lone tuple slice is unpacked: a tuple beside other slice elements or a trailing comma is a value, so a Literal nest inside it is reported on its own; Ruff's fixtures have no such case.
    "RUF041-tuple-element-not-unpacked.py": "from typing import Literal\n\n"
                                            "a = Literal[(1, Literal[2, Literal[3]]), 4]\n"
                                            "b = Literal[(Literal[Literal[1]],), 2]\nc = Literal[(Literal[1], 2), 3]\n"
                                            "d = Literal[(1, (Literal[2, Literal[3]])), 4]\n"
                                            "e: Literal[(1, Literal[2, Literal[3]]), 4] = 1\n"
                                            "f: Literal[(Literal[Literal[1]],), 2] = 1\n"
                                            "g = Literal[(1, Literal[2, Literal[3]]),]\nh = Literal[Literal[1],]\n"
                                            "i: Literal[(1, Literal[2, Literal[3]]),] = 1\n"
                                            "j: \"Literal[(1, Literal[2, Literal[3]]), 4]\" = 1\n",
    # RUF043: Pins that the callee is found through every import form and alias, that pytest.warns, pytest.deprecated_call and a positional pattern are not reported, and that a func argument does not stop the match= check.
    "RUF043-import-forms-and-other-calls.py": "import pytest\nimport pytest as pt\nfrom pytest import raises\n"
                                              "from pytest import raises as r\n\n\ndef test():\n"
                                              "    with pytest.raises(E, match=\"a.\"): ...\n"
                                              "    with pt.raises(E, match=\"a.\"): ...\n"
                                              "    with raises(E, match=\"a.\"): ...\n"
                                              "    with r(E, match=\"a.\"): ...\n"
                                              "    with pytest.warns(W, match=\"a.\"): ...\n"
                                              "    with pytest.deprecated_call(match=\"a.\"): ...\n"
                                              "    with pytest.raises(E, \"a.\"): ...\n"
                                              "    pytest.raises(E, lambda: 1, match=\"a.\")\n",
    # RUF046: Pins Ruff's binding-type reading for round(name, ndigits): only-binding int names via assignment, tuple and chained unpacking, walrus, annotation and parameter annotation count, while rebound, float, bool and string-annotated names and an int-typed ndigits name do not.
    "RUF046-round-name-bindings.py": "inferred = 1\nf_ = 1.0\na = b = 1\nc, d = 1, 2\ne = 1\ne = 2\ng: int\n"
                                     "h: int = 1\ni: float = 1\nj = True\nk = int(\"3\")\nl = int\n(w := 1)\n\n\n"
                                     "def fn(p: int, q: float, r, s: \"int\" = 1):\n    int(round(inferred, 2))\n"
                                     "    int(round(f_, 2))\n    int(round(a, 2))\n    int(round(b, 2))\n"
                                     "    int(round(c, 2))\n    int(round(d, 2))\n    int(round(e, 2))\n"
                                     "    int(round(g, 2))\n    int(round(h, 2))\n    int(round(i, 2))\n"
                                     "    int(round(j, 2))\n    int(round(k, 2))\n    int(round(l, 2))\n"
                                     "    int(round(w, 2))\n    int(round(p, 2))\n    int(round(q, 2))\n"
                                     "    int(round(r, 2))\n    int(round(s, 2))\n    int(round((inferred), 2))\n"
                                     "    int(round(inferred, -1))\n    int(round(inferred, 1 + 1))\n"
                                     "    int(round(inferred, h))\n    int(round(1, h))\n    int(round(1, 1 + 1))\n"
                                     "    int(round(1, True))\n    int(round(1, inferred))\n    int(round(inferred))\n"
                                     "    int(round(inferred, None))\n    int(round(q))\n    int(round(r))\n"
                                     "    int(round(r, None))\n    int(round(r, 2))\n",
    # RUF048: a parenthesized split callee, a starred map followed by a comment, and line continuations before the argument lists are reported exactly as Ruff does
    "RUF048-parenthesized-split-and-starred-comment.py": "import foo\n__version__ = \"1.2.3\"\n"
                                                         "a = tuple(map(int, (__version__.split)(\".\")))\n"
                                                         "b = tuple(map(int, ((foo.__version__).split)(sep=\".\")))\n"
                                                         "c = [*  # star then comment\n"
                                                         "    map(int, __version__.split(\".\"))]\nd = map \\\n"
                                                         "    (int, foo.__version__.split \\\n    (\".\"))\n",
    # RUF049: self-named enum base, a global-declared base, a nested enum class through Outer.Inner and a rebound one, a '#' inside a decorator string, and line continuations
    "RUF049-self-named-global-and-nested-bases.py": "from dataclasses import dataclass\nfrom enum import Enum\n\n\n"
                                                    "class E(Enum): ...\n\n\n@dataclass\nclass E(E): ...\n\n\n"
                                                    "@dataclass\nclass Enum(Enum): ...\n\n\ndef g():\n    global E\n\n"
                                                    "    @dataclass\n    class Y(E): ...\n\n\nclass Outer:\n"
                                                    "    class Inner(Enum): ...\n    class Gone(Enum): ...\n"
                                                    "    Gone = 1\n\n\n@dataclass(repr=\"#\")  # it's)\n"
                                                    "class V(Outer.Inner): ...\n\n\n@dataclass\n"
                                                    "class W(Outer.Gone): ...\n\n\n@dataclass \\\n    (frozen=True)\n"
                                                    "class X(Enum \\\n        [int]): ...\n",
    # RUF051: a parenthesized with item, a tuple with target, and a dict() call and conditional split by line continuations are known dicts
    "RUF051-parenthesized-with-and-continued-dict-call.py": "with (dict() as a):\n    pass\nif 1 in a:\n    del a[1]\n"
                                                            "b = dict \\\n    ()\nif 1 in b:\n    del b[1]\nc = {} \\\n"
                                                            "    if b else {}\nif 1 in c:\n    del c[1]\n"
                                                            "with ({}, 1) as (d, e):\n    pass\nif 1 in d:\n"
                                                            "    del d[1]\nif 1 in e:\n    del e[1]\n",
    # RUF057: The `number=` keyword wins over the first positional and unknown keywords do not block the report, and literal ndigits beyond i64 (decimal, hex, octal, binary, underscores) is not reported while i64 max is.
    "RUF057-number-keyword-and-i64-ndigits.py": "round(2, number=1.5)\nround(1.5, number=2)\nround(1, number=2)\n"
                                                "round(1, foo=2)\nround(1.5, foo=2)\nround(x=1, y=2)\n"
                                                "round(ndigits=2, number=1)\nround(1, ndigits=2.5)\n"
                                                "round(1, 2, ndigits=None)\nround(number=1, ndigits=-1)\n"
                                                "round(number=1, ndigits=(3))\nround(1, (-0))\nround(1, -0)\n"
                                                "round(1, +2)\nround()\nround(ndigits=1)\n"
                                                "round(1, 9223372036854775806)\nround(1, 9223372036854775807)\n"
                                                "round(1, 9223372036854775808)\nround(1, 9_223_372_036_854_775_808)\n"
                                                "round(1, 9_223_372_036_854_775_807)\nround(1, 9223372036854775810)\n"
                                                "round(1, 9299999999999999999)\nround(1, 8999999999999999999)\n"
                                                "round(1, 10000000000000000000)\nround(1, 0x7FFFFFFFFFFFFFFF)\n"
                                                "round(1, 0x8000000000000000)\nround(1, 0x0000_7FFF_FFFF_FFFF_FFFF)\n"
                                                "round(1, 0x1_0000_0000_0000_0000)\nround(1, 0o777777777777777777777)\n"
                                                "round(1, 0o1000000000000000000000)\n"
                                                "round(1, 0b111111111111111111111111111111111111111111111111111111111111111)\n"
                                                "round(1, 0b1000000000000000000000000000000000000000000000000000000000000000)\n"
                                                "round(1, (9223372036854775808))\n"
                                                "round(1, ndigits=9223372036854775808)\n"
                                                "round(1, ndigits=(0x8000000000000000))\nround(1, 00)\nround(1, 0)\n"
                                                "round(1, 0xFFFFFFFFFFFFFFFFFFFF)\nround(1, 18446744073709551616)\n"
                                                "round(1, 9223372036854775807 + 1)\nround(1, 1e3)\nround(1, 2j)\n",
    # RUF058: Starred first argument, mixed and all-starred zip positionals, empty zip, parenthesized callees and zip, generator argument, and comments inside the call.
    "RUF058-starred-and-parenthesized-forms.py": "import itertools\nfrom itertools import starmap as sm\n"
                                                 "import itertools as it\nitertools.starmap(*fs, zip(a))\n"
                                                 "sm(f, zip(*a, b))\nsm(f, zip(*a, *b))\nsm(f, zip())\n"
                                                 "it.starmap(f, (zip(a)))\n(it.starmap)(f, zip(a))\nsm(f, *zip(a))\n"
                                                 "sm(f, zip(a), )\nsm(f, zip(a), **kw)\nsm(*fs)\n"
                                                 "sm(f, zip(x for x in y))\nsm(f, zip(a)(b))\n"
                                                 "sm(f, zip(a, strict=True))\nsm(f, zip(a, **kw))\nsm(f, (zip)(a))\n"
                                                 "sm(f, zip(a) if c else zip(b))\nsm(f,\n   # comment\n"
                                                 "   zip(a, b)  # trailing\n)\n",
    # RUF059: Pins that an empty `()`/`[]` target still counts as a tuple target of the chain and that a walrus inside a tuple `with` target is reported (3 reports).
    "RUF059-empty-tuple-target-and-with-walrus.py": "def f(t, g, d):\n    a, b = () = t\n    c, e = () = 1, 2\n"
                                                    "    [] = h, i = 1, 2\n    with g() as (j, d[(y := 1)]):\n"
                                                    "        pass\n    with g(z := 1) as [k, m]:\n        pass\n"
                                                    "    return j\n",
    # RUF060: Pins that plain-string parts of a concatenation inside an interpolation are ignored, backslash-newline f-strings (raw excepted), and comparisons in string annotations and cast strings (6 reports).
    "RUF060-fstring-literal-parts-and-annotations.py": "from typing import cast\na = \"a\"\nr1 = a in f\"{'x' f''}\"\n"
                                                       "r2 = a in f\"{f'' 'x'}\"\nr3 = a in f\"{'x' 'y'}\"\n"
                                                       "r4 = a in f\"\\\n{''}\"\nr5 = a in rf\"\\\n\"\n"
                                                       "x: \"1 in []\" = 1\ny = cast(\"1 not in list()\", 1)\n"
                                                       "z: \"list['1 in ()']\" = []\nw = \"1 in []\"\n",
    # RUF061: Pins that legacy-form calls inside string annotations, nested annotation strings and cast strings are reported at the call inside the string (5 reports).
    "RUF061-string-annotations.py": "import pytest\nfrom typing import cast\n\n\ndef func():\n    pass\n\n\n"
                                    "x: \"pytest.warns(UserWarning, func)\" = 1\n"
                                    "y = cast(\"pytest.deprecated_call(func)\", 1)\n"
                                    "z: \"list['pytest.raises(ValueError, func=func)']\" = []\n"
                                    "w: \"pytest.raises(ValueError)\" = 1\nv = \"pytest.raises(ValueError, func)\"\n\n"
                                    "\n"
                                    "def test_a(a: \"pytest.raises(ValueError, func)\" = None) -> \"pytest.raises(func, ValueError, func)\":\n"
                                    "    pass\n",
    # RUF063: Pins reports inside f-string fields and string annotations (including cast, TypeVar bound, Optional, Annotated) and silence for Literal[...], plain strings and comments inside attribute chains, none of which Ruff's own fixture exercises.
    "RUF063-string-annotations-and-fstrings.py": "from typing import Optional, cast, TypeVar, Annotated, Literal\n"
                                                 "(foo.__dict__  # c\n .get(\"__annotations__\"))\n(foo\n # c\n"
                                                 " .__dict__\n # d\n [\"__annotations__\"])\n"
                                                 "f\"{foo.__dict__['__annotations__']}\"\n"
                                                 "f\"{foo.__dict__.get('__annotations__')!r:>{10}}\"\n"
                                                 "f\"{foo.__dict__['__annotations__']}\" f\"{foo.__dict__.get('__annotations__', {})}\"\n"
                                                 "x: Optional[\"foo.__dict__['__annotations__']\"] = None\n"
                                                 "y = cast(\"foo.__dict__.get('__annotations__')\", z)\n"
                                                 "T = TypeVar(\"T\", bound=\"foo.__dict__['__annotations__']\")\n"
                                                 "w: Annotated[int, \"foo.__dict__['__annotations__']\"]\n"
                                                 "v: Literal[\"foo.__dict__['__annotations__']\"]\n"
                                                 "u: \"list[foo.__dict__['__annotations__']]\"\n"
                                                 "def fn(a: \"foo.__dict__['__annotations__']\", b: 'foo.__dict__.get(\"__annotations__\")') -> \"foo.__dict__['__annotations__']\": ...\n"
                                                 "q = \"foo.__dict__['__annotations__']\"\n"
                                                 "print(\"foo.__dict__.get('__annotations__')\")\n"
                                                 "t: \"\"\"foo.__dict__['__annotations__']\"\"\"\n"
                                                 "s: (\"foo.__dict__\"\n    \"['__annotations__']\")\n"
                                                 "r: \"foo.__dict__['__annotations__']\" \"\" = 1\nclass A:\n"
                                                 "    z: \"foo.__dict__['__annotations__']\"\n"
                                                 "    def m(self) -> \"foo.__dict__.get('__annotations__')\": ...\n",
    # RUF064: Pins mode-slot counting with backslash continuations and comments in the argument list, keyword-over-positional and starred cut-offs, and parenthesised callees and Path receivers at any depth, none of which Ruff's fixture exercises.
    "RUF064-continuation-and-parenthesized-callees.py": "import pathlib\nimport os\nfrom pathlib import Path\n"
                                                        "os.chmod(\"f\", \\\n         755)\nos.chmod(\\\n"
                                                        "    \"f\", 755)\nos.umask(\\\n    22)\nos.open(\"f\", \\\n"
                                                        "    0, \\\n    777)\nos.chmod(\"f\", \\\n    mode=755)\n"
                                                        "os.makedirs(\"d\", \\\n    parent_mode=511)\n"
                                                        "Path(\"x\").chmod(\\\n    777)\nos.chmod(\"f\",  # c\n"
                                                        "    755)\nos.chmod(  # c\n    \"f\", 755)\n"
                                                        "(os.chmod)(\"f\", 755)\n((os.umask))(22)\n(  # c\n"
                                                        "    os.open)(\"f\", 0, 777)\n((Path))(\"x\").chmod(755)\n"
                                                        "(Path(\"x\")).chmod(755)\n((Path(\"x\")).touch)(755)\n"
                                                        "(pathlib.Path)(\"x\").mkdir(parent_mode=511)\n"
                                                        "p = ((Path(\"x\")))\n((p)).chmod(755)\n"
                                                        "(p.mkdir)(755, parent_mode=(511))\n"
                                                        "with ((Path(\"x\"))) as q:\n    q.chmod(755)\n"
                                                        "r = (Path)(\"x\")\nr.lchmod(755)\ns = Path(\"x\"),\n"
                                                        "s.chmod(755)\nt = [Path(\"x\")][0]\nt.chmod(755)\n"
                                                        "(os.makedirs)(\"d\", parent_mode=0o7, mode=7)\n"
                                                        "os.chmod(\"f\", mode=0o7, *a)\nos.chmod(*a, \"f\", 7)\n"
                                                        "os.chmod(\"f\", 7, *a)\n",
    # RUF068: Pins extend calls inside string annotations (forward references, cast, TypeAlias, nested quoting; not Literal, class or return annotations), backslash continuations in the extend call and list, and the comprehension target being outside module scope, none of which Ruff's fixture exercises.
    "RUF068-string-annotations-continuation-comprehension-target.py": "from typing import cast, TypeAlias, Literal\n"
                                                                      "a: \"list[__all__.extend(['a', 'a'])]\" = 1\n"
                                                                      "b = cast(\"__all__.extend(['b', \\\"b\\\", 'c'])\", x)\n"
                                                                      "c: \"'__all__.extend([\\\"c\\\", \\\"c\\\"])'\"\n"
                                                                      "d: TypeAlias = \"__all__.extend(('d', 'd'))\"\n"
                                                                      "type E = \"__all__.extend(['e', 'e'])\"\n"
                                                                      "f: Literal[\"__all__.extend(['f', 'f'])\"]\n"
                                                                      "g: \"(lambda: __all__.extend(['g', 'g']))\"\n"
                                                                      "h: \"x.__all__.extend(['h', 'h'])\"\n"
                                                                      "i: \"__all__.extend(['i', 'i'], k)\"\n"
                                                                      "j: \"__all__.extend(['j', 'j' 'j', 'jj'])\"\n"
                                                                      "class K:\n"
                                                                      "    k: \"__all__.extend(['k', 'k'])\"\n"
                                                                      "def m() -> \"__all__.extend(['m', 'm'])\": ...\n"
                                                                      "n: \"(__all__).extend((['n', ('n')]))\"\n"
                                                                      "__all__.extend(\\\n    [\"a\", \"a\"])\n"
                                                                      "__all__.extend([\"b\", \\\n    \"b\"])\n"
                                                                      "__all__ = [\"c\", \\\n    \"c\"]\n"
                                                                      "__all__ += \\\n    [\"d\", \"d\"]\n"
                                                                      "[0 for a[__all__.extend([\"Z\", \"Z\"])] in y]\n"
                                                                      "[0 for _ in __all__.extend([\"Y\", \"Y\"]) if 1]\n"
                                                                      "{0: 1 for _ in __all__.extend([\"X\", \"X\"])}\n",
    # SLF001: A module-level instance and a function returning the class are exempt only inside that class's body. In a dunder operator, lambda defaults are exempt but lambda bodies and class bases are reported.
    "SLF001-current-class-and-dunder-scopes.py": "handle = Outer()\n\n\ndef make() -> Outer: ...\n\n\nclass Outer:\n"
                                                 "    def m(self):\n        return handle._a, make._b\n\n"
                                                 "    def __eq__(self, other):\n        f = lambda a=other._c: a._d\n"
                                                 "        class In(other._e):\n            pass\n        return f, In\n"
                                                 "\n\nhandle._f\n",
    # SLOT000: Nested-class enum members under if, rebound later, two levels deep, and method-local classes, which Ruff's class-scope lookup treats differently.
    "SLOT000-nested-member-enums-rebinding-and-depth.py": "import enum\nimport sys\n\n\nclass Outer:\n"
                                                          "    if sys.version_info >= (3, 8):\n"
                                                          "        class Inner(enum.Enum):\n            pass\n\n"
                                                          "    class Gone(enum.Enum):\n        pass\n\n    Gone = 1\n\n"
                                                          "    class Deep:\n        class Deeper(enum.Flag):\n"
                                                          "            pass\n\n    def method(self):\n"
                                                          "        class Hidden(enum.Enum):\n            pass\n\n\n"
                                                          "class InsideIf(str, Outer.Inner):  # a member bound under `if` still counts\n"
                                                          "    pass\n\n\n"
                                                          "class Rebound(str, Outer.Gone):  # the live binding is no longer the enum\n"
                                                          "    pass\n\n\nclass TwoLevels(str, Outer.Deep.Deeper):\n"
                                                          "    pass\n\n\n"
                                                          "class FunctionLocal(str, Outer.Hidden):  # method locals are not class members\n"
                                                          "    pass\n",
    # SLOT001: A type parameter named tuple does not shadow the builtin base, while an imported name is resolved where it is read; one subscript level is unwrapped.
    "SLOT001-type-parameter-does-not-shadow-base.py": "from typing import Tuple\n\n\n"
                                                      "class A[tuple](tuple):  # bases resolve outside the type-parameter scope\n"
                                                      "    pass\n\n\nclass B[T](tuple[T, ...]):\n    pass\n\n\n"
                                                      "class C[Tuple](Tuple):  # an imported name is resolved where it is read\n"
                                                      "    pass\n\n\n"
                                                      "class D(Tuple[int][0]):  # only one subscript level is unwrapped\n"
                                                      "    pass\n",
    # SLOT002: A backslash line continuation inside the base list is not an extra base.
    "SLOT002-line-continuation-in-bases.py": "from collections import namedtuple\n\n\n"
                                             "class A(namedtuple(\"A\", \"x\"), \\\n        object):\n    pass\n\n\n"
                                             "class B(namedtuple(\"B\", \"x\"), \\\n        make()):\n    pass\n",
    # T201: Ruff reports print calls inside string type definitions (annotations, nested List["..."], cast, TypeVar bound) at their position in the string, honours file= there and skips Literal and plain strings; Ruff's fixture has none.
    "T201-string-annotations.py": "from typing import List, Tuple, TypeVar, cast\nimport sys\n\nx: \"print(1)\" = 1\n"
                                  "y: \"Tuple[print(2), print(3)]\" = 1\nz: List[\"print(4)\"] = []\n\n\n"
                                  "def f(a: \"print(5, file=sys.stderr)\", b: \"print(6, file=open('x'))\") -> \"print(7)\":\n"
                                  "    pass\n\n\nT = TypeVar(\"T\", bound=\"print(8)\")\nw = cast(\"print(9)\", 1)\n"
                                  "v: \"List[Literal['print(10)']]\" = []\nu = \"print(11)\"\ns: \"print\" = 1\n",
    # T203: Ruff reports pprint calls inside string type definitions at their position in the string, resolving the name at module scope even where a parameter shadows it, and honours stream= there; Ruff's fixture has none.
    "T203-string-annotations.py": "from typing import List, Tuple, TypeVar, cast\nfrom pprint import pprint\n"
                                  "import sys\n\nx: \"pprint(1)\" = 1\ny: \"Tuple[pprint(2), pprint(3)]\" = 1\n"
                                  "z: List[\"pprint(4)\"] = []\n\n\n"
                                  "def f(a: \"pprint(5, stream=sys.stderr)\", b: \"pprint(6, stream=open('x'))\") -> \"pprint(7)\":\n"
                                  "    pass\n\n\ndef g(pprint):\n    q: \"pprint(8)\" = 1\n\n\n"
                                  "T = TypeVar(\"T\", bound=\"pprint(9)\")\nw = cast(\"pprint(10)\", 1)\n"
                                  "u = \"pprint(11)\"\n",
    # TC001: an import used at runtime in an enclosing class body exempts same-module typing-only imports in its methods
    "TC001-class-body-runtime-import-ancestor.py": "class A:\n    import pkga\n    import os\n    from . import rel\n"
                                                   "    v = pkga.thing\n    w = os.sep\n    u = rel.x\n\n"
                                                   "    def m(self):\n        from pkga import B\n"
                                                   "        from os import path\n        from . import other\n"
                                                   "        y: B\n        z: path\n        q: other\n\n\nclass C:\n"
                                                   "    import pkgc\n    c = pkgc.x\n\n\ndef g():\n"
                                                   "    from pkgc import D\n    d: D\n",
    # TC002: a `__future__` import of a feature other than annotations leaves parameter annotations runtime-evaluated
    "TC002-future-import-other-feature.py": "from __future__ import division\n\nimport numpy as np\nimport os\n"
                                            "from . import rel\n\n\n"
                                            "def f(a: np.ndarray, b: os.PathLike) -> rel.Thing:\n    pass\n",
    # TC003: under future annotations, `InitVar`/`KW_ONLY` heads of a `@dataclass` field annotation are runtime uses
    "TC003-dataclass-initvar-kw-only-runtime.py": "from __future__ import annotations\n\n"
                                                  "from dataclasses import InitVar\nfrom dataclasses import KW_ONLY\n"
                                                  "import pkgp\n\n\ndef make():\n"
                                                  "    from dataclasses import dataclass\n\n    @dataclass\n"
                                                  "    class A:\n        a: InitVar[pkgp.X]\n        _: KW_ONLY\n"
                                                  "        b: int\n\n    return A\n",
    # TC004: A TYPE_CHECKING re-import inherits the runtime reads of an earlier import of the same name in the same scope, as Ruff copies shadowed references.
    "TC004-inherited-runtime-reference.py": "from typing import TYPE_CHECKING\nimport foo\nfoo.x()\nif TYPE_CHECKING:\n"
                                            "    import foo\n",
    # TC005: Empty TYPE_CHECKING blocks nested in try/with/for/while/match/decorated functions, with elif/else variants staying silent.
    "TC005-nested-blocks.py": "from typing import TYPE_CHECKING\n\n\ndef f():\n    if TYPE_CHECKING:\n        pass\n\n"
                              "\nclass C:\n    if TYPE_CHECKING:\n        pass\n\n    def m(self):\n        try:\n"
                              "            if TYPE_CHECKING:\n                pass\n        finally:\n"
                              "            pass\n        with open(\"x\") as fh:\n            if TYPE_CHECKING:\n"
                              "                pass\n        for i in range(3):\n            if TYPE_CHECKING:\n"
                              "                pass\n        while True:\n            if TYPE_CHECKING:\n"
                              "                pass\n            break\n\n\nasync def g():\n    if TYPE_CHECKING:\n"
                              "        pass\n\n\n@decorator\ndef h():\n    if TYPE_CHECKING:\n        pass\n"
                              "    elif x:\n        pass\n    else:\n        pass\n\n\nmatch x:\n    case 1:\n"
                              "        if TYPE_CHECKING:\n            pass\nlambda: (yield)\n",
    # TC006: A cast call inside a quoted annotation or quoted cast type is reported at its type argument, as Ruff parses string type definitions.
    "TC006-cast-in-string-annotation.py": "import typing\nx: \"typing.cast(int, 1)\"\n"
                                          "y = typing.cast(\"typing.cast(int, 1)\", 2)\n",
    # TC007: Pins that Ruff looks the name up in the module scope in source order: local classes, outer-function locals and class attributes are reported, an earlier module binding is not, a later one is.
    "TC007-module-scope-runtime-lookup.py": "from typing import TypeAlias, TYPE_CHECKING\n\nif TYPE_CHECKING:\n"
                                            "    from foo import Foo, Early, Late\n\nEarly = 1\n\n\ndef f():\n"
                                            "    class Local: ...\n    a: TypeAlias = Local | Early | Late | list\n\n\n"
                                            "def outer():\n    Foo = 3\n    def inner():\n        b: TypeAlias = Foo\n"
                                            "    return inner\n\n\nclass K:\n    Bar = int\n    c: TypeAlias = Bar\n\n"
                                            "\nLate = 2\n",
    # TC010: Pins that a non-annotations __future__ import does not defer annotations, that Annotated metadata (Literal, bytes) is not a type definition, and the legacy TypedDict keyword-field rules (total excluded, mapping or no positional name disables them).
    "TC010-legacy-typeddict-annotated-metadata.py": "from __future__ import division\n"
                                                    "from typing import Annotated, Literal, TypedDict\n\n"
                                                    "x1: Annotated[int, Literal[\"a\" | b], b\"x\" | c]\n"
                                                    "x2: Annotated[Literal[\"a\" | b], 1] | \"c\"\n\n"
                                                    "TD1 = TypedDict(\"TD1\", a=\"A\" | B)\n"
                                                    "TD2 = TypedDict(\"TD2\", None, total=\"A\" | B, b=Literal[\"A\" | B])\n"
                                                    "TD3 = TypedDict(\"TD3\", {\"x\": int}, a=\"A\" | B)\n"
                                                    "TD4 = TypedDict(name=\"TD4\", a=\"A\" | B)\n",
    # TD001: The first tag across `#` segments decides: `# TODO # FIXME` is silent, tags after non-tag segments report.
    "TD001-first-tag-across-hash-segments.py": "# x # fixme\n# TODO # FIXME\n# note # XXX: later\n#: FIXME\n# FIXMEs\n",
    # TD002: An `@name` or bare `@` ending the comment reports while `@:` and `@name text` count as authors; ` : x` reports.
    "TD002-trailing-at-author-and-spaced-colon.py": "# TODO @bob\n# TODO @\n# TODO @: c\n# TODO @bob text\n# TODO : x\n"
                                                    "# note # TODO(bob): y\n# TODO\t@a\tz\n",
    # TD003: Link search follows blank lines and stops at the next directive or code; same-line `#<digit>` and URL count.
    "TD003-link-chain-and-blank-lines.py": "# TODO: a\n# some text\n\n# ABC-12\nx = 1\n\n# TODO: b\nx = 2\n# 1\n"
                                           "# TODO: c\n# ABC-1\n# TODO: d  http://x\n# TODO: e\n# line\n# TODO: f\n"
                                           "# 99\n# TODO: g #12\n# TODO: h\n# abc-1\n# Ab-1\n",
    # TD004: Colon check after parenthesised, unclosed and `@` author forms.
    "TD004-colon-after-author-forms.py": "# TODO(bob) text\n# TODO(bob\n# TODO @bob text\n# TODO @bob: text\n"
                                         "# TODO(a)(b): x\n# TODO-x\n# TODO\n",
    # TD005: pins @author, unclosed paren and double-paren author handling beyond Ruff's fixtures
    "TD005-at-author-and-unclosed-paren.py": "# TODO @name:\n# TODO @name: described\n# TODO(a)(b):\n# TODO(a\n"
                                             "# XXX @name\n# TODO @a b:\nx = 1  # FIXME(a)\n",
    # TD006: pins that only the first directive in a comment counts and HACK is skipped
    "TD006-first-directive-wins.py": "# hack # todo\n# todo # TODO\n# TODO # todo\n# fixme # Todo\n"
                                     "x = 1  # tOdo: later\n# todo_later\n",
    # TD007: pins @author and double-paren author followed by a colon with no space
    "TD007-at-author-colon-no-space.py": "# TODO @name:text\n# TODO@name:text\n# TODO(a)(b):text\n# TODO @a b:text\n"
                                         "# xxx(a):text\n# TODO @name: text\n",
    # TID252: Pins that spaces or a backslash continuation between the leading dots still make the import level 2 or more, that `from ..import x` with no space is caught, and that each imported name is its own finding.
    "TID252-dots-with-spaces.py": "from . import sibling\nfrom . . import parent\n"
                                  "from . .pkg import mod as alias, other\nfrom .\\\n .backslash import thing\n"
                                  "from ... .deep import *\nfrom ..import nospace\nfrom ..import*\nfrom .. import (\n"
                                  "    a,\n    b as c,\n)  # noqa: TID252\n",
    # TRY002: a local rebinding after the raise does not shadow it, but an enclosing function's binding does shadow a nested function's raise
    "TRY002-later-local-binding.py": "def f():\n    raise Exception(\"a\")\n    Exception = ValueError\n\n\ndef g():\n"
                                     "    def inner():\n        raise Exception(\"b\")\n\n    Exception = ValueError\n"
                                     "    return inner\n\n\ndef h():\n    raise BaseException(\"c\")\n"
                                     "    import BaseException\n",
    # TRY003: named escapes decode to whitespace (including lower-case names and aliases) or not, in plain, u-, concatenated and field-less f-strings
    "TRY003-named-whitespace-escapes.py": "def f(x):\n    raise ValueError(\"\\N{SPACE}\")\n"
                                          "    raise ValueError(\"a\\N{no-break space}b\")\n"
                                          "    raise ValueError(\"\\N{NBSP}\")\n"
                                          "    raise ValueError(f\"\\N{EM SPACE}\")\n"
                                          "    raise ValueError(\"ab\" \"\\N{LINE FEED}\")\n"
                                          "    raise ValueError(u\"\\N{IDEOGRAPHIC SPACE}\")\n"
                                          "    raise ValueError(\"\\N{ZERO WIDTH SPACE}\")\n"
                                          "    raise ValueError(\"\\N{TAG SPACE}\")\n"
                                          "    raise ValueError(\"\\N{SYMBOL FOR SPACE}\")\n"
                                          "    raise ValueError(\"\\N{MONGOLIAN VOWEL SEPARATOR}\")\n"
                                          "    raise ValueError(\"\\N{ZERO WIDTH NO-BREAK SPACE}\")\n",
    # TRY004: without a builtins import, a check function rebound later in the same if (same scope only) turns the report off; nested scopes, params and comprehension variables do not
    "TRY004-rebound-in-own-if.py": "def f(x):\n    if isinstance(x, int):\n        raise ValueError(\"a\")\n"
                                   "        isinstance = 1\n\n\ndef g(x):\n    if callable(x):\n"
                                   "        raise ValueError(\"b\")\n    else:\n        callable = 2\n\n\ndef f1(x):\n"
                                   "    if isinstance(x, int):\n        raise ValueError(\"nested def name\")\n"
                                   "        def isinstance(): ...\n\n\ndef f2(x):\n    if callable(x):\n"
                                   "        raise ValueError(\"nested def param\")\n        def g(callable): ...\n\n\n"
                                   "def f3(x):\n    if issubclass(x, int):\n"
                                   "        raise ValueError(\"lambda param\")\n        h = lambda issubclass: 1\n\n\n"
                                   "def f4(x, y):\n    if isinstance(x, int):\n"
                                   "        raise ValueError(\"comprehension var\")\n"
                                   "        z = [isinstance for isinstance in y]\n\n\ndef f5(x):\n    if callable(x):\n"
                                   "        raise ValueError(\"for target\")\n    else:\n        for callable in x:\n"
                                   "            pass\n\n\ndef f6(x):\n    if issubclass(x, int):\n"
                                   "        raise ValueError(\"import\")\n    elif isinstance(x, int):\n"
                                   "        from m import issubclass\n\n\ndef f8(x):\n    if isinstance(x, int):\n"
                                   "        raise ValueError(\"nested class\")\n        class isinstance: ...\n\n\n"
                                   "def f9(x):\n    if callable(x):\n        raise ValueError(\"other name\")\n"
                                   "        isinstance = 1\n",
    # TRY201: pins Ruff's double report when two nested handlers bound to the same name both reach the raise, and the single report when the inner try sits directly in the handler
    "TRY201-same-name-nested-handlers.py": "def same_name_twice():\n    try:\n        work()\n"
                                           "    except ValueError as e:\n        try:\n            try:\n"
                                           "                pass\n            except KeyError as e:\n"
                                           "                raise e\n        finally:\n            pass\n\n\n"
                                           "def inner_try_not_walked():\n    try:\n        work()\n"
                                           "    except ValueError as e:\n        try:\n            pass\n"
                                           "        except KeyError as e:\n            raise e\n",
    # TRY203: pins the handler end when the last statement holds strings with `#` or quotes, the trailing comment has quotes, and a trailing semicolon follows
    "TRY203-handler-end-strings-comments.py": "def quote_in_trailing_comment():\n    try:\n        work()\n"
                                              "    except ValueError:\n        raise\n"
                                              "        log('it\\'s # here')  # say \"hi\"\n\n\n"
                                              "def hash_in_triple_quoted_string():\n    try:\n        work()\n"
                                              "    except ValueError as e:\n        raise e\n        x = \"\"\"\n"
                                              "    # not a comment\"\"\"\n\n\ndef trailing_semicolon():\n    try:\n"
                                              "        work()\n    except ValueError as e:\n"
                                              "        raise e ;  # don't\n",
    # TRY300: pins that a t-string operand makes a binary operation side-effecting while an f-string operand does not
    "TRY300-template-string-operand.py": "def template_operand(x):\n    try:\n        n = 1\n"
                                         "        return t\"{x}\" + t\"a\"\n    except ValueError:\n        pass\n\n\n"
                                         "def format_string_operand(x):\n    try:\n        n = 1\n"
                                         "        return \"a\" + f\"{x}\"\n    except ValueError:\n        pass\n",
    # TRY301: Raises in a function or class defined inside the try body are reported, while raises in a nested try's handler or finally are left to that inner try.
    "TRY301-nested-def-and-nested-try.py": "def f():\n    try:\n        def g():\n            raise ValueError(\"x\")\n"
                                           "\n        class C:\n            raise ValueError\n\n        try:\n"
                                           "            pass\n        except KeyError:\n            raise ValueError\n"
                                           "        finally:\n            raise ValueError\n    except ValueError:\n"
                                           "        pass\n",
    # TRY400: Logger calls in the type expression of a handler nested inside another handler are not reported; only the nested handler's body is.
    "TRY400-nested-handler-type-not-visited.py": "import logging\n\nlogger = logging.getLogger(__name__)\n\n\n"
                                                 "def f():\n    try:\n        pass\n    except Exception:\n"
                                                 "        try:\n            pass\n"
                                                 "        except (logger.error(\"t\") or ValueError):\n"
                                                 "            logger.error(\"inner body\")\n"
                                                 "        except (lambda: logger.error(\"u\"))() as e:\n"
                                                 "            pass\n",
    # TRY401: Exception names inside an unparenthesized generator argument of the exception call (iterable, element and condition) are reported.
    "TRY401-bare-generator-argument.py": "import logging\n\nlogger = logging.getLogger(__name__)\n\n\ndef f(y):\n"
                                         "    try:\n        pass\n    except Exception as e:\n"
                                         "        logger.exception(str(x) for x in e)\n"
                                         "        logger.exception(e for _ in y)\n"
                                         "        logger.exception(x for x in y if e)\n",
    # UP006: Annotated metadata nested in Optional or in a parenthesized tuple slice is not a type definition; only the first element is.
    "UP006-annotated-metadata-in-generic.py": "from typing import Annotated, List, Optional\n"
                                              "a: Optional[Annotated[int, \"List[int]\"]]\n"
                                              "b = Optional[Annotated[int, \"List[int]\"]]\n"
                                              "c: Annotated[(\"List[int]\", \"List[int]\")]\n"
                                              "d = Annotated[(\"List[int]\", \"List[int]\"), \"List[int]\"]\n"
                                              "e: Optional[Annotated[\"List[int]\", \"List[int]\"]]\n",
    # UP007: A __future__ import other than annotations does not make annotations typing-only. A parenthesized TYPE_CHECKING, PEP 695 bounds and type statements are typing-only contexts.
    "UP007-typing-only-contexts.py": "from __future__ import division\nfrom typing import TYPE_CHECKING, Union\n\n"
                                     "a: Union[\"int\", str] = 1\nif (TYPE_CHECKING):\n    b: Union[\"int\", str]\n\n\n"
                                     "def f[T: Union[\"int\", str]](x: T) -> T: ...\n\n\ntype X = Union[\"int\", str]\n",
    # UP008: Recursive class chain four levels deep, parentheses around the callee and arguments, and the class name rebound by nonlocal or by a walrus in a comprehension.
    "UP008-deep-chain-and-parens.py": "class B: pass\nclass A:\n    class B:\n        class C:\n"
                                      "            class D(B):\n"
                                      "                def f(self): return super(A.B.C.D, self).f()\n"
                                      "                def g(self): return super(B.C.D, self).g()\n"
                                      "                def h(self): return super(((A.B.C.D)), (self)).h()\n"
                                      "                def i(self): return (super)(D, self).i()\ndef outer():\n"
                                      "    C = 1\n    class C(B):\n        def a(self):\n            nonlocal C\n"
                                      "            return super(C, self).a()\n        def b(self):\n"
                                      "            [(C := 1) for _ in range(2)]\n"
                                      "            return super(C, self).b()\n    return C\n",
    # UP009: A word that starts like `coding` (cocoa) before the real declaration must not stop the first-occurrence scan; Ruff reports this comment
    "UP009-partial-prefix-before-coding.py": "# cocoa coding: utf-8\nx = 1\n",
    # UP010: Ruff treats `from .__future__ import` as a future import regardless of level, so relative forms are reported too
    "UP010-relative-future-module.py": "from .__future__ import division\n"
                                       "from ..__future__ import generators, print_function\nprint(print_function)\n",
    # UP011: Comments with parentheses and a backslash continuation between the callee, the empty call and the closing parenthesis keep Ruff's exact range
    "UP011-comments-and-continuations.py": "import functools\n\n\n@(functools.lru_cache  # a (b)\n  )(  # c\n  \\\n"
                                           ")  # d\ndef f(): ...\n\n\n@functools.lru_cache(  # )\n)\n"
                                           "async def g(): ...\n",
    # UP012: a parenthesized `.encode` attribute as the callee, which Ruff's AST sees through
    "UP012-parenthesized-encode-callee.py": "(\"a\".encode)()\n(\"a\".encode)(\"utf-8\")\n(\"é\".encode)(\"utf-8\")\n"
                                            "((\"a\").encode)(encoding=\"utf8\")\n(\"a\" .encode)()\n",
    # UP013: a parenthesized dict literal or dict(...) fields argument, extra positional fields and a conditional fields expression
    "UP013-parenthesized-fields-argument.py": "from typing import TypedDict\n"
                                              "A = TypedDict(\"A\", {\"a\": int}, {\"b\": int}, total=False)\n"
                                              "B = TypedDict(\"B\", {\"a\": int} if x else {\"b\": int})\n"
                                              "C = TypedDict(\"C\", ({\"a\": int}))\n"
                                              "D = TypedDict(\"D\", (dict(a=int)))\n",
    # UP014: field-list shape rejections: 3-tuples, dunder names, starred annotation, nested lists, mixed keywords, tuple fields
    "UP014-field-shape-rejections.py": "from typing import NamedTuple as NT\nT = NT(\"T\", [(\"a\", int, 3)])\n"
                                       "W = NT(\"W\", [(\"__x__\", int)])\nX = NT(\"X\", [(\"_x\", int)])\n"
                                       "Z = NT(\"Z\", [(\"a\", *x)])\na1 = NT(\"a1\", [[\"a\", int]])\n"
                                       "a4 = NT(\"a4\", ())\nb2 = NT(\"b2\", [(\"a\", int)], a=int)\n"
                                       "if x: c5 = NT(\"c5\", [(\"a\", int)])\n",
    # UP015: a positional mode after a backslash continuation before its comma is reported, while a keyword mode or a preceding starred argument wins as in Ruff
    "UP015-continuation-before-comma.py": "open(\"f\" \\\n     , \"rt\")\nopen(\"f\" \\\n     , \"wt\")\n"
                                          "open(\"f\" \\\n     , \"rt\", mode=\"w\")\nopen(*a \\\n     , \"rt\")\n",
    # UP017: a longer dotted case value reports only the resolved timezone.utc prefix (at target 3.11)
    "UP017-case-dotted-prefix.py": "import datetime as dt\nfrom datetime import timezone as tz\n\nmatch 1:\n"
                                   "    case dt.timezone.utc.x:\n        pass\n    case tz.utc.y.z:\n        pass\n"
                                   "    case dt.timezone.min.x:\n        pass\n",
    # UP018: calls inside string annotations, including a nested quoted one, are reported at their position inside the string
    "UP018-string-annotation.py": "x: \"int(1)\"\ny: \"list['str()']\"\n\n\ndef f(a: \"bool(True)\") -> \"float()\":\n"
                                  "    pass\n",
    # UP019: Strings in a typing or builtin generic's slice outside annotations are type definitions Ruff reads for Text.
    "UP019-runtime-generic-string-slice.py": "from typing import Text, Optional, List, Dict, Union\nimport typing\n\n"
                                             "A = Optional[\"Text\"]\nB = List[\"Text\"]\nC = Dict[str, \"Text\"]\n"
                                             "D = Union[\"Text\", int]\nE = list[\"Text\"]\nF = dict[\"Text\", int]\n"
                                             "G = typing.Optional[\"typing.Text\"]\nH = tuple[\"Text\", ...]\n",
    # UP020: Ruff analyzes calls inside string annotations and cast type strings and reports io.open there.
    "UP020-call-inside-string-annotation.py": "import io\nimport typing\n\nx: \"io.open('f')\"\n"
                                              "y = typing.cast(\"io.open('f')\", 1)\n\n\n"
                                              "def f(a: \"io.open('a')\") -> \"list[io.open('b')]\":\n    pass\n",
    # UP021: Ruff reports universal_newlines in subprocess.run calls inside string annotations, choosing rename or drop by the text keyword.
    "UP021-call-inside-string-annotation.py": "import subprocess\n\n"
                                              "x: \"subprocess.run('ls', universal_newlines=True)\"\n"
                                              "y: \"subprocess.run('ls', universal_newlines=True, text=True)\"\n",
    # UP022: Calls inside string annotations are reported at their position in the string, in both clauses; a plain string is not.
    "UP022-string-annotation.py": "import subprocess\n"
                                  "x: \"subprocess.run(['a'], stdout=subprocess.PIPE, stderr=subprocess.PIPE)\"\n"
                                  "y: \"list[subprocess.run(['a'], stdout=subprocess.PIPE, stderr=subprocess.PIPE, capture_output=True)]\"\n"
                                  "w = \"subprocess.run(['a'], stdout=subprocess.PIPE, stderr=subprocess.PIPE)\"\n",
    # UP023: Backslash line continuations inside the dotted module name still match, as Ruff compares the parsed name.
    "UP023-line-continuation-in-dotted-name.py": "import xml.\\\n    etree.cElementTree as ET\nfrom xml.etree\\\n"
                                                 ".cElementTree import XML\nfrom xml.\\\n"
                                                 "  etree import cElementTree as C\nimport xml.etree.\\\n"
                                                 "    cElementTree\n",
    # UP024: Alias calls inside string annotations are reported in the string; a parenthesized tuple before a Python 3.14 except comma is not, but an inner try's tuple is.
    "UP024-string-annotation-and-comma-except.py": "import socket\n"
                                                   "def f(x: \"IOError(1)\") -> \"list[socket.error()]\":\n    pass\n"
                                                   "y: \"IOError\"\ntry:\n    pass\nexcept (IOError, KeyError), e:\n"
                                                   "    pass\nexcept KeyError, e:\n    try:\n        pass\n"
                                                   "    except (IOError, KeyError):\n        pass\n",
    # UP026: Default values and annotations are visited before a later import (not reported) while function bodies, lambda bodies and string annotations are visited after it, including a nested function after an import in its enclosing function.
    "UP026-deferred-visit-order.py": "def f(a=mock.mock.X, b: mock.mock.Y = None):\n    return mock.mock.Z\n\n\n"
                                     "g = lambda a=mock.mock.X: mock.mock.Y\nh: \"mock.mock.X\"\n\n\ndef outer():\n"
                                     "    def inner():\n        return (mock).mock.X\n    import mock\n"
                                     "    return inner\n",
    # UP028: An unconditional del followed by a rebinding drops the reads before it; a conditional del, a parenthesized del with a later read, and a del without a rebinding do not.
    "UP028-del-restarts-reads.py": "def f():\n    for x in y:\n        yield x\n    print(x)\n    del x\n    x = 1\n\n"
                                   "\ndef g():\n    for x in y:\n        yield x\n    if c:\n        del x\n    x = 1\n"
                                   "\n\ndef h():\n    for x in y:\n        yield x\n    del (x)\n    x = 1\n"
                                   "    print(x)\n\n\ndef i():\n    for x in y:\n        yield x\n    del x\n"
                                   "    del x\n",
    # UP029: A name bound twice in one statement is not reported, an annotation without a value and a type parameter do not shadow, and a nonlocal declaration does.
    "UP029-same-statement-and-type-params.py": "from builtins import str, str\n"
                                               "from builtins import zip, map as zip, max\nstr2: int\n"
                                               "from builtins import min\n\n\ndef f[int]():\n"
                                               "    from builtins import int\n\n\ndef g():\n    def outer():\n"
                                               "        pass\n    list = 1\n    def h():\n        nonlocal list\n"
                                               "        from builtins import list\n",
    # UP030: A sole generator argument with an in-order template, starred arguments after the permuted ones, and .format calls inside string annotations.
    "UP030-generator-and-annotation.py": "a = b = c = 1\n\"{0}\".format(x for x in a)\n"
                                         "\"{1} {0}\".format(x for x in a)\n\"{1} {0}\".format(a, b, *c, *c)\n"
                                         "\"{1} {0}\".format(a, *c, b)\nx: \"'{1} {0}'.format(a, b)\"\n"
                                         "y: \"'{0} {0}'.format(a, b)\"\n",
    # UP031: Inside string annotations: a percent string on the right of % is not the format, and dict right sides are reported whole or at the string as Ruff does.
    "UP031-annotation-operands-and-dicts.py": "x = 1\na: \"x % '%s'\"\ne: \"'%(a)s' % {'a': 1}\"\n"
                                              "j: \"'%(a)s' % {'a': 1, 'a': 2}\"\nk: \"'%s' % {'a': 1}\"\n",
    # UP032: A quote inside a comment within a parenthesized argument does not block the report; sole generator arguments follow Ruff's per-segment and auto-index rules; an annotation .format on a non-string object is silent.
    "UP032-comment-quote-and-generator.py": "a = 1\n\"{}\".format((a  # it's\n))\n\"{x}\".format(x=(a  # it's\n))\n"
                                            "\"{}\".format(x for x in a)\n\"{}\" \"{}\".format(x for x in a)\n"
                                            "\"{0}\" \"{0}\".format(x for x in a)\nt: \"f('{}').format(a)\"\n",
    # UP033: A comment inside a parenthesized callee and around the keyword: the range still starts right after the callee name, as Ruff's func.end().
    "UP033-comments-around-callee.py": "import functools\n\n\n@(functools.lru_cache  # c\n)(maxsize=None)\n"
                                       "def a(): pass\n\n\n@functools.lru_cache(  # c\n    maxsize=None  # d\n)\n"
                                       "def b(): pass\n",
    # UP034: Token-scan consumption up to the first depth-1 comma (lambda parameters, comprehension targets, a tuple nested at the start of another scan), a callee group that is itself an outer candidate, and a class-pattern argument.
    "UP034-scan-consumption-lambda-target-pattern.py": "x = ((lambda a=((1)), b=2: 0))\ny = ((v for ((a)), b in c))\n"
                                                       "z = (((x, y) + ((z)), w))\n"
                                                       "w = ((f(a))(((b))((b if a else b))))\nmatch x:\n"
                                                       "    case Point(((1))):\n        pass\n",
    # UP035: One statement yields one finding per destination module plus one per renamed name, all with the same range, and noqa on the last line of a parenthesized import.
    "UP035-multiple-destinations-and-renames.py": "from typing import Callable, Match, Counter, List, Dict\n"
                                                  "from typing import (\n    List,  # noqa: UP035\n)\n",
    # UP036: an invalid version tuple or integer wrapped in extra parentheses is reported at the inner literal, not the parentheses
    "UP036-invalid-specifier-inside-parentheses.py": "import sys\n\nif sys.version_info[0] == (1000):\n    pass\n\n"
                                                     "if sys.version_info >= ((3, 1000)):\n    pass\n\n"
                                                     "if sys.version_info < ( (2, 1000) ):\n    pass\n\n"
                                                     "if sys.version_info <= ((3, 5, 1000)):\n    pass\n",
    # UP037: functional TypedDict keyword values: total never, extra_items always, legacy names only with a name-only or name-and-None call, unparsable values skipped
    "UP037-typeddict-keyword-fields.py": "from typing import TypedDict\n\ndef f():\n"
                                         "    a: TypedDict(\"E\", total=\"int\") = 1\n"
                                         "    b: TypedDict(\"E\", {\"k\": int}, foo=\"int\") = 1\n"
                                         "    c: TypedDict(\"E\", None, foo=\"int\") = 1\n"
                                         "    d: TypedDict(\"E\", extra_items=\"int\") = 1\n"
                                         "    e: TypedDict(\"E\", foo=\"a b\") = 1\n"
                                         "    g: TypedDict(\"E\", foo=\"list[\") = 1\n"
                                         "    i: TypedDict(\"E\", {\"k\": \"int\"}) = 1\n"
                                         "    j: TypedDict(\"E\", **{\"k\": \"int\"}) = 1\n"
                                         "    k: TypedDict(\"E\", foo=\"int | None\") = 1\n",
    # UP040: f-string/bytes TypeVar names and ** splats do not block the alias, parenthesized type_params names are accepted, chained targets are skipped (pins silence at the recorded 3.10 target; verified at py312)
    "UP040-typevar-shapes-and-type-params.py": "from typing import ParamSpec, TypeAlias, TypeAliasType, TypeVar\n\n"
                                               "T1 = TypeVar(f\"T1\", default=int)\n"
                                               "T2 = TypeVar(b\"T2\", default=int)\nP1 = ParamSpec(\"P1\", **kw)\n"
                                               "T = TypeVar(\"T\")\nU = TypeVar(\"U\")\n\na1: TypeAlias = list[T1]\n"
                                               "a2: TypeAlias = list[T2]\na3: TypeAlias = Callable[P1, int]\n"
                                               "A = TypeAliasType(\"A\", list[T], type_params=((T),))\n"
                                               "B = TypeAliasType(\"B\", list[T], type_params=(T, (U)))\n"
                                               "X = X = TypeAliasType(\"X\", int)\nY = Z = TypeAliasType(\"Z\", int)\n",
    # UP043: string annotations (nested, concatenated, escaped), parenthesized-tuple index forms with and without a trailing comma, and an over-long Generator
    "UP043-string-annotations-and-tuple-forms.pyi": "from typing import AsyncGenerator, Generator, List, Optional\n\n"
                                                    "a: Optional[\"Generator[int, None]\"]\n"
                                                    "b: \"List['Generator[int, None]']\"\n"
                                                    "c: (\"Generator[int, None, None]\" \"\")\n"
                                                    "d: \"Generator[int, None, \\x4eone]\"\n"
                                                    "e: Generator[(int, None),]\nf: Generator[((int, None))]\n"
                                                    "g: Generator[int, None, None, None]\n"
                                                    "h: AsyncGenerator[(int), (None)]\n",
    # UP045: Pins string type definitions outside annotations (typing generic slices, PEP 695 type alias values, type parameter bounds) and quoted Optional in type alias and type parameter typing contexts.
    "UP045-runtime-string-type-definitions.py": "from typing import Optional, List\n\na = List[\"Optional[int]\"]\n"
                                                "b = Optional[\"Optional[int]\"]\nc = list[\"Optional[int]\"]\n"
                                                "d = dict[str, \"Optional[int]\"]\ne = foo[\"Optional[int]\"]\n"
                                                "type X = \"Optional[int]\"\ntype Y = Optional[\"A\"]\n"
                                                "type Z = list[Optional[\"A\"]]\n"
                                                "def f[T: Optional[\"A\"]](x: T) -> T: ...\n"
                                                "def g[T: \"Optional[int]\"](x: T) -> T: ...\n"
                                                "class C[T: Optional[\"A\"] = Optional[\"B\"]]: ...\n",
    # UP049: underscored names in bounds, constraints and defaults (including starred defaults) are not reported; only the parameter names are
    "UP049-bound-default-names-not-reported.py": "class A[_T: _B = _D]: ...\n"
                                                 "def f[X: _B = _D, _Y: (_C, _E) = _F](): ...\n"
                                                 "class B[*_Ts = *_Us, **_P = _Q]: ...\n",
    # YTT101: One-element starred list/set displays (attribute and from-import forms, comment after the star, empty step) fire at the range after the star; a two-element display and a lower-bounded slice behave as elsewhere.
    "YTT101-starred-one-element-display.py": "import sys\nfrom sys import version\n\na = [*sys.version[:3]]\n"
                                             "b = {*version[:3]}\nc = [*  # c\n     sys.version[:3:]]\n"
                                             "d = [*sys.version[:3], 1]\ne = [*sys.version[1:3]]\n",
    # YTT102: One-element starred list/set displays fire at the range after the star, including the from-import form; index 1 stays silent.
    "YTT102-starred-one-element-display.py": "import sys\nfrom sys import version\n\na = [*sys.version[2]]\n"
                                             "b = {*version[2]}\nc = [*sys.version [2]]\nd = [*sys.version[1]]\n",
    # YTT103: Length is the decoded UTF-8 byte length: a raw two-character escape, an astral escape, an empty concatenation and a two-byte octal escape fire; one-byte escapes, bytes and t-strings stay silent.
    "YTT103-escapes-and-prefixes.py": "import sys\n\nsys.version < r\"\\n\"\nsys.version < \"\\n\"\n"
                                      "sys.version < \"\\x41\"\nsys.version < \"\\U0001F600\"\n"
                                      "sys.version < \"\" \"\"\nsys.version < rb\"34\"\nsys.version < t\"34\"\n"
                                      "sys.version < \"\\777\"\n",
    # YTT201: Parenthesized operands, hex literal and alias fire; reversed, chained, tuple index and float stay silent.
    "YTT201-parenthesized-operands.py": "import sys\nfrom sys import version_info as vi\n\n"
                                        "sys.version_info[(0)] == (3)\n(sys.version_info[0]) != 3\n"
                                        "(sys.version_info)[0] == 0x3\nvi[0] != 3\n3 == sys.version_info[0]\n"
                                        "sys.version_info[0] == 3 != 4\nsys.version_info[0, 1] == 3\n"
                                        "sys.version_info[0] == 3.0\n",
    # YTT202: Value patterns (bare, aliased with spaces, nested in sequence and mapping patterns, head of a longer dotted name, from-import head) fire at six.PY3; a capture pattern and six.PY2 stay silent.
    "YTT202-match-value-patterns.py": "import six\nimport six as s\nfrom six import PY3\n\nmatch x:\n"
                                      "    case six.PY3:\n        pass\n"
                                      "    case [s . PY3, 1] | {\"k\": six.PY3.real}:\n        pass\n"
                                      "    case PY3.y | six.PY2:\n        pass\n    case PY3:\n        pass\n",
    # YTT203: a bare `sys: int` annotation does not shadow the import, an annotated assignment does
    "YTT203-bare-annotation-keeps-import.py": "import sys\n\nsys: int\nsys.version_info[1] > 5\n\n\ndef f():\n"
                                              "    sys: int = 3\n    return sys.version_info[1] > 5\n",
    # YTT204: class-level names are invisible inside a comprehension body but visible in its first iterable
    "YTT204-class-scope-comprehension.py": "import sys\n\n\nclass C:\n    sys = None\n"
                                           "    a = [sys.version_info.minor > 5 for _ in range(3)]\n"
                                           "    b = [x for x in [sys.version_info.minor > 5]]\n",
    # YTT301: after an except handler that rebinds `sys`, the name resolves to the import again, as Ruff models it
    "YTT301-except-name-restores-import.py": "import sys\n\ntry:\n    pass\nexcept Exception as sys:\n    pass\n\n"
                                             "sys.version[0]\n",
    # YTT302: Ruff keeps raw CRLF in string values (two bytes) while a lone CR or backslash-continuation does not add a byte
    "YTT302-crlf-string-byte-length.py": "import sys\r\nsys.version < \"\"\"\r\n\"\"\"\r\nsys.version < \"\\\r\n3\"\r\n"
                                         "sys.version < (\r\n    \"3\"\r\n    \"\"\r\n)\r\nsys.version < \"\"\"\\\r\n"
                                         "3\"\"\"\r\nsys.version < \"\"\"\\\\\r\n\"\"\"\r\nsys.version < \"3\"\r\n"
                                         "sys.version < (\"\" \"\"\"\r\n\"\"\")\r\nsys.version < \"\"\"\r\"\"\"\r\n",
    # YTT303: an empty step `[:1:]` matches while `[:1:None]`, `[::1]`, `[None:1]` and a tuple subscript do not
    "YTT303-slice-step-and-bounds.py": "import sys\n\nsys.version[:1:]\nsys.version[:1:None]\nsys.version[::1]\n"
                                       "sys.version[None:1]\nsys.version[:1,]\n",

}


def authored() -> dict[str, str]:
    """Rerail's own cases, by path under fixtures/."""
    noqa = common.suppression_cases(NOQA, NOQA_COMMENTS, "ruff", paths={code: f"{code}.pyi" for code in NOQA_STUBS},
                                    exempt=NOQA_EXEMPT)
    cases = {f"rerail/noqa/{path}": text for path, text in noqa.items()}
    cases.update({f"rerail/probes/{name}": text for name, text in PROBES.items()})
    return cases


def ruff(executable: str, paths: list[str], cwd: pathlib.Path) -> list[dict]:
    """The ported rules' findings over `paths` (relative to `cwd`), in batches."""
    findings = []
    for start in range(0, len(paths), 500):
        run = subprocess.run(
            [executable, "check", "--isolated", "--no-cache", "--exit-zero", "--output-format", "json",
             "--target-version", common.TARGET_VERSION, "--select", ",".join(sorted(RULES)),
             *paths[start:start + 500]],
            capture_output=True, text=True, cwd=cwd, check=True,
        )
        for diagnostic in json.loads(run.stdout):
            if diagnostic["code"] == "invalid-syntax":
                continue
            assert diagnostic["code"] in RULES, diagnostic
            findings.append({
                "rule": diagnostic["code"],
                "path": pathlib.Path(diagnostic["filename"]).resolve().relative_to(cwd.resolve()).as_posix(),
                "line": diagnostic["location"]["row"],
                "column": diagnostic["location"]["column"],
            })
    return findings


def not_copied() -> list[str]:
    """The upstream path prefixes licenses/ruff/UPSTREAM.md lists as "Not copied": fixtures of copyleft plugins' ports."""
    record = HERE.parent.parent / "licenses" / "ruff" / "UPSTREAM.md"
    prefixes = re.findall(r"^- Not copied: `([^`]+)`", record.read_text(encoding="utf-8"), re.MULTILINE)
    if not prefixes:
        raise SystemExit(f"{record} lists no \"Not copied\" upstream paths")
    return prefixes


def fixtures(checkout: pathlib.Path, executable: str) -> None:
    excluded = not_copied()
    refused = sorted(file for paths in RULES.values() for file in paths
                     if any(f"{FIXTURES}/{file}".startswith(prefix) for prefix in excluded))
    if refused:
        raise SystemExit(f"RULES names fixtures licenses/ruff/UPSTREAM.md lists as not copied: {', '.join(refused)}")
    version = common.tool_version(executable, VERSION)
    head = subprocess.run(["git", "-C", str(checkout), "rev-parse", "HEAD"], capture_output=True, text=True,
                          check=True).stdout.strip()
    if head != ORIGIN.rsplit("@", 1)[1]:
        raise SystemExit(f"{checkout} is at {head}, not Ruff {VERSION}")
    # Rerail's cases first: a shipped rule without one stops the run before any fixture is deleted.
    cases = authored()
    target = HERE / "fixtures"
    # Only the files the last run wrote: fixtures/ also holds the case sources the port readiness recorder commits
    # (conformance/recorders/port-readiness/record.ts), which this run must keep.
    expected = HERE / "fixtures.expected.json"
    for file in json.loads(expected.read_text(encoding="utf-8"))["source"]["files"] if expected.exists() else []:
        (target / file).unlink(missing_ok=True)
    files = sorted({file for paths in RULES.values() for file in paths})
    for file in files:
        (target / file).parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(checkout / FIXTURES / file, target / file)
    ours = common.write_authored(target, cases)
    common.write_record("ruff", f"Ruff {VERSION}", ours, "`NOQA`, `PROBES`")
    files = sorted(files + ours)
    common.write(HERE / "fixtures.expected.json", "ruff", version,
                 {"kind": "fixtures", "origin": ORIGIN, "rules": sorted(RULES), "files": files},
                 ruff(executable, files, target))


def corpus(executable: str, name: str, root: pathlib.Path) -> None:
    version = common.tool_version(executable, VERSION)
    files = common.corpus_files(root, name)
    skipped = common.unparsable(root, files)
    analysed = sorted(set(files) - set(skipped))
    common.write(HERE / f"corpus-{name}.expected.json", "ruff", version,
                 common.corpus_source(name, sorted(RULES), skipped), ruff(executable, analysed, root))


def main() -> None:
    common.require_python()
    if sys.argv[1] == "fixtures":
        fixtures(pathlib.Path(sys.argv[2]).resolve(), sys.argv[3])
    else:
        for argument in sys.argv[3:]:
            name, _, path = argument.partition("=")
            corpus(sys.argv[2], name, pathlib.Path(path).resolve())


if __name__ == "__main__":
    main()
