def test_function_scoped_override(app):
    app.config["X"] = 1
    app.debug = True


def test_session_registry_still_shared(registry):
    registry.clear()  # caught
