from importlib import import_module

from django.apps import apps


def load():
    import_module("shop.loaded_by_string")
    for app in apps.get_app_configs():
        import_module(f"{app.name}.hooks")
