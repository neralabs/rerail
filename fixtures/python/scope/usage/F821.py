import os

print(undefined_module_name)
os.path.join(also_undefined)

known = 1
del known
print(known)

maybe = 1
if os.sep:
    del maybe
print(maybe)


def function_body():
    return later_defined, never_defined


later_defined = 1

try:
    optional_name
except NameError:
    optional_name = None


class Body:
    __qualname__
    __module__
    inside_class = 1
    value = inside_class

    def method(self):
        return __class__, inside_class


def declares():
    global declared_elsewhere


print(declared_elsewhere)

[comprehension_var for comprehension_var in range(3)]
print(comprehension_var)

try:
    pass
except Exception as handled:
    pass
print(handled)

aiter
ExceptionGroup


def a_clause_iterable_before_its_target():
    return [a for a in range(3) for b in b if b]


hidden_by_a_local_annotation = 1


def a_local_annotation_hides_outer_names():
    hidden_by_a_local_annotation: int
    print: int
    return hidden_by_a_local_annotation, print


def initialized_by_a_nested_nonlocal():
    value: int

    def set_value(v):
        nonlocal value
        value = v

    set_value(1)
    return value


def read_from_a_nested_scope():
    later: int

    def inner():
        return later

    return inner
