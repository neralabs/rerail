import importlib
import sys

from shop import factory


# The fixture swaps the decorator, then reloads shop.views so its from-import binds the replacement.
def test_home(monkeypatch):
    monkeypatch.setattr(factory, "login_required", lambda view: view)
    sys.modules.pop("shop.views", None)
    views = importlib.import_module("shop.views")
    assert views.home() == "home"
