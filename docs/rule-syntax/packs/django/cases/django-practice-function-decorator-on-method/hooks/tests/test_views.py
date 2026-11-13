from django.views import View
from django.views.decorators.csrf import csrf_exempt


class FakeView(View):
    @csrf_exempt
    def post(self, request):
        pass
