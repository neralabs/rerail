# A Django REST framework module: its import makes this Django code.
from rest_framework.decorators import api_view


@api_view(["GET"])
async def me(request):
    return request.user.username  # caught
