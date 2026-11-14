import pickle as pk
from pickle import loads


def read_blob(blob):
    return pk.loads(blob)


def read_other(blob):
    return loads(blob)
