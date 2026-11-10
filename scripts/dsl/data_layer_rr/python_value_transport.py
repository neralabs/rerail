"""Lossless primitive value transport. Coercion and formatting belong to native readers."""
import struct


def encode(value):
    kind=type(value)
    if value is None:return ['null',None]
    if kind is bool:return ['bool',value]
    if kind is int:return ['int',str(value)]
    if kind is float:return ['float',struct.pack('>d',value).hex()]
    if kind is str:return ['str',value]
    if kind is list:return ['list',[encode(v) for v in value]]
    if kind is dict:return ['dict',[[k,encode(v)] for k,v in value.items()]]
    raise TypeError('Unsupported primitive transport type: '+kind.__name__)
