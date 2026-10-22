import functools


@functools.lru_cache()
def f():
    pass

import functools


@functools.lru_cache()  # noqa
def f():
    pass

import functools


@functools.lru_cache()  # noqa: UP011
def f():
    pass

import functools


@functools.lru_cache()  # NOQA: UP011
def f():
    pass

import functools


@functools.lru_cache()  # noqa: E501
def f():
    pass
