# No Django import here, but Book is a model class of the run: its queryset is Django's.
from inventory.models import Item
from shop.models import Book


def in_stock(title):
    return Book.objects.filter(title=title).count() > 0  # caught


def item_known(sku):
    # Item is a model of the run in a src layout (src/inventory/models.py is the module inventory.models).
    return Item.objects.filter(sku=sku).count() > 0  # caught
