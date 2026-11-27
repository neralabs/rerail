# A reusable app: no REST_FRAMEWORK settings in the run, so the host project's default decides.
from rest_framework import viewsets
from rest_framework.decorators import api_view
from rest_framework.response import Response


class ReviewViewSet(viewsets.ModelViewSet):
    pass


@api_view(["GET"])
def summary(request):
    return Response({})
