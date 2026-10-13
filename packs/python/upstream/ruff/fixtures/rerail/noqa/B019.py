import functools
class C:
    @functools.lru_cache
    def f(self):
        pass

import functools
class C:
    @functools.lru_cache  # noqa
    def f(self):
        pass

import functools
class C:
    @functools.lru_cache  # noqa: B019
    def f(self):
        pass

import functools
class C:
    @functools.lru_cache  # NOQA: B019
    def f(self):
        pass

import functools
class C:
    @functools.lru_cache  # noqa: E501
    def f(self):
        pass
