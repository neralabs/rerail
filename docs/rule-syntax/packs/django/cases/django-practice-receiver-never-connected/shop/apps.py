from django.apps import AppConfig


class ShopConfig(AppConfig):
    name = "shop"

    def ready(self):
        from . import signals  # noqa: F401
        import shop.handlers.orders  # noqa: F401
        from shop.handlers import refunds  # noqa: F401
