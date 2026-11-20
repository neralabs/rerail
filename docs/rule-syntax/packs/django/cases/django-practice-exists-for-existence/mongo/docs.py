from mongoengine import Document, StringField


class Page(Document):
    slug = StringField()


def has_page(slug):
    # mongoengine's QuerySet has count() but no exists(); no django import in this file
    if Page.objects(slug=slug).count() > 0:
        return True
    return len(Page.objects.filter(slug=slug)) == 0
