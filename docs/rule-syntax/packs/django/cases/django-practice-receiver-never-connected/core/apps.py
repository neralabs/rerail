from importlib import import_module

from django.apps import AppConfig


class ManagedAppConfig(AppConfig):
    def ready(self):
        for rel in ["webhooks"]:
            import_module(f"{self.name}{'.'}{rel}")
