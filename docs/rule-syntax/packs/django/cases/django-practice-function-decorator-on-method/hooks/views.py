from django.contrib.auth.decorators import login_not_required, login_required, permission_required
from django.utils.decorators import method_decorator
from django.views import View
from django.views.decorators import cache
from django.views.decorators.cache import cache_page, never_cache
from django.views.decorators.csrf import csrf_exempt
from django.views.decorators.debug import sensitive_post_parameters
from django.views.decorators.http import require_POST, require_http_methods
from django.views.decorators.vary import vary_on_cookie
from django.views.generic.base import ContextMixin
from rest_framework.views import APIView


class Hook(View):
    @csrf_exempt  # caught
    def post(self, request):
        pass

    @login_required  # caught
    def get(self, request):
        pass

    @permission_required("hooks.change_hook")  # caught
    def put(self, request):
        pass

    @require_POST  # caught
    def patch(self, request):
        pass

    @require_http_methods(["GET", "POST"])  # caught
    def options(self, request):
        pass


class Cached(View):
    @cache_page(60 * 15)  # caught
    def get(self, request):
        pass

    @never_cache  # caught
    def head(self, request):
        pass

    @cache.cache_control(max_age=60)  # caught
    def options(self, request):
        pass

    @sensitive_post_parameters("password")  # caught
    def post(self, request):
        pass


class Open(View):
    @login_not_required  # caught
    def get(self, request):
        pass


class Api(APIView):
    @csrf_exempt  # caught
    def post(self, request):
        pass


class NotAView:
    # Any instance method gets `self` first: the decorator breaks there too.
    @login_required  # caught
    def handle(self, request):
        pass

    # A mark on a method passed as a bound view (`admin_view(self.upload)`) is read through the bound method: it works.
    @csrf_exempt
    def upload(self, request):
        pass


class Namespace(View):
    # A function without `self` used directly as a URL callback (`Namespace.callback`): the decorator gets the request.
    @csrf_exempt
    def callback(request):
        pass


# Wrapped in method_decorator: right.
@method_decorator(csrf_exempt, name="dispatch")
class Wrapped(View):
    @method_decorator(login_required)
    def get(self, request):
        pass

    @method_decorator(never_cache)
    def head(self, request):
        pass


class Dispatch(View):
    # csrf_exempt and login_not_required only mark the function; as_view() copies dispatch's marks.
    @csrf_exempt
    def dispatch(self, *args, **kwargs):
        return super().dispatch(*args, **kwargs)


class OpenDispatch(View):
    @login_not_required
    def dispatch(self, *args, **kwargs):
        return super().dispatch(*args, **kwargs)


class LoginDispatch(View):
    # login_required on dispatch still gets `self` as its request.
    @login_required  # caught
    def dispatch(self, *args, **kwargs):
        return super().dispatch(*args, **kwargs)


class Harmless(View):
    # vary_on_cookie touches only the response.
    @vary_on_cookie
    def get(self, request):
        pass

    @staticmethod
    @csrf_exempt
    def helper(request):
        pass


# A plain function view: the decorator's own use.
@csrf_exempt
@login_required
def hook(request):
    pass


class ExtraContext(ContextMixin):
    # Django's view mixins are not views: a mark on a plain class's method may serve a bound-method view.
    @csrf_exempt
    def post(self, request):
        pass


class MixedView(ContextMixin, View):
    @csrf_exempt  # caught
    def post(self, request):
        pass
