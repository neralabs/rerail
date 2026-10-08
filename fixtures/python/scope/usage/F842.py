import os


def unused_annotations():
    bare: int
    (parenthesized): int
    _dummy: int


def read_in_its_scope():
    read_here: int
    print(read_here)


def read_from_a_nested_scope():
    read_nested: int

    def inner():
        return read_nested

    return inner


def initialized_by_a_nonlocal():
    shared: int

    def inner():
        nonlocal shared
        shared = 1

    return inner


def assigned_later():
    then_assigned: int
    then_assigned = 1


def annotated_after_an_assignment():
    first = 1
    first: int
    return first


def shadows_an_import():
    os: int
    print(os)


class Body:
    in_a_class: int


at_module_level: int
