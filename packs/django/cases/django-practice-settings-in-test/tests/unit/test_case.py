from unittest import TestCase

from django.conf import settings

from shop.conf import settings as app_settings
from shop.testutils.base import NewsCase


class RestoredInTearDown(TestCase):
    def setUp(self):
        self.old = settings.NEWS_URL
        settings.NEWS_URL = "http://localhost"

    def tearDown(self):
        settings.NEWS_URL = self.old

    def test_url(self):
        settings.NEWS_URL = ""
        assert not settings.NEWS_URL


class NotRestored(TestCase):
    def setUp(self):
        settings.USE_TZ = True  # caught

    def tearDown(self):
        settings.TIME_ZONE = "UTC"

    def test_timeout(self):
        settings.NEWS_URL = "http://10.255.255.1"  # caught
        settings.NEWS_URL = None


class ClassLevel(TestCase):
    @classmethod
    def setUpClass(cls):
        super().setUpClass()
        settings.LANGUAGE_CODE = "de"

    @classmethod
    def tearDownClass(cls):
        settings.LANGUAGE_CODE = "en"
        super().tearDownClass()


class Cleanups(TestCase):
    def test_add_cleanup(self):
        self.addCleanup(setattr, settings, "SITE_ID", settings.SITE_ID)
        settings.SITE_ID = 9

    def test_cleanup_lambda(self):
        old = settings.SITE_ID
        self.addCleanup(lambda: setattr(settings, "SITE_ID", old))
        settings.SITE_ID = 9

    def test_self_settings(self):
        with self.settings(SITE_ID=9):
            assert settings.SITE_ID == 9


# Another library's settings object named `settings`.
def test_dynaconf():
    app_settings.DEBUG = True


# The base class's tearDown restores the setting.
class ChildOfBase(NewsCase):
    def test_news(self):
        settings.NEWS_URL = "http://localhost"

    def test_other_setting(self):
        settings.NEWS_TIMEOUT = 1  # caught


# A cleanup registered as a bound method.
class BoundMethodCleanup(TestCase):
    def setUp(self):
        self.old = settings.SITE_ID
        settings.SITE_ID = 7
        self.addCleanup(self._restore_site)

    def _restore_site(self):
        settings.SITE_ID = self.old
