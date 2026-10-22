import pytest
pytest.raises(ZeroDivisionError, int, "x")

import pytest
pytest.raises(ZeroDivisionError, int, "x")  # noqa

import pytest
pytest.raises(ZeroDivisionError, int, "x")  # noqa: RUF061

import pytest
pytest.raises(ZeroDivisionError, int, "x")  # NOQA: RUF061

import pytest
pytest.raises(ZeroDivisionError, int, "x")  # noqa: E501
