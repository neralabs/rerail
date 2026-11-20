from shop.models import Article


def test_like():
    a = Article.objects.get(pk=1)
    a.likes += 1
    a.save()
