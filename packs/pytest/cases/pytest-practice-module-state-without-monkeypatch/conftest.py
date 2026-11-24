import pytest

from shop import registry

# Module-level code runs once at import, before the first test: every test sees the same value.
registry.TIMEOUT = 10


# A pytest hook: session-wide set-up, not a test.
def pytest_configure(config):
    registry.PLUGINS.append("session")


# An autouse session fixture: every test of the run sees the same value.
@pytest.fixture(scope="session", autouse=True)
def session_flow():
    registry.DEFAULT_FLOW = ("cart",)


@pytest.fixture(scope="module")
def module_handlers():
    registry.HANDLERS["m"] = print  # caught
    return registry.HANDLERS


@pytest.fixture
def fast_timeout():
    registry.TIMEOUT = 0  # caught
    return registry


# Undone after the yield.
@pytest.fixture
def extra_plugin():
    registry.PLUGINS.append("p")
    yield
    registry.PLUGINS.remove("p")


# Undone in a finally around the yield.
@pytest.fixture
def short_flow():
    old = registry.DEFAULT_FLOW
    registry.DEFAULT_FLOW = ("payment",)
    try:
        yield
    finally:
        registry.DEFAULT_FLOW = old


# Undone by a finalizer.
@pytest.fixture
def handler(request):
    registry.HANDLERS["h"] = print
    request.addfinalizer(lambda: registry.HANDLERS.pop("h"))


# A session fixture an autouse fixture requests: it runs before the first test, so every test sees the same value.
@pytest.fixture(scope="session")
def app_overrides():
    registry.HANDLERS["db"] = print


@pytest.fixture(autouse=True)
def clean_cookies(app_overrides):
    return app_overrides


# A session fixture only some tests request: the tests before the first of them see the old value.
@pytest.fixture(scope="session")
def slow_timeout():
    registry.TIMEOUT = 60  # caught
