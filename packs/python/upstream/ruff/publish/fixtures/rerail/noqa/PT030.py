import pytest

with pytest.warns(Warning):  
    pass

import pytest

with pytest.warns(Warning):    # noqa
    pass

import pytest

with pytest.warns(Warning):    # noqa: PT030
    pass

import pytest

with pytest.warns(Warning):    # NOQA: PT030
    pass

import pytest

with pytest.warns(Warning):    # noqa: E501
    pass
