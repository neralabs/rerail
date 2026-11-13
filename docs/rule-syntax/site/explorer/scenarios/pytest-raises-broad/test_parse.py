import pytest


def parse(text):
    return int(text)


def test_bad_number():
    with pytest.raises(ValueError):
        parse("x")


def test_bad_number_ok():
    with pytest.raises(ValueError, match="invalid literal"):
        parse("x")


def test_type():
    with pytest.raises(TypeError):
        parse(None)
