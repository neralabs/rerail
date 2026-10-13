import functools
class C:
    @(staticmethod)
    @functools.cache
    def f(x):
        return x
    @(classmethod)
    @functools.cache
    def g(cls):
        return cls
