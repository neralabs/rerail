from shop.tasks import notify_order


def fake_view(request):
    notify_order.delay(1)
