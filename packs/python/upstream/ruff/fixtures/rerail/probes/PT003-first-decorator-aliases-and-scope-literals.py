import pytest
import pytest as pt
from pytest import fixture as fx
from pytest import yield_fixture as yf


@pt.fixture
@pytest.fixture(scope="function")
def first_decorator_is_bare_alias(): ...


@yf
@fx(scope="function")
def first_decorator_is_bare_yield_fixture(): ...


@pytest.fixture(scope="function")
@fx(scope="function")
def first_decorator_is_reported(): ...


@pytest.fixture(scope="function", name="a)#b")  # c
def with_trailing_comment(): ...


@pytest.fixture(scope=("func" "tion"))
def implicit_concatenation(): ...


@pytest.fixture(scope="\x66unction")
def escaped_literal(): ...


@pytest.fixture(scope=b"function")
def bytes_is_not_a_string(): ...


@pytest.fixture(scope=f"function")
def fstring_is_not_a_string(): ...


@pytest.fixture(scope="function" if True else "module")
def conditional(): ...


@pytest.fixture(name="é😀", scope="function")
def non_ascii_before_keyword(): ...


@pytest.fixture(**{"scope": "function"}, name=dict(scope="function"))
def not_a_keyword_argument(): ...


@pytest.fixture(
    "x", *args,
    scope=\
        "function"  # c
    , **kwargs
)
def messy_arguments(): ...
