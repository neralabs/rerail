from django.http import JsonResponse

from shop.models import Customer, SiteConfig


def status(request, pk):
    # The project keeps one SiteConfig row: get() without arguments fails on two.
    config = SiteConfig.objects.get()
    customer = Customer.objects.get(pk=pk)
    return JsonResponse({"site": config.name, "customer": customer.name})
