"""n/no-unsupported-features/es-syntax's features, one eslint-plugin-es-x rule each, gathered from this package's modules.

Each module defines FEATURES: es-x rule id (`no-arrow-functions`) → {"rule": AST rule matching each node the es-x rule
reports, at the node it reports, "utils"?: its utilities, "languageOptions"?: what it adds to the Node rules' module
system (globals, say), "when"?: the clause's gate on the file's configuration (`{"sourceType": "module"}`), "file"?: file
conditions beside the Node range gate (`{"path": {"regex": "\\.[cm]?tsx?$"}}`)}, or a list of such specs, one clause each. es_syntax_doc in node_policies.py makes one clause per feature, gated on the configured Node
range (and on strictness for the features es-syntax.json gives a strictMode range).
"""
import importlib
import pkgutil

FEATURES = {}
for _module in sorted(info.name for info in pkgutil.iter_modules(__path__)):
    for _feature, _spec in importlib.import_module(f"{__name__}.{_module}").FEATURES.items():
        assert _feature not in FEATURES, f"{_feature} is defined twice"
        FEATURES[_feature] = _spec
