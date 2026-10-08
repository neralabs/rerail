module_value = 1
module_only: int
print(module_only)


class Body:
    class_only: int
    read_in_class = class_only


def hides_outer_names():
    module_value: int
    len: int
    return module_value, len


def initialized_by_a_nested_nonlocal():
    value: int

    def set_value(new):
        nonlocal value
        value = new

    set_value(1)
    return value


def read_from_a_nested_scope():
    later: int

    def inner():
        return later

    return inner


def assigned_first():
    first = 1
    first: int
    return first


def annotated_then_assigned():
    then: int
    then = 1
    return then
