from django.core.management.base import BaseCommand

from shop.helpers import recount_books


class Command(BaseCommand):
    def handle(self, *args, **options):
        recount_books()
