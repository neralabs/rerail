# SPDX-License-Identifier: Apache-2.0
"""Use the production observer in-process so upstream monkeypatches remain effective."""
from pathlib import Path
from runpy import run_path


def observations():
 path=Path(__file__).resolve().parents[3]/'packs/data-layer/runtime/observations.py'
 return run_path(str(path))['observations']()
