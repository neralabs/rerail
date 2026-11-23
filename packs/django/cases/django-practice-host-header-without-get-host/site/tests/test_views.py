def test_host(rf):
    request = rf.get("/")
    assert request.META["HTTP_HOST"] == "testserver"
