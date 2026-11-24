from django.http import JsonResponse

from shop.models import SiteSettings


def site_info(request):
    settings = SiteSettings.objects.first()
    return JsonResponse({"currency": settings.currency})
