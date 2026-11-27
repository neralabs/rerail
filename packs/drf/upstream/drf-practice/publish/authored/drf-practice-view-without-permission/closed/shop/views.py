from rest_framework import generics, viewsets
from rest_framework.decorators import api_view
from rest_framework.response import Response
from rest_framework.views import APIView


class BookViewSet(viewsets.ModelViewSet):
    pass


class BookList(generics.ListAPIView):
    pass


class StatsView(APIView):
    pass


@api_view(["GET"])
def ping(request):
    return Response({"ok": True})
