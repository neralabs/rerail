from rest_framework import viewsets
from rest_framework.views import APIView

from notes.mixins import AuditMixin, OwnerPermsMixin


class NoteViewSet(AuditMixin, viewsets.ModelViewSet):  # caught
    pass


class TagView(APIView):  # caught
    pass


# A mixin of the run sets permission_classes.
class FolderViewSet(OwnerPermsMixin, viewsets.ModelViewSet):
    pass
