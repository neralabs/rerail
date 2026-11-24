import os

import pytest

# Session-wide configuration before any test runs: every test sees the same value, nothing leaks between tests.
os.environ["DJANGO_SETTINGS_MODULE"] = "shop.settings.test"
os.environ.setdefault("TZ", "UTC")


def pytest_configure(config):
    os.environ["SHOP_ENV"] = "test"


def pytest_sessionstart(session):
    from tests.helpers import load_test_env

    load_test_env()


@pytest.fixture
def insecure_transport():
    os.environ["AUTHLIB_INSECURE_TRANSPORT"] = "true"  # caught


@pytest.fixture(scope="session")
def api_keys():
    os.environ.update({"STRIPE_KEY": "sk_test", "MAIL_KEY": "mk_test"})  # caught
    return "sk_test"


# The fix: monkeypatch puts the old value back after the test.
@pytest.fixture
def debug_on(monkeypatch):
    monkeypatch.setenv("DEBUG", "1")
    monkeypatch.delenv("PROXY", raising=False)


# A yield fixture that puts the old value back after the yield.
@pytest.fixture
def region():
    old = os.environ.get("AWS_REGION")
    os.environ["AWS_REGION"] = "eu-west-1"
    yield
    if old is None:
        del os.environ["AWS_REGION"]
    else:
        os.environ["AWS_REGION"] = old


# The teardown restores another variable than the one changed.
@pytest.fixture
def media_root(tmp_path):
    old = os.environ.get("STATIC_ROOT")
    os.environ["MEDIA_ROOT"] = str(tmp_path)  # caught
    yield
    os.environ["STATIC_ROOT"] = old


# The whole environment saved and put back.
@pytest.fixture
def clean_env():
    saved = dict(os.environ)
    os.environ.clear()
    os.environ["PATH"] = "/usr/bin"
    yield
    os.environ.clear()
    os.environ.update(saved)


# Restored by a finalizer the fixture registers.
@pytest.fixture
def eager(request):
    os.environ["CELERY_EAGER"] = "1"
    request.addfinalizer(lambda: os.environ.pop("CELERY_EAGER", None))


# patch.dict restores the whole mapping when the fixture's test ends.
@pytest.fixture
def no_proxy(mocker):
    mocker.patch.dict(os.environ, {"NO_PROXY": "*"})
    os.environ["HTTP_PROXY"] = ""


# A session-scoped autouse fixture runs before the first test: every test sees the value.
@pytest.fixture(autouse=True, scope="session")
def feature_flags():
    os.environ["FFLAG_NEW_CHECKOUT"] = "true"


# A session-scoped fixture that only some tests request: the tests before the first of them run without it.
@pytest.fixture(scope="session")
def azure_credentials():
    os.environ["AZURE_ACCOUNT_KEY"] = "testing"  # caught
