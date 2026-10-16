"""Bandit, as the port readiness gates run it, inside the pinned runtime of conformance/recorders/port-readiness/runtimes/bandit-<version>.

One JSON request on stdin, one JSON response on stdout:
- {"op": "facts"}: every registered test id with what upstream's own registry says about it.
- {"op": "lint", "files": [...]}: Bandit's default configuration (no profile, no config file, `# nosec` honoured,
  every registered test with its gen_config defaults) on the given paths, as bare `bandit <files>` runs.

It reads only the extension manager's registries, gen_config, BanditManager and BanditTestSet, Issue's positions and
the manager's NOSEC_COMMENT.
"""

import importlib
import importlib.metadata
import json
import re
import sys


def facts():
    from bandit.core.extension_loader import MANAGER

    rules = {}
    for test_id, plugin in MANAGER.plugins_by_id.items():
        function = plugin.plugin
        module = importlib.import_module(function.__module__)
        options = module.gen_config(function._takes_config) if getattr(function, "_takes_config", None) else None
        rules[test_id] = {"kind": "plugin", "name": plugin.name, "module": function.__module__, "file": module.__file__, "options": options}
    for test_id, entries in MANAGER.blacklist_by_id.items():
        if test_id == "B001":
            continue
        entry = entries[0] if isinstance(entries, list) else entries
        rules[test_id] = {
            "kind": "blacklist",
            "name": entry["name"],
            "module": "bandit.blacklists.calls" if test_id.startswith("B3") else "bandit.blacklists.imports",
            "file": importlib.import_module("bandit.blacklists.calls" if test_id.startswith("B3") else "bandit.blacklists.imports").__file__,
            "options": None,
            "qualnames": sorted({name for item in (entries if isinstance(entries, list) else [entries]) for name in item["qualnames"]}),
        }
    # Patterns upstream builds at run time (str.format, concatenation) cannot be read from source; read the objects.
    regexes = {}
    for name in sorted({rule["module"] for rule in rules.values()} | {"bandit.core.utils", "bandit.core.context"}):
        module = importlib.import_module(name)
        regexes[name] = [
            {"pattern": value.pattern, "flags": value.flags}
            for value in vars(module).values()
            if isinstance(value, re.Pattern) and isinstance(value.pattern, str)
        ]
    import bandit.core.manager as manager_module

    return {
        "version": importlib.metadata.version("bandit"),
        "license": importlib.metadata.metadata("bandit").get("License-Expression") or importlib.metadata.metadata("bandit").get("License"),
        "python": ".".join(str(part) for part in sys.version_info[:3]),
        "rules": rules,
        "regexes": regexes,
        "nosec": manager_module.NOSEC_COMMENT.pattern if hasattr(manager_module.NOSEC_COMMENT, "pattern") else str(manager_module.NOSEC_COMMENT),
    }


def lint(paths):
    from bandit.core import config, manager, test_set

    conf = config.BanditConfig()
    run = manager.BanditManager(conf, "file", ignore_nosec=False)
    run.b_ts = test_set.BanditTestSet(config=conf)
    run.discover_files(paths, False)
    run.run_tests()
    issues = {path: [] for path in paths}
    for issue in run.get_issue_list():
        issues.setdefault(issue.fname, []).append({
            "test_id": issue.test_id,
            "line": issue.lineno,
            "col": issue.col_offset,
            "end_col": issue.end_col_offset,
            "line_range": list(issue.linerange),
        })
    skipped = {path: reason for path, reason in run.get_skipped()}
    return {"issues": issues, "skipped": skipped}


def main():
    request = json.load(sys.stdin)
    if request["op"] == "facts":
        response = facts()
    elif request["op"] == "lint":
        response = lint(request["files"])
    else:
        raise SystemExit(f"unknown op {request['op']}")
    json.dump(response, sys.stdout)


if __name__ == "__main__":
    main()
