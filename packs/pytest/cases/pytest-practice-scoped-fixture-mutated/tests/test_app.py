from unittest.mock import MagicMock

import pytest

from shop.app import create_app


def test_item_store(app):
    app.config["RETRY_COUNT"] = 4  # caught
    assert app.config["RETRY_COUNT"] == 4


def test_attribute_store(app):
    app.appbuilder = MagicMock()  # caught


def test_delete_item(settings_dict):
    del settings_dict["a"]  # caught


def test_augmented(app):
    app.counter += 1  # caught


def test_append(registry):
    registry.append(1)  # caught


def test_update_chain(app):
    app.config.update({"A": 1})  # caught


def test_attribute_of_item(app):
    app.extensions["mail"].enabled = False  # caught


def test_named_fixture(api_client):
    api_client.debug = True  # caught


def test_nested_helper(app):
    def hook():
        app.counter = 10  # caught

    hook()


class TestMethods:
    def test_in_method(self, app):
        app.debug = True  # caught


@pytest.mark.parametrize("app", [{"FEATURE": True}], indirect=True)
def test_indirect_param_still_shared(app):
    app.config["OTHER"] = 1  # caught


# --- left alone ---


def test_http_delete_is_no_mutation(api_client, registry):
    api_client.delete("/items/1")
    registry.index(1)


def test_read_only(app):
    assert app.config.get("RETRY_COUNT") is None
    assert not app.debug


def test_restored_in_finally(app):
    old = app.config.get("X")
    app.config["X"] = 1
    try:
        assert app.config["X"] == 1
    finally:
        app.config["X"] = old


def test_restored_chain_in_finally(app):
    saved = dict(app.config)
    app.config["Y"] = 1
    try:
        assert app.config["Y"]
    finally:
        app.config.clear()
        app.config.update(saved)


def test_monkeypatch(app, monkeypatch):
    monkeypatch.setitem(app.config, "X", 1)
    monkeypatch.setattr(app, "debug", True)


def test_local_copy(app):
    cfg = dict(app.config)
    cfg["X"] = 1


def test_function_scoped(fresh):
    fresh["a"] = 1
    fresh.update(b=2)


def test_factory(make_user):
    user = make_user("ann")
    user["admin"] = True


def test_unresolved_plugin_fixture(plugin_thing):
    plugin_thing["a"] = 1


def test_rebound(app):
    app = create_app()
    app.config["X"] = 1


@pytest.mark.parametrize("registry", [[1], [2]])
def test_parametrized_name(registry):
    registry.append(3)


def test_restored_fixture(flagged_and_restored, configured_app):
    assert flagged_and_restored.config["FLAG"]


def test_database_rows(app, db_session):
    app.db.add({"row": 1})
    db_session.add({"row": 2})
    app.db.session.remove()


def test_restored_by_finalizer(request, app):
    saved = app.debug
    request.addfinalizer(lambda: setattr(app, "debug", saved))
    app.debug = True


# A restore by hand without finally: a failing assert skips it. Reported at the first change.
def test_restored_by_hand(app):
    old = app.counter
    app.counter = 5  # caught
    assert app.counter == 5
    app.counter = old


# Two plain stores of the same target: the second is no restore. Reported once, at the first.
def test_two_stores(app):
    app.config["MODE"] = "a"  # caught
    app.config["MODE"] = "b"


def test_repository_update(api_client):
    api_client.repos.recipes.update("slug", {"name": "soup"})


def test_sdk_client_calls(api_client, registry):
    api_client.agents.update(agent_id="a1", tags=[])
    registry.add(name="tool")
    agent = registry.update({"a": 1})
    assert agent


def test_defensive_pop(app):
    app.config.pop("DEV_USERNAME", None)


def test_monkeypatched_then_deleted(app, monkeypatch):
    monkeypatch.setitem(app.extensions, "namespace", None)
    del app.extensions["namespace"]
    monkeypatch.setattr(app, "debug", False)
    app.debug = True


def test_patch_dict(app, mocker):
    mocker.patch.dict(app.config, {"A": 1})
    app.config["B"] = 2
    app.config.update({"C": 3})


def test_pop_without_default(app):
    app.config.pop("SECRET_KEY")  # caught


# Undone in a finally by a defensive pop of the same key: the key was absent before the test.
def test_popped_in_finally(app):
    app.config["RESOLVER"] = object()
    try:
        assert app.config["RESOLVER"]
    finally:
        app.config.pop("RESOLVER", None)


# A function-scoped fixture that pops what it set after the yield.
@pytest.fixture
def mcp_enabled(app):
    app.config["MCP_ENABLED"] = True
    yield app
    app.config.pop("MCP_ENABLED", None)


def test_uses_mcp(mcp_enabled):
    assert mcp_enabled.config["MCP_ENABLED"]


# The finally pops another key than the one set.
def test_popped_other_key(app):
    app.config["ISSUER"] = "x"  # caught
    try:
        assert app.config["ISSUER"]
    finally:
        app.config.pop("AUDIENCE", None)


# Undone by a helper the finally calls.
def _reset_debug(app):
    app.debug = False


def test_undo_through_helper(app):
    app.debug = True
    try:
        assert app.debug
    finally:
        _reset_debug(app)


# A fixture value fetched by name at run time is the same shared object.
def test_getfixturevalue(request):
    shared = request.getfixturevalue("app")
    shared.config["LATE"] = 1  # caught


# A helper of the test module the test hands the fixture to: the helper's change reaches the shared object.
def _enable_feature(target):
    target.config["FEATURE"] = True  # caught


def test_helper_changes(app):
    _enable_feature(app)


# A function-scoped fixture whose yield expression registers on the shared app: set-up, not its teardown.
def _register(target):
    target.extensions["service"] = object()  # caught
    return target


@pytest.fixture
def registered_app(app):
    yield _register(app)


def test_registered(registered_app):
    assert registered_app.extensions["service"]
