from shop.models import Customer, SiteConfig


def site_config():
    # The lazy singleton: the first row, created when there is none. SiteConfig holds one row.
    config = SiteConfig.objects.all().first()
    if config is None:
        config = SiteConfig.objects.create()
    return config


def sample_customer():
    # A bare first() reads one row of many: Customer is not a one-row table.
    return Customer.objects.first()
