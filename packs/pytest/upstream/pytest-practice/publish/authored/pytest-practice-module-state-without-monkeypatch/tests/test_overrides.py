import pytest

from shop import main
from shop.main import _ENGINE_CACHE, TASKS, FxaView, app, get_db, settings


def fake_db():
    return None


# FastAPI's dependency overrides, cleared after the yield: a mutator on any object counts as the undo.
@pytest.fixture
def client_with_db():
    app.dependency_overrides[get_db] = fake_db
    yield app
    app.dependency_overrides.clear()


# Popped in a finally.
@pytest.fixture
def client_popped():
    app.dependency_overrides[get_db] = fake_db
    try:
        yield app
    finally:
        app.dependency_overrides.pop(get_db, None)


# Replaced as a whole after the yield.
@pytest.fixture
def client_reset():
    app.dependency_overrides[get_db] = fake_db
    yield app
    app.dependency_overrides = {}


# Set in a test and never cleared.
def test_override_left_behind():
    app.dependency_overrides[get_db] = fake_db  # caught


# A settings instance of the app's own module.
def test_settings_instance():
    settings.debug = True  # caught


# A cache emptied before a cold-cache assertion: a reset, not a leak.
def test_cold_cache():
    _ENGINE_CACHE.clear()
    assert main.get_db() is None


# Cleanup of an entry other code registered: pop with a default as the only change.
def test_cleanup_registration():
    TASKS.pop("tmp", None)


# `del` as the only change of the target: a cleanup.
def test_delete_cached_keys():
    del FxaView.keys


# A cache cleared and then filled: the clear is a reset, the write leaks.
def test_clear_then_fill():
    _ENGINE_CACHE.clear()
    _ENGINE_CACHE["sqlite"] = object()  # caught


# An annotation without a value is no write.
def test_annotation_only():
    main.TASKS["x"]: int
