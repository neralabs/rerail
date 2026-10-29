import pytest

with pytest.raises(ValueError, match="x"):  
    a = 1
    raise ValueError(a)

import pytest

with pytest.raises(ValueError, match="x"):    # noqa
    a = 1
    raise ValueError(a)

import pytest

with pytest.raises(ValueError, match="x"):    # noqa: PT012
    a = 1
    raise ValueError(a)

import pytest

with pytest.raises(ValueError, match="x"):    # NOQA: PT012
    a = 1
    raise ValueError(a)

import pytest

with pytest.raises(ValueError, match="x"):    # noqa: E501
    a = 1
    raise ValueError(a)
