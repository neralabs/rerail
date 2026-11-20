from django.views import View

from blog.models import Post


class ViewSetMixin:
    """A vendored copy of DRF's ViewSetMixin: as_view() routes GET to list() and retrieve()."""

    @classmethod
    def as_view(cls, actions=None, **initkwargs):
        return super().as_view(**initkwargs)


class GenericViewSet(ViewSetMixin, View):
    pass


class SettingsViewSet(GenericViewSet):
    def list(self, request):  # caught
        Post.objects.get_or_create(title="settings")
        return None
