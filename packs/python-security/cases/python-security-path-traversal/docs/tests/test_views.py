import os

from docs import storage

MEDIA = "/tmp/media"


def test_download(request):
    with open(os.path.join(MEDIA, request.GET["name"])) as fh:
        assert fh.read()


def test_read(request):
    assert storage.read_bundled(request.GET["name"])
