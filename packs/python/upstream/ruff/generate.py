"""Records Ruff 0.16.9 for the Python pack's ported rules (rerail/packs/python/conformance.json).
Run by `npm run fixtures:derive` (a maintainer, or the bump workflow for an upstream release; never pull request CI),
under CPython 3.14:

    python3.14 generate.py fixtures <Ruff checkout at tag 0.16.9> <ruff executable, 0.16.9>

`fixtures` copies each rule's own test fixtures (the files its tests lint with default settings, under
`crates/ruff_linter/resources/test/fixtures/<linter>/`; see RULES) to fixtures/<linter>/<file> (the pack's
licenses/ruff/UPSTREAM.md lists them), writes Rerail's own cases (NOQA, PROBES) to fixtures/rerail/ (listed in
upstream/ruff/UPSTREAM.md), and runs Ruff with every ported rule selected over every file. It runs Ruff isolated from any
configuration, at target version py310 (Ruff's CLI default and the engine's default `pythonVersion`, which the ports
keep), with `# noqa` honoured, as Ruff runs by default. Each finding is the rule's code, path, line and column
(Ruff's, 1-based, in characters). `source.units` names each copied fixture's origin: the rules RULES lists it for, whose
upstream tests lint it (Rerail's own cases have none)."""

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
# candidate A005. Nor is the candidate INP001, which Ruff's tests lint only with a `namespace-packages` option.
RULES: dict[str, list[str]] = {
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
    "PIE790": ["flake8_pie/PIE790.py", "flake8_pyi/PYI021_1.pyi"],
    "PIE794": ["flake8_pie/PIE794.py"],
    "PIE796": ["flake8_pie/PIE796.py", "flake8_pie/PIE796.pyi"],
    "PIE800": ["flake8_pie/PIE800.py"],
    "PIE804": ["flake8_pie/PIE804.py"],
    "PIE807": ["flake8_pie/PIE807.py"],
    "PIE808": ["flake8_pie/PIE808.py"],
    "PIE810": ["flake8_pie/PIE810.py"],
    "RET501": ["flake8_return/RET501.py"],
    "RET502": ["flake8_return/RET502.py"],
    "RET503": ["flake8_return/RET503.py"],
    "RET504": ["flake8_return/RET504.py"],
    "RET505": ["flake8_return/RET505.py"],
    "RET506": ["flake8_return/RET506.py"],
    "RET507": ["flake8_return/RET507.py"],
    "RET508": ["flake8_return/RET508.py"],
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
    "T100": ["flake8_debugger/T100.py"],
    "W605": ["pycodestyle/W605_0.py", "pycodestyle/W605_1.py"],
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
}

NOQA_COMMENTS = ["", "  # noqa", "  # noqa: {code}", "  # NOQA: {code}", "  # noqa: E501"]

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
}


def authored() -> dict[str, str]:
    """Rerail's own cases, by path under fixtures/."""
    noqa = common.suppression_cases(NOQA, NOQA_COMMENTS, "ruff")
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
    # Only the files the last run wrote: the rest of fixtures/ is not this run's to delete.
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
                 {"kind": "fixtures", "origin": ORIGIN, "rules": sorted(RULES), "files": files,
                  "units": common.origins(RULES)},
                 ruff(executable, files, target))


def main() -> None:
    common.require_python()
    if sys.argv[1] == "fixtures":
        fixtures(pathlib.Path(sys.argv[2]).resolve(), sys.argv[3])
    else:
        raise SystemExit(__doc__)


if __name__ == "__main__":
    main()
