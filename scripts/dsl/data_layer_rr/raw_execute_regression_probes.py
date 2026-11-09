"""Supply caller configuration and execute the selected API without a semantic fallback."""
from contextlib import ExitStack
from unittest.mock import patch


def probe(module,entry,values):
    values=dict(values);root=values.pop('root');compatibility=values.pop('compatibility',None)
    with ExitStack() as stack:
        stack.enter_context(patch.object(module,'REPO_ROOT',root))
        if compatibility is not None:stack.enter_context(patch.object(module,'COMPATIBILITY_STUB_FILES',set(compatibility)))
        if entry=='context':return module._FunctionContext(**values).is_raising_stub
        return getattr(module,entry)(**values)
