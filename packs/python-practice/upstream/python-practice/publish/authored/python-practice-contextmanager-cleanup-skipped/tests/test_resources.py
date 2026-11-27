from contextlib import contextmanager


@contextmanager
def scratch(path):
    open(path, "w").close()
    yield path
    path.unlink()


def test_scratch(tmp_path):
    with scratch(tmp_path / "x") as p:
        assert p
