"""Serialize JSON container types and member positions; make no reporter decisions."""


def kind(value):
    return {dict: 'object', list: 'array', str: 'string', bool: 'boolean',
            int: 'number', float: 'number', type(None): 'null'}[type(value)]


def members(value):
    if isinstance(value, dict): return list(value.items())
    if isinstance(value, list): return list(enumerate(value))
    return []


def encode_bundles(bundles):
    # Retain the untouched value for passive model DTO transport. Member keys
    # are text and positions are numbers, including malformed mixed containers.
    return [dict(bundle, kind=kind(bundle['value']), rows=[
        dict(key=str(key), kind=kind(row), fields=[dict(key=str(k), value=v)
            for k,v in members(row)]) for key,row in members(bundle['value'])])
        for bundle in bundles]
