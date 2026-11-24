from rest_framework.permissions import IsAuthenticated
from rest_framework.views import APIView

from library.api.permissions import IsOwner


class OwnerAPIView(APIView):
    permission_classes = [IsAuthenticated, IsOwner]


class SignedInAPIView(APIView):
    permission_classes = [IsAuthenticated]
