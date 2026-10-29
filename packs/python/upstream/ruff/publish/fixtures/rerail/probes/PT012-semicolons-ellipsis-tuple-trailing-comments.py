import pytest


def test_semicolons_and_ellipsis_tuples():
    with pytest.raises(E):
        a();

    with pytest.raises(E): a();

    with pytest.raises(E):
        for x in y: ...;

    with pytest.raises(E):
        for x in y:
            ..., 1

    with pytest.raises(E):
        for x in y:
            ...,

    with pytest.raises(E):
        with ctx(): a(); b()


def test_trailing_comments():
    with pytest.raises(E):
        a()
        b()  # don't
        # trailing at body indent
            # deeper

    with pytest.raises(E):
        a()
        b("x # y")  # it's

    with pytest.raises(E):
        a()
        x = """
y""" + z("#")  # z's

    with pytest.raises(E):
        a()
        if x:
            b()
            # inner trailing
        # outer trailing
    c()
