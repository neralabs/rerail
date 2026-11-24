from typing import Any

from rest_framework import generics, permissions
from rest_framework.views import APIView


# A base view with no permissions: reported through its subclasses, not itself.
class ShopView(generics.GenericAPIView):
    pagination_class = None


class BaseReportView(APIView):
    renderer_classes = []


class AbstractExportView(APIView):
    pass


class AuthenticatedView(APIView):
    permission_classes = [permissions.IsAuthenticated]


class ScopedView(APIView):
    def get_permissions(self):
        return [permissions.IsAdminUser()]


# Named as a base, with no subclass yet: not reported.
class BaseImportView(APIView):
    pass


class AbstractSyncView(generics.GenericAPIView):
    pass


# drf-stubs' generic bases: `GenericAPIView[Any]` is GenericAPIView.
class TypedAuthenticatedView(generics.GenericAPIView[Any]):
    permission_classes = [permissions.IsAuthenticated]
