import subprocess
import sys

from django.core.management.base import BaseCommand


class Command(BaseCommand):
    def handle(self, *args, **options):
        subprocess.run(f"echo {sys.argv[-1]}", shell=True)
        subprocess.run(options["cmd"], shell=True)
