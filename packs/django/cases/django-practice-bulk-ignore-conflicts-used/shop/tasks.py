from celery import shared_task


@shared_task
def send_invite(email, token):
    pass
