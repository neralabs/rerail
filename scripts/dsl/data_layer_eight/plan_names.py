# SPDX-License-Identifier: Apache-2.0
"""Shared SQL expressions for plan-helper path and name contracts."""
import re
from data_layer_six.common import q


def path_scope(path, skipped):
    markers = r'(?:^|/)(?:impl|api)(?:/|$)|(?:^|/)l[0-9]{4}_'
    excluded = r'(?:^|/)(?:' + '|'.join(re.escape(value) for value in sorted(skipped)) + r')(?:/|$)'
    return f"substr({path},-3)='.py' AND NOT rx_matches({path},'(?:^|/)\\.py$') AND rx_matches({path},{q(markers)}) AND NOT rx_matches({path},{q(excluded)})"


def module_sql(path):
    # Normalize components first, and strip .py only when it is present.
    normalized = f"trim(rx_replace(replace({path},char(92),'/'),'/+','/'),'/')"
    raw = f"replace(CASE WHEN substr({normalized},-3)='.py' THEN substr({normalized},1,length({normalized})-3) ELSE {normalized} END,'/','.')"
    return f"CASE WHEN instr('.'||{raw}||'.','.impl.')>0 THEN substr({raw},instr('.'||{raw}||'.','.impl.')) WHEN instr('.'||{raw}||'.','.api.')>0 THEN substr({raw},instr('.'||{raw}||'.','.api.')+4) ELSE coalesce(rx_extract({raw},'(?:^|\\.)(l[0-9]{{4}}_.*)$',1),{raw}) END"


def recognized(name, module_alias):
    return f"({name} IN (SELECT name FROM helper_names) OR {name} GLOB '_*via_plan*' AND ({module_alias}))"
