from rest_framework.decorators import api_view
from rest_framework.response import Response
from rest_framework.views import APIView


class FakeView(APIView):
    def get(self, request):
        return Response({})


@api_view(["GET"])
def fake(request):
    return Response({})
