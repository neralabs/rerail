from shop.models import Order


def check_latest_order(page):
    order = Order.objects.first()
    assert page.text_content("#order") == order.number
