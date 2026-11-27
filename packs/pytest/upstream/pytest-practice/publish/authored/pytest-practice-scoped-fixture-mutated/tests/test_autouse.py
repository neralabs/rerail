import pytest


@pytest.fixture(autouse=True)
def _module_defaults(app):
    app.config["DEFAULT_URL"] = "/lab"


@pytest.fixture(autouse=True)
def _count_runs(registry):
    registry.append("run")  # caught


def test_defaults(app):
    assert app.config["DEFAULT_URL"] == "/lab"
