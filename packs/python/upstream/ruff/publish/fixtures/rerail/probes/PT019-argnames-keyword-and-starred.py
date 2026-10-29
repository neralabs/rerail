import pytest


@pytest.mark.parametrize("_x", argnames="_a")
def test_a(_a, _x):
    pass


@pytest.mark.parametrize(argvalues=[1], argnames="_a")
def test_b(_a):
    pass


@pytest.mark.parametrize(*["_a"], [1])
def test_c(_a):
    pass


@pytest.mark.parametrize(*x, "_a")
def test_d(_a):
    pass


@pytest.mark.parametrize(**{"argnames": "_a"})
def test_e(_a):
    pass


@pytest.mark.parametrize(names, argnames="_a")
def test_f(_a):
    pass


@pytest.mark.parametrize("_a", argnames=names)
def test_g(_a):
    pass
