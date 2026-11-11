"""Lossless source API values, with only the caller's temporary root normalized."""
from dataclasses import fields, is_dataclass
from enum import Enum
from pathlib import Path
import json


def canonical(value, root=None):
    if isinstance(value,type):
        return dict(kind='type',name=value.__module__+'.'+value.__qualname__)
    if isinstance(value, bytes):
        return dict(kind='bytes', hex=value.hex())
    if isinstance(value, Enum):
        return dict(kind='enum', type=type(value).__module__ + '.' + type(value).__qualname__,
                    name=value.name, value=canonical(value.value, root))
    if isinstance(value, str) and root is not None:
        prefix = root.as_posix().rstrip('/') + '/'
        if value.startswith(prefix):
            return value[len(prefix):]
    if isinstance(value, Path):
        path = value
        if root is not None and path.is_absolute() and path.is_relative_to(root):
            path = path.relative_to(root)
        return dict(kind='path', value=path.as_posix())
    if is_dataclass(value) and not isinstance(value, type):
        return dict(kind='dataclass', type=type(value).__module__ + '.' + type(value).__qualname__,
                    fields={f.name: canonical(getattr(value, f.name), root) for f in fields(value)})
    if isinstance(value, tuple):
        return dict(kind='tuple', items=[canonical(item, root) for item in value])
    if isinstance(value, (set, frozenset)):
        items=[canonical(item, root) for item in value]
        return dict(kind='frozenset' if isinstance(value, frozenset) else 'set',
                    items=sorted(items,key=lambda item:json.dumps(item,sort_keys=True,ensure_ascii=True)))
    if isinstance(value, list):
        return [canonical(item, root) for item in value]
    if isinstance(value, dict):
        return dict(kind='dict', items=[[canonical(k, root), canonical(v, root)] for k, v in value.items()])
    if value is None or isinstance(value, (str, int, float, bool)):
        return value
    raise TypeError(f'No lossless source DTO contract for {type(value).__module__}.{type(value).__qualname__}')
