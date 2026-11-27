from django.db.models import F


def copy_prices(apps, schema_editor):
    Product = apps.get_model("shop.Product")
    Product.objects.filter(price_cents=None).update(price_cents=F("price") * 100)
