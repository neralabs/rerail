from django.db import migrations, models


def merge_duplicate_customers(apps, schema_editor):
    Customer = apps.get_model("shop", "Customer")
    Order = apps.get_model("shop", "Order")
    kept = {}
    for customer in Customer.objects.order_by("pk"):
        key = customer.email.lower()
        if key in kept:
            Order.objects.filter(customer=customer).update(customer=kept[key])
            customer.delete()
        else:
            kept[key] = customer


class Migration(migrations.Migration):
    dependencies = [("shop", "0002_order_number")]

    # Order's foreign key points at the deleted customers: PostgreSQL queues a check for each one until commit.
    operations = [
        migrations.RunPython(  # caught
            merge_duplicate_customers,
            reverse_code=migrations.RunPython.noop,
        ),
        migrations.AlterField(
            model_name="customer",
            name="email",
            field=models.EmailField(max_length=254, unique=True),
        ),
    ]
