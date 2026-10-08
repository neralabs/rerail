# Name-resolution oracle fixture: every reference to an `undefined_*` or later-bound name.
early_use(later_name)
later_name = 1
early_use = later_name


def uses_module_names():
    return later_import, module_after, undefined_in_function


import later_import
module_after = 2


class Klass:
    class_attr = 1
    copy = class_attr

    def method(self):
        return class_attr, Klass, self

    comprehension = [class_attr for _ in range(2)]
    first_iterable = [item for item in [class_attr]]


def params(pos, /, normal, *args, key=undefined_default, **kwargs):
    return pos, normal, args, key, kwargs, undefined_body


def defaults_run_outside(value=value_default, *, other: undefined_annotation = 1):
    return value, other


lam = lambda alpha, beta=undefined_lambda_default: (alpha, beta, gamma_free)


def closure():
    free = 1

    def inner():
        nonlocal free
        free = 2
        return free

    def reader():
        return free, never_bound

    return inner, reader


def globals_():
    global declared_global
    declared_global = 1
    return declared_global


def reads_declared():
    return declared_global


declared_global_at_module = declared_global


def walrus():
    [(captured := n) for n in range(3)]
    return captured, n


with open_ctx() as (w_one, w_two):
    pass
print_it(w_one, w_two)

try:
    pass
except ValueError as caught:
    print_it(caught)

for f_one, (f_two, *f_rest) in pairs:
    print_it(f_one, f_two, f_rest)

a_one, [a_two, *a_rest] = pairs
annotated: int = 1
annotation_only: int
print_it(a_one, a_two, a_rest, annotated, annotation_only)

counter += 1


class Generic[T](list[T]):
    def get(self) -> T:
        return T


def generic_fn[U, *Us, **Ps](x: U) -> U:
    return U, Us, Ps


type Alias[V] = list[V]
print_it(T, U, V, Alias)

match subject:
    case Point(x=m_one, y=[m_two, *m_rest]) as m_all:
        print_it(m_one, m_two, m_rest, m_all)
    case {"key": m_three, **m_four} | [m_three, m_four]:
        print_it(m_three, m_four)
    case Color.RED:
        pass
    case other_capture:
        print_it(other_capture)

print_it(f"{fstring_name!r:{fstring_spec}}")


@decorator_name
def decorated():
    return decorated


def shadow_in_body():
    print_it(local_later)
    local_later = 1
    return local_later


from package import imported_name as renamed
from . import relative_name
import package.module
import package.other as other_alias
print_it(renamed, relative_name, package, other_alias, imported_name)
