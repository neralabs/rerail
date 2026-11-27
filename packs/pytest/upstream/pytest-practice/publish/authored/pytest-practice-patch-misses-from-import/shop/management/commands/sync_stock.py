from django.core.management.base import BaseCommand

from shop.stock import refresh


class Command(BaseCommand):
    def handle(self, *args, **options):
        refresh()
