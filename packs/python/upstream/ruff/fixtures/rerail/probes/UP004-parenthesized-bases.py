import builtins
import builtins as b


class A((object)):
    ...


class B(  # comment
    (  # inner
        builtins.object
    ),
    metaclass=object,
):
    ...


class C(*object):
    ...


class D(b.object, \
        object):
    ...


def f(object):
    class E(object):
        ...


class F(object()):
    ...
