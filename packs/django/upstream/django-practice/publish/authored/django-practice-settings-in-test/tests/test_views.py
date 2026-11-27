import django.conf
import pytest
from django.conf import settings
from django.conf import settings as dj_settings
from django.test import modify_settings, override_settings


def test_assign():
    settings.DEBUG = True  # caught


def test_augmented():
    settings.INSTALLED_APPS += ["shop.extra"]  # caught


def test_delete():
    del settings.FEATURE_FLAG  # caught


def test_item():
    settings.CACHES["default"] = {"BACKEND": "django.core.cache.backends.dummy.DummyCache"}  # caught


def test_nested_item():
    settings.CACHES["default"]["LOCATION"] = "other"  # caught


def test_mutator():
    settings.MIDDLEWARE.append("shop.middleware.Timing")  # caught


def test_setattr():
    setattr(settings, "S3_TIMEOUT", 1)  # caught


def test_delattr():
    delattr(settings, "S3_TIMEOUT")  # caught


def test_alias():
    dj_settings.MAINTENANCE = True  # caught
    django.conf.settings.SITE_ID = 2  # caught


# Restored by hand at the end: a failing assert skips the restore. Only the first write is reported.
def test_restore_by_hand():
    old = settings.DEDUPE_FIELDS
    settings.DEDUPE_FIELDS = ["host"]  # caught
    assert old
    settings.DEDUPE_FIELDS = old


# Restored in a finally: left alone.
def test_finally():
    old = settings.DEDUPE_FIELDS
    settings.DEDUPE_FIELDS = ["host"]
    try:
        assert old
    finally:
        settings.DEDUPE_FIELDS = old


# The finally restores another setting.
def test_finally_other():
    old = settings.DEDUPE_FIELDS
    settings.DEDUPE_ALGO = "hash"  # caught
    try:
        assert old
    finally:
        settings.DEDUPE_FIELDS = old


# pytest-django's settings fixture: the documented way.
def test_fixture(settings):
    settings.DEBUG = True
    settings.CACHES["default"] = {}
    del settings.FEATURE_FLAG


# override_settings, modify_settings and the settings fixture are the fix.
@override_settings(DEBUG=True)
def test_override_decorator():
    assert settings.DEBUG


def test_override_block():
    with override_settings(DEBUG=True):
        assert settings.DEBUG
    with modify_settings(MIDDLEWARE={"append": "shop.middleware.Timing"}):
        assert settings.MIDDLEWARE


# A plain write inside an override is made on the override's copy and dropped with it.
def test_write_inside_override():
    with override_settings(DEBUG=True):
        settings.SITE_ID = 3
    with override_settings(CACHES={"default": {}}):
        settings.CACHES["default"]["LOCATION"] = "x"


@override_settings(SITE_ID=1)
def test_write_inside_override_decorator():
    settings.SITE_ID = 3


# An item write inside an override that does not replace that setting changes the shared value.
def test_item_inside_override():
    with override_settings(DEBUG=True):
        settings.CACHES["default"]["LOCATION"] = "x"  # caught


# Reads, lower-case attributes and calls are not writes of a setting.
def test_reads():
    timeout = settings.S3_TIMEOUT
    flags = settings.CACHES["default"].get("LOCATION")
    settings._wrapped = None
    assert timeout, flags


def test_dynamic_name(key):
    setattr(settings, key, 1)


# Module-level code of a test module runs once at import, before the first test: every test sees one value.
settings.ALLOWED_HOSTS = ["testserver"]


# A helper of the test module that a test calls: its write leaks out of that test.
def enable_beta():
    settings.BETA = True  # caught


def test_beta():
    enable_beta()


# A helper no test calls: not run inside a test.
def unused_helper():
    settings.UNUSED = True


# An override started by hand: the write lands on its copy, and disable() drops it.
def test_override_enable():
    override = override_settings(SITE_ID=1)
    override.enable()
    try:
        settings.SITE_ID = 3
    finally:
        override.disable()


# A function-scoped fixture whose teardown puts the setting back after the test that requests it.
@pytest.fixture
def restore_language():
    old = settings.LANGUAGE_CODE
    yield
    settings.LANGUAGE_CODE = old


def test_restored_by_fixture(restore_language):
    settings.LANGUAGE_CODE = "de"


# Reported: the requested fixture restores another setting.
def test_fixture_restores_other(restore_language):
    settings.TIME_ZONE = "Europe/Berlin"  # caught


@pytest.fixture
def no_store(settings):
    settings.STORE = None
