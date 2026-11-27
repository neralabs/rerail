from django.contrib.sites.models import Site
from django.http import JsonResponse

from shop.models import Customer, SiteConfig


def status(request, pk):
    # The project keeps one SiteConfig row: get() without arguments fails on two.
    config = SiteConfig.objects.get()
    customer = Customer.objects.get(pk=pk)
    return JsonResponse({"site": config.name, "customer": customer.name})


def telemetry(request):
    # The current site's settings: one row per site.
    store = Site.objects.get_current().store_settings
    # The current user's profile: one row per user, many rows in the table.
    profile = request.user.profile
    return JsonResponse({"instance": str(store.instance_id), "theme": profile.theme})
