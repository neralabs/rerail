from svc.services.http import fetch


async def test_fetch():
    assert fetch("http://example.com") is not None
