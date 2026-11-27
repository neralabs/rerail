from unittest import TestCase, mock

from shop import tasks
from shop.cache import cache


class TestCeleryTasks(TestCase):
    # As in addons-server's test_celery: the cache-miss set-up goes to the statsd mock.
    @mock.patch("shop.tasks.cache")
    @mock.patch("shop.tasks.statsd")
    def test_cache_miss(self, celery_cache, celery_statsd):  # caught
        celery_cache.get.return_value = None
        tasks.run()
        assert celery_statsd.incr.call_count == 1

    @mock.patch("shop.tasks.cache")
    @mock.patch("shop.tasks.statsd")
    def test_cache_miss_ok(self, celery_statsd, celery_cache):
        celery_cache.get.return_value = None
        tasks.run()

    # patch.object, leading underscores stripped from the attribute; a fixture-like extra parameter.
    @mock.patch.object(cache, "_get", autospec=True)
    @mock.patch.object(tasks, "statsd")
    def test_object(self, mock_get, mock_statsd, extra=None):  # caught
        tasks.run()

    @classmethod
    @mock.patch("shop.tasks.cache")
    @mock.patch("shop.tasks.statsd")
    def test_classmethod(cls, mock_cache, mock_statsd):  # caught
        tasks.run()


# Patches on the class reach every test method after the method's own mocks: not read (a gap).
@mock.patch("shop.tasks.cache")
@mock.patch("shop.tasks.statsd")
class TestPatchedClass(TestCase):
    def test_run(self, mock_cache, mock_statsd):
        tasks.run()
