# Tests may import a settings module to test it.
from myproject.settings import DEBUG


def test_debug():
    assert DEBUG is not None
