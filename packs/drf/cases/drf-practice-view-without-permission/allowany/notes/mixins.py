from rest_framework.permissions import IsAuthenticated


class OwnerPermsMixin:
    permission_classes = [IsAuthenticated]


class AuditMixin:
    def perform_create(self, serializer):
        serializer.save(created_by=self.request.user)
