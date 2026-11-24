from unittest.mock import patch


# Not a test document: a `test_` function outside test code is left alone.
@patch("shop.tasks.cache")
@patch("shop.tasks.statsd")
def test_connection(mock_cache, mock_statsd):
    return mock_cache, mock_statsd
