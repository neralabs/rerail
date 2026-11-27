from catalog.models import Book


def test_filter(request):
    assert not Book.objects.filter(**request.GET.dict()).exists()
    assert Book.objects.order_by(request.GET["sort"]).count() == 0
