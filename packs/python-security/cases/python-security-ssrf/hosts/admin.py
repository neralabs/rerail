import requests
from django.contrib import admin
from django.contrib.admin.views.decorators import staff_member_required
from django.contrib.auth.decorators import login_required, user_passes_test
from django.core.exceptions import PermissionDenied
from django.http import HttpResponse
from fastapi import APIRouter, Depends
from rest_framework.permissions import IsAdminUser, IsAuthenticated
from rest_framework.response import Response
from rest_framework.views import APIView

router = APIRouter()


def get_admin_user():
    ...


def get_verified_user():
    ...


@router.post("/tools/fetch")
def admin_fetch(url: str, user=Depends(get_admin_user)):
    return requests.get(url, timeout=5).text


@router.post("/tools/load", dependencies=[Depends(get_admin_user)])
def admin_load(url: str):
    return requests.get(url, timeout=5).text


@router.post("/proxy")
def user_fetch(url: str, user=Depends(get_verified_user)):
    return requests.get(url, timeout=5).text  # caught


@staff_member_required
def staff_view(request):
    return HttpResponse(requests.get(request.GET["url"], timeout=5).content)


@user_passes_test(lambda u: u.is_superuser)
def superuser_view(request):
    return HttpResponse(requests.get(request.GET["url"], timeout=5).content)


def staff_checked(request):
    if not request.user.is_staff:
        raise PermissionDenied
    return HttpResponse(requests.get(request.GET["url"], timeout=5).content)


@login_required
def member_view(request):
    return HttpResponse(requests.get(request.GET["url"], timeout=5).content)  # caught


class WebhookAdmin(admin.ModelAdmin):
    def test_hook(self, request):
        return HttpResponse(requests.post(request.POST["url"], timeout=5).status_code)


class AdminProbe(APIView):
    permission_classes = [IsAdminUser]

    def post(self, request):
        return Response(requests.get(request.data["url"], timeout=5).status_code)


class UserProbe(APIView):
    permission_classes = [IsAuthenticated]

    def post(self, request):
        return Response(requests.get(request.data["url"], timeout=5).status_code)  # caught
