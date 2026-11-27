from django.db import migrations, models


def lowercase_emails(apps, schema_editor):
    Customer = apps.get_model("shop", "Customer")
    for customer in Customer.objects.all():
        customer.email = customer.email.lower()
        customer.save(update_fields=["email"])


class Migration(migrations.Migration):
    dependencies = [("shop", "0004_product_price_cents")]

    # The index is built first; its lock on the table lasts through the whole backfill.
    operations = [
        migrations.AddIndex(
            model_name="customer",
            index=models.Index(fields=["email"], name="shop_customer_email_idx"),
        ),
        migrations.RunPython(lowercase_emails, migrations.RunPython.noop),  # caught
    ]
