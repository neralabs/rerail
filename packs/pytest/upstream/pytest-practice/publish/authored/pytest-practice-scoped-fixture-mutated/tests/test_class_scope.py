import pytest


@pytest.fixture(scope="module")
def module_rows():
    return []


@pytest.fixture(scope="module")
def module_seeded(module_rows):
    module_rows.append("seed")
    return module_rows


class TestShared:
    @pytest.fixture(scope="class")
    def shared(self):
        return {}

    def test_first(self, shared):
        shared["k"] = 1  # caught

    def test_second(self, shared, module_rows):
        module_rows.extend([1, 2])  # caught
        assert shared


def test_cleanup_callback(module_rows, request):
    module_rows.append("x")
    request.addfinalizer(lambda: module_rows.remove("x"))


@pytest.fixture(scope="module")
def released():
    refs = []
    yield refs
    assert all(ref is None for ref in refs)


def test_collects(released):
    released.append(None)
