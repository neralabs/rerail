import mock

from shop import tasks


# The third-party `mock` package passes its mocks the same way.
@mock.patch("shop.tasks.cache")
@mock.patch("shop.tasks.statsd")
def test_legacy_mock(mock_cache, mock_statsd):  # caught
    tasks.run()


# A `patch` of an unrelated library is not unittest.mock's.
from shop.fakes import patch


@patch("shop.tasks.cache")
@patch("shop.tasks.statsd")
def test_other_patch(mock_cache, mock_statsd):
    tasks.run()
