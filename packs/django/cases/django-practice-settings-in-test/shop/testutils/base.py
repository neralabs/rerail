from django.conf import settings
from django.test import TestCase


# A base class whose tearDown puts the setting back for every test of its subclasses. The restore is no leak itself.
class NewsCase(TestCase):
    def tearDown(self):
        settings.NEWS_URL = "https://news.example"
        super().tearDown()
