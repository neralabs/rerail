from shop.tasks import notify_order


def audit_middleware(get_response):
    # Function-style middleware runs outside the view's transaction: Django wraps only the view
    # (BaseHandler.make_view_atomic).
    def middleware(request):
        response = get_response(request)
        notify_order.delay(0)
        return response

    return middleware
