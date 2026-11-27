import os

BASE = "/srv/docs"


def read(name):
    with open(os.path.join(BASE, name)) as fh:  # caught
        return fh.read()


def resolve_safe(name):
    target = os.path.realpath(os.path.join(BASE, name))
    if not target.startswith(BASE):
        raise ValueError(name)
    return target


def locate(name):
    target = os.path.realpath(os.path.join(BASE, name))
    if os.path.commonpath([BASE, target]) != BASE:
        raise ValueError(name)
    return target


def read_bundled(name):
    with open(os.path.join(BASE, "bundled", name)) as fh:
        return fh.read()


def load_defaults():
    return read_bundled("defaults.json")
