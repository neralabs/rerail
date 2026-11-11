"""Carry container types and primitive values without non-finite JSON numbers."""
from fact_transport import kind
from python_value_transport import encode


def value_kind(value):
    return 'tuple' if isinstance(value,tuple) else kind(value)


def members(value):
    if isinstance(value,dict):return list(value.items())
    if isinstance(value,(list,tuple)):return list(enumerate(value))
    return []


def tagged(value):
    if isinstance(value,tuple):return ['tuple',[tagged(v) for v in value]]
    if isinstance(value,list):return ['list',[tagged(v) for v in value]]
    if isinstance(value,dict):return ['dict',[[k,tagged(v)] for k,v in value.items()]]
    return encode(value)


def encode_bundles(bundles):
    return [dict({k:v for k,v in bundle.items() if k!='value'},kind=value_kind(bundle['value']),rows=[
        dict(key=str(key),kind=value_kind(row),fields=[dict(key=str(k),encoded=tagged(v)) for k,v in members(row)])
        for key,row in members(bundle['value'])]) for bundle in bundles]
