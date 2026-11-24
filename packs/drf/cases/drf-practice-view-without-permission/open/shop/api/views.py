from typing import Any

from dj_rest_auth.views import LoginView
from guardian.mixins import PermissionRequiredMixin
from django.views import View
from drf_spectacular.views import SpectacularAPIView
from rest_framework import generics, mixins, viewsets
from rest_framework.authentication import SessionAuthentication
from rest_framework.decorators import action, api_view, permission_classes
from rest_framework.permissions import AllowAny, IsAdminUser, IsAuthenticated
from rest_framework.response import Response
from rest_framework.views import APIView

from shop.api.base import (
    AbstractExportView,
    AuthenticatedView,
    BaseReportView,
    ScopedView,
    ShopView,
    TypedAuthenticatedView,
)
from shop.api.serializers import BookSerializer
from shop.models import Book


class BookList(generics.ListAPIView):  # caught
    queryset = Book.objects.all()
    serializer_class = BookSerializer


class BookViewSet(viewsets.ModelViewSet):  # caught
    queryset = Book.objects.all()
    serializer_class = BookSerializer


class StatsView(APIView):  # caught
    def get(self, request):
        return Response({"books": Book.objects.count()})


# Its base, in another module, sets nothing either.
class ShelfView(ShopView):  # caught
    def get(self, request):
        return Response([])


class SalesReport(BaseReportView):  # caught
    def get(self, request):
        return Response({})


class CsvExport(AbstractExportView):  # caught
    def get(self, request):
        return Response("")


class BookAuthorViewSet(viewsets.GenericViewSet, mixins.ListModelMixin):  # caught
    queryset = Book.objects.all()
    serializer_class = BookSerializer


class TypedBookViewSet(viewsets.ModelViewSet[Book]):  # caught
    queryset = Book.objects.all()
    serializer_class = BookSerializer


class VersionView(generics.GenericAPIView[Any]):  # caught
    def get(self, request):
        return Response({"version": 1})


@api_view(["GET"])
def ping(request):  # caught
    return Response({"ok": True})


@api_view(["POST"])
def delete_all(request):  # caught
    Book.objects.all().delete()
    return Response(status=204)


# ---- silent ----------------------------------------------------------------------------------------------------------


# The base view, in another module, sets permission_classes.
class AccountView(AuthenticatedView):
    def get(self, request):
        return Response({})


# The base view, in another module, defines get_permissions().
class AdminView(ScopedView):
    def get(self, request):
        return Response({})


class OwnBookList(generics.ListAPIView):
    queryset = Book.objects.all()
    serializer_class = BookSerializer
    permission_classes = [IsAuthenticated]


class BookDetail(generics.RetrieveAPIView):
    queryset = Book.objects.all()
    serializer_class = BookSerializer

    def get_permissions(self):
        return [IsAuthenticated()]


# Its subscripted base, in another module, sets permission_classes.
class TypedAccountView(TypedAuthenticatedView):
    def get(self, request):
        return Response({})


# A base from another package, subscripted or not, decides the permissions.
class GuardedBookViewSet(PermissionRequiredMixin, viewsets.ModelViewSet[Book]):
    queryset = Book.objects.all()


# Public on purpose, and says so.
class HealthView(APIView):
    permission_classes = [AllowAny]

    def get(self, request):
        return Response({"ok": True})


class StatusView(APIView):
    permission_classes = ()

    def get(self, request):
        return Response({"ok": True})


# A third-party view class sets its own permissions.
class ShopLoginView(LoginView):
    pass


class SchemaView(SpectacularAPIView):
    pass


class ShopLoginBase(LoginView):
    throttle_scope = "login"


class StaffLoginView(ShopLoginBase):
    pass


@api_view(["GET"])
@permission_classes([IsAuthenticated])
def me(request):
    return Response({"user": request.user.pk})


@api_view(["GET"])
@permission_classes([AllowAny])
def version(request):
    return Response({"version": 1})


# Mixins alone do not make a view.
class ListOnly(mixins.ListModelMixin):
    pass


# A plain Django view: not DRF.
class HomeView(View):
    pass


def helper(request):
    return Response({})


# Every handler is an action that names its own permission classes, and the bases route nothing else.
class ReviewerViewSet(viewsets.GenericViewSet):
    @action(detail=True, methods=["post"], permission_classes=[IsAdminUser])
    def approve(self, request, pk=None):
        return Response(status=202)

    @action(detail=True, methods=["post"], permission_classes=[IsAdminUser])
    def reject(self, request, pk=None):
        return Response(status=202)

    def _notify(self, book):
        pass


# One action names its permission classes, the other does not.
class ModerationViewSet(viewsets.GenericViewSet):  # caught
    @action(detail=True, methods=["post"], permission_classes=[IsAdminUser])
    def hide(self, request, pk=None):
        return Response(status=202)

    @action(detail=True, methods=["post"])
    def flag(self, request, pk=None):
        return Response(status=202)


# The list and retrieve handlers ReadOnlyModelViewSet brings stay open.
class CatalogViewSet(viewsets.ReadOnlyModelViewSet):  # caught
    queryset = Book.objects.all()
    serializer_class = BookSerializer

    @action(detail=False, permission_classes=[IsAdminUser])
    def export(self, request):
        return Response([])


# No authentication at all: the view is public on purpose.
class TagListView(APIView):
    authentication_classes = []

    def get(self, request):
        return Response([])


# Authentication without permission classes still lets anonymous users in.
class FeedView(APIView):  # caught
    authentication_classes = [SessionAuthentication]

    def get(self, request):
        return Response([])
