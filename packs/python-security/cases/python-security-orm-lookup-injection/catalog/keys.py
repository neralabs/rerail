import logging

from django.http import JsonResponse

from catalog.models import Book

ALLOWED = {"title", "year", "title__icontains"}
logger = logging.getLogger(__name__)


def loop_check(request):
    for key in request.GET:
        if key not in ALLOWED:
            raise ValueError(key)
    return JsonResponse({"n": Book.objects.filter(**request.GET.dict()).count()})


def keys_loop(request):
    for key in request.GET.keys():
        if key.split("__")[0] not in ALLOWED:
            return JsonResponse({"error": key}, status=400)
    return JsonResponse({"n": Book.objects.filter(**request.GET.dict()).count()})


def items_loop(request):
    for key, _value in request.query_params.items():
        if key not in ALLOWED:
            raise ValueError(key)
    return JsonResponse({"n": Book.objects.filter(**request.query_params).count()})


def set_difference(request):
    if set(request.GET) - ALLOWED:
        raise ValueError("unknown filter")
    return JsonResponse({"n": Book.objects.filter(**request.GET.dict()).count()})


def set_subset(request):
    if not set(request.GET.keys()) <= ALLOWED:
        raise ValueError("unknown filter")
    return JsonResponse({"n": Book.objects.filter(**request.GET.dict()).count()})


def issubset(request):
    if not set(request.GET).issubset(ALLOWED):
        raise ValueError("unknown filter")
    return JsonResponse({"n": Book.objects.filter(**request.GET.dict()).count()})


# Loops that do not stop an unknown key, or check another mapping.
def loop_logs_only(request):
    for key in request.GET:
        if key not in ALLOWED:
            logger.warning("unknown filter %s", key)
    return JsonResponse({"n": Book.objects.filter(**request.GET.dict()).count()})  # caught


def other_mapping_checked(request):
    for key in request.POST:
        if key not in ALLOWED:
            raise ValueError(key)
    return JsonResponse({"n": Book.objects.filter(**request.GET.dict()).count()})  # caught
