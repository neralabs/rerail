from django.conf import settings


# Test code is left alone: tests should use override_settings, but that is a test-hygiene rule of its own.
def test_debug():
    settings.DEBUG = True


# pytest-django's fixture is the documented way to change settings in a test.
def test_fixture(settings):
    settings.DEBUG = True
