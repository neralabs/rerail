from django.conf import settings


# Called only from pytest_configure: session-wide set-up.
def prefix_indexes(config):
    for key, index in settings.ES_INDEXES.items():
        settings.ES_INDEXES[key] = f"test_{index}"


# Called from pytest_configure and from a test: the test's call leaks.
def use_dummy_cache():
    settings.CACHES["default"] = {"BACKEND": "django.core.cache.backends.dummy.DummyCache"}  # caught


def test_with_dummy_cache():
    use_dummy_cache()
