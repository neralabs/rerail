import pytest


@pytest.mark.usefixtures(f"{"#"}", f'{'#'}')  # trailing
@pytest.fixture
def a(): ...


@pytest.mark.usefixtures(f"{{#}}")  # trailing
@pytest.fixture
def b(): ...


@pytest.mark.usefixtures(rf"{"a"}#", F'{x!r:>{w}}')  # c
@pytest.fixture
def c(): ...
