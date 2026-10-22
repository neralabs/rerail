import pytest

with pytest.raises(ValueError):  
    raise ValueError

import pytest

with pytest.raises(ValueError):    # noqa
    raise ValueError

import pytest

with pytest.raises(ValueError):    # noqa: PT011
    raise ValueError

import pytest

with pytest.raises(ValueError):    # NOQA: PT011
    raise ValueError

import pytest

with pytest.raises(ValueError):    # noqa: E501
    raise ValueError
