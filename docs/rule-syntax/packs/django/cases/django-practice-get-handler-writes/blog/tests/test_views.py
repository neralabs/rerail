from django.views import View

from blog.models import Post


class FakeView(View):
    def get(self, request):
        Post.objects.create(title="x")
