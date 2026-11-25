from django.contrib import admin
from django.contrib.admin.views.decorators import staff_member_required
from django.contrib.auth.decorators import permission_required, user_passes_test
from django.core.exceptions import PermissionDenied
from django.http import Http404, HttpResponseForbidden, JsonResponse
from django.utils.decorators import method_decorator
from django.views import View
from rest_framework.decorators import api_view, permission_classes
from rest_framework.permissions import IsAdminUser, IsAuthenticated
from rest_framework.response import Response
from rest_framework.views import APIView

from catalog import helpers
from catalog.models import Book


class BookAdmin(admin.ModelAdmin):
    def stock_view(self, request):
        return JsonResponse({"n": Book.objects.order_by(request.GET.get("o", "title")).count()})


class SpecialBookAdmin(BookAdmin):
    def report_view(self, request):
        return JsonResponse({"rows": list(Book.objects.values(*request.GET.getlist("f")))})


def staff_correction(request):
    if not request.user.is_staff:
        raise PermissionDenied
    sort = request.GET.get("o", "title")
    return JsonResponse({"n": Book.objects.filter(**request.GET.dict()).order_by(sort).count()})


def superuser_report(request):
    if not (request.user.is_superuser or request.user.is_staff):
        return HttpResponseForbidden()
    return JsonResponse({"n": Book.objects.order_by(request.GET["o"]).count()})


def staff_branch(request):
    books = Book.objects.all()
    if request.user.is_staff:
        books = books.order_by(request.GET["o"])
    return JsonResponse({"n": books.count()})


@staff_member_required
def staff_export(request):
    return JsonResponse({"n": helpers.staff_sort(request).count()})


@user_passes_test(lambda u: u.is_superuser)
def superuser_export(request):
    return JsonResponse({"n": helpers.shared_sort(request).count()})


@permission_required("catalog.view_book")
def permitted_export(request):
    return JsonResponse({"n": Book.objects.order_by(request.GET["o"]).count()})


@api_view(["GET"])
@permission_classes([IsAdminUser])
def admin_api(request):
    return Response({"n": helpers.staff_sort(request).count()})


class AdminBooks(APIView):
    permission_classes = [IsAdminUser]

    def get(self, request):
        return Response({"n": Book.objects.filter(**request.query_params.dict()).count()})


@method_decorator(staff_member_required, name="dispatch")
class StaffBooks(View):
    def get(self, request):
        return JsonResponse({"n": Book.objects.order_by(request.GET["o"]).count()})


class GuardedDispatch(View):
    def dispatch(self, request, *args, **kwargs):
        if not request.user.is_staff:
            raise Http404
        return super().dispatch(request, *args, **kwargs)

    def get(self, request):
        return JsonResponse({"n": Book.objects.order_by(request.GET["o"]).count()})


# Guards that let users who are not staff through.
def logged_in_only(request):
    if not request.user.is_authenticated:
        raise PermissionDenied
    return JsonResponse({"n": Book.objects.order_by(request.GET["o"]).count()})  # caught


def staff_flag_only(request):
    show_all = request.user.is_staff
    return JsonResponse({"n": Book.objects.order_by(request.GET["o"]).count(), "all": show_all})  # caught


class AdminOrUser(APIView):
    permission_classes = [IsAdminUser | IsAuthenticated]

    def get(self, request):
        return Response({"n": Book.objects.filter(**request.query_params.dict()).count()})  # caught
