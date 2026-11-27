TIMEOUT = 5


def render_body(order):
    return f"Order {order}"


def send_email(to, body):
    raise RuntimeError("real SMTP")


def confirm(order):
    return render_body(order)


def archive(order):
    return order


def log_sent(order):
    return order


def audit(order):
    return order
