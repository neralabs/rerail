import pytest

with pytest.raises():  
    pass

import pytest

with pytest.raises():    # noqa
    pass

import pytest

with pytest.raises():    # noqa: PT010
    pass

import pytest

with pytest.raises():    # NOQA: PT010
    pass

import pytest

with pytest.raises():    # noqa: E501
    pass
