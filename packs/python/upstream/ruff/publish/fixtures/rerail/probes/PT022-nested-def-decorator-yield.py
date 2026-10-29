import pytest


def deco(x):
    return lambda f: f


@pytest.fixture
def nested_def_decorator_yield_skipped():
    @deco((yield))
    def inner():
        pass
    yield inner


@pytest.fixture
def nested_class_decorator_yield_counted():
    @deco((yield))
    class C:
        pass
    yield C
