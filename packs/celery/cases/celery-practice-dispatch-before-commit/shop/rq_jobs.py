from rq.decorators import job


@job("default")
def reindex(order_id):
    return order_id
