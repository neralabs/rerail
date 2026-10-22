import pytest

with pytest.warns(UserWarning):
    foo()
    bar()

import pytest

with pytest.warns(UserWarning):  # noqa
    foo()
    bar()

import pytest

with pytest.warns(UserWarning):  # noqa: PT031
    foo()
    bar()

import pytest

with pytest.warns(UserWarning):  # NOQA: PT031
    foo()
    bar()

import pytest

with pytest.warns(UserWarning):  # noqa: E501
    foo()
    bar()
