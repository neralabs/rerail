import pytest
from django.conf import settings

# Session-wide configuration before any test runs: the whole run sees one value, nothing leaks between tests.
settings.PASSWORD_HASHERS = ["django.contrib.auth.hashers.MD5PasswordHasher"]


def pytest_configure(config):
    settings.CELERY_TASK_ALWAYS_EAGER = True


@pytest.fixture(scope="function")
def enable_sentry():
    settings.SENTRY_RATE = 0  # caught
    settings.SENTRY_DSN = "https://key@sentry.example/1"  # caught


# pytest-django's `settings` fixture parameter: reverted after each test.
@pytest.fixture(autouse=True)
def disable_sentry(settings):
    settings.SENTRY_RATE = 0
    settings.SENTRY_DSN = None


# A yield fixture that writes the old value back after the yield.
@pytest.fixture
def feature_on():
    old = settings.FEATURE_FLAGS
    settings.FEATURE_FLAGS = {"beta": True}
    yield
    settings.FEATURE_FLAGS = old


# The teardown restores another setting than the one changed.
@pytest.fixture
def media_root(tmp_path):
    old = settings.STATIC_ROOT
    settings.MEDIA_ROOT = str(tmp_path)  # caught
    yield
    settings.STATIC_ROOT = old


# Restored in a finalizer the fixture registers.
@pytest.fixture
def eager(request):
    old = settings.CELERY_ALWAYS_EAGER
    settings.CELERY_ALWAYS_EAGER = True
    request.addfinalizer(lambda: setattr(settings, "CELERY_ALWAYS_EAGER", old))


def pytest_sessionstart(session):
    from tests.helpers import prefix_indexes, use_dummy_cache

    prefix_indexes(session.config)
    use_dummy_cache()


# pytest-django's database set-up, overridden: runs once for the session, before any database test.
@pytest.fixture(scope="session")
def django_db_setup():
    settings.DATABASES = {"default": {"ENGINE": "django.db.backends.postgresql"}}


# An autouse session fixture: every test sees the same value.
@pytest.fixture(scope="session", autouse=True)
def fast_hashers():
    settings.PASSWORD_HASHERS = ["django.contrib.auth.hashers.MD5PasswordHasher"]


# A session fixture only some tests request: the tests that run before the first of them see another value.
@pytest.fixture(scope="session")
def eager_tasks():
    settings.CELERY_TASK_ALWAYS_EAGER = True  # caught
