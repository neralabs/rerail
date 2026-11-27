import pytest

from shop.app import create_app


@pytest.fixture(scope="module")
def app():
    return create_app()


@pytest.fixture(scope="session")
def registry():
    return []


@pytest.fixture(scope="session")
def settings_dict():
    return {"a": 1, "b": 2}


@pytest.fixture(scope="session", name="api_client")
def _api_client():
    return create_app()


@pytest.fixture
def fresh():
    return {}


@pytest.fixture(scope="session")
def make_user():
    def make(name):
        return {"name": name}

    return make


@pytest.fixture(scope="module")
def configured_app(app):
    app.config["SET_UP_ONCE"] = True
    return app


@pytest.fixture
def flagged(app):
    app.config["FLAG"] = True  # caught
    return app


@pytest.fixture
def flagged_and_restored(app):
    old = app.config.get("FLAG")
    app.config["FLAG"] = True
    yield app
    app.config["FLAG"] = old


@pytest.fixture
def scratch(registry):
    registry.append("scratch")  # caught
    yield registry


@pytest.fixture(scope="session")
def db_session():
    return create_app()


@pytest.fixture
def registered(registry):
    registry.append("temp")
    yield registry
    registry.remove("temp")
