import requests


def test_live_endpoint():
    s = requests.Session()
    assert s.get("http://localhost:8000/health").ok
