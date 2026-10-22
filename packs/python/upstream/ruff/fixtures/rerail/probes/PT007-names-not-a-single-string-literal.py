import pytest

NAMES = "a,b"


@pytest.mark.parametrize(NAMES, [[1, 2]])
def t1(a): ...


@pytest.mark.parametrize(f"a", [[1, 2]])
def t2(a): ...


@pytest.mark.parametrize(b"a", [[1, 2]])
def t3(a): ...


@pytest.mark.parametrize(f"a,b", [[1, 2]])
def t4(a): ...


@pytest.mark.parametrize(get(), [[1, 2]])
def t5(a): ...


@pytest.mark.parametrize([], [[1, 2]])
def t6(a): ...


@pytest.mark.parametrize((), [[1, 2]])
def t7(a): ...


@pytest.mark.parametrize("a" f"b", [[1, 2]])
def t8(a): ...


@pytest.mark.parametrize(r"a", [[1, 2]])
def t9(a): ...


@pytest.mark.parametrize(u"a", [[1, 2]])
def t10(a): ...


@pytest.mark.parametrize("a" if x else "b", [[1, 2]])
def t11(a): ...


@pytest.mark.parametrize(1, [[1, 2]])
def t12(a): ...


@pytest.mark.parametrize(None, [[1, 2]])
def t13(a): ...


@pytest.mark.parametrize(("a" "b"), [[1, 2]])
def t14(a): ...


@pytest.mark.parametrize((("a")), [[1, 2]])
def t15(a): ...


@pytest.mark.parametrize(("a,b"), (()))
def t16(a): ...


@pytest.mark.parametrize("", [[1, 2]])
def t17(a): ...


@pytest.mark.parametrize(",", [[1, 2]])
def t18(a): ...


@pytest.mark.parametrize(" , ", [[1, 2]])
def t19(a): ...
