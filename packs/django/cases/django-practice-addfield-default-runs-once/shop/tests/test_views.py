from shop.models import Order


def test_one_order(db):
    # A test's fixture holds one row; that says nothing about production tables.
    assert Order.objects.get().token
