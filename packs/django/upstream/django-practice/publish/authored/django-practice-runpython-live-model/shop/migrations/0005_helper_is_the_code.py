from django.db import migrations

from shop.helpers import recount_books


class Migration(migrations.Migration):
    dependencies = [("shop", "0004_job_runner")]

    operations = [
        migrations.RunPython(recount_books, migrations.RunPython.noop),  # caught
    ]
