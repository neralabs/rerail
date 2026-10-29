from typing import override


class A:
    @override
    def good(self, X, Y=1, *Z, **W): ...

    @override
    @staticmethod
    def static(X): ...

    @staticmethod
    @override
    def static2(X): ...

    def bad(self, X): ...

    if True:
        @override
        def cond(self, X): ...

    def method(self):
        @override
        def nested(X): ...

        class Inner:
            @override
            def inner(self, X): ...

            def innerbad(self, X): ...

        return nested, Inner


@override
def module_level(X): ...


def func():
    @override
    def in_func(X): ...

    class K:
        @override
        def in_class_in_func(self, X): ...

        def bad(self, X): ...
