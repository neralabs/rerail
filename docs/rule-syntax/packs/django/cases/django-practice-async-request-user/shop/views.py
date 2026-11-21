from asgiref.sync import sync_to_async
from django.http import HttpResponse, JsonResponse
from django.views import View


async def profile(request):
    if not request.user.is_authenticated:  # caught
        return HttpResponse(status=403)
    return JsonResponse({"name": request.user.username})  # caught


async def by_getattr(request):
    user = getattr(request, "user", None)  # caught
    return JsonResponse({"id": user.pk})


class ProfileView(View):
    async def get(self, request):
        user = self.request.user  # caught
        other = request.user  # caught
        return JsonResponse({"a": user.pk, "b": other.pk})


async def closure(request):
    async def inner():
        return request.user.pk  # caught

    return JsonResponse({"pk": await inner()})


# The async API and code handed to a thread are left alone.
async def good(request):
    user = await request.auser()
    allowed = await sync_to_async(lambda: request.user.has_perm("x"))()

    def check():
        return request.user.is_staff

    staff = await sync_to_async(check)()
    owner = await sync_to_async(is_owner)(request.user)
    return JsonResponse({"u": user.pk, "a": allowed, "s": staff, "o": owner})


def is_owner(user):
    return user.is_superuser


# Sync views may read request.user.
def sync_view(request):
    return JsonResponse({"name": request.user.username})


class AuthMiddleware:
    def __init__(self, get_response):
        self.get_response = get_response

    async def __call__(self, request):
        request.user = await get_user(request)  # a write, not a read
        return await self.get_response(request)


async def get_user(request):
    return None


# `request` that is not a parameter (a module global here) is not this rule's business.
request = None


async def not_a_param():
    return request.user
