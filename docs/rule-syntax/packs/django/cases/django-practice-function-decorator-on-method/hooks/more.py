from django.contrib.admin.views.decorators import staff_member_required
from django.views import View
from django.views.decorators.common import no_append_slash
from django.views.decorators.csrf import csrf_exempt
from django.views.decorators.gzip import gzip_page
from rest_framework.generics import ListAPIView


class Admin(View):
    # A user_passes_test wrapper: AttributeError on `self.user`.
    @staff_member_required  # caught
    def get(self, request):
        pass

    # Fails on the responses it would compress.
    @gzip_page  # caught
    def post(self, request):
        pass


class Slash(View):
    # A mark like csrf_exempt: as_view() copies dispatch's marks only.
    @no_append_slash  # caught
    def get(self, request):
        pass


class SlashDispatch(View):
    @no_append_slash
    def dispatch(self, *args, **kwargs):
        return super().dispatch(*args, **kwargs)


class Items(ListAPIView):
    # DRF exempts the view already: reported as redundant (remove it), not as "wrap it".
    @csrf_exempt  # caught
    def post(self, request):
        pass


class OwnItems(Items):
    @csrf_exempt  # caught
    def put(self, request):
        pass

    # On dispatch the mark is copied, and redundant on a DRF view: left alone like on any view.
    @csrf_exempt
    def dispatch(self, *args, **kwargs):
        return super().dispatch(*args, **kwargs)
