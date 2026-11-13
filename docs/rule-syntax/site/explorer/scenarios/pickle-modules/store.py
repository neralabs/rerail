import json
import shelve

import dill


def open_store(path):
    return shelve.open(path)


def thaw(blob):
    return dill.loads(blob)


def parse(text):
    return json.loads(text)
