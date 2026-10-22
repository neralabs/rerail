import pytest


@pytest.mark.parametrize(argvalues=[1], argnames="a,b")
def t1(a): ...


@pytest.mark.parametrize("a,b", argvalues=[1])
def t2(a): ...


@pytest.mark.parametrize(argnames="a,b")
def t3(a): ...


@pytest.mark.parametrize("a,b")
def t4(a): ...


@pytest.mark.parametrize("a,b", *rows)
def t5(a): ...


@pytest.mark.parametrize(*args)
def t6(a): ...


@pytest.mark.parametrize(*args, "a,b", [1])
def t7(a): ...


@pytest.mark.parametrize("a,b", [1], *rest)
def t8(a): ...


@pytest.mark.parametrize("a,b", **kw)
def t9(a): ...


@pytest.mark.parametrize(**kw)
def t10(a): ...


@pytest.mark.parametrize("a,b", argnames="x,y", argvalues=[1])
def t11(a): ...


@pytest.mark.parametrize(["a", "b"], argvalues=[1], ids=["x"])
def t12(a): ...


@pytest.mark.parametrize(
    argvalues=[1],
    argnames=("a", ),
    ids=["x"],
)
def t13(a): ...


@pytest.mark.parametrize("a,b", [1], **kw)
def t14(a): ...


@pytest.mark.parametrize("a,b", argnames=["x"], argvalues=[1])
def t15(a): ...


@pytest.mark.parametrize(*a, argnames="a,b", argvalues=[1])
def t16(a): ...


@pytest.mark.parametrize("a,b", [1], "extra")
def t17(a): ...


@pytest.mark.parametrize("a,b", values=[1])
def t18(a): ...
