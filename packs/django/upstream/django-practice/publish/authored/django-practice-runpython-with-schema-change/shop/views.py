from django.http import JsonResponse

from shop.models import Order, SiteSettings


def site_settings():
    # The lazy singleton: SiteSettings holds one row.
    settings = SiteSettings.objects.first()
    if settings is None:
        settings = SiteSettings.objects.create(currency="EUR")
    return settings


def site_info(request):
    return JsonResponse({"currency": site_settings().currency})


def latest_order(request):
    # A bare first() reads one row of many: Order is no one-row table.
    return JsonResponse({"id": Order.objects.first().pk})
