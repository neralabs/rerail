from proj.celery import app


@app.task
def revoke_access(user_id):
    return user_id


def on_lost_eligibility(user_id):
    revoke_access.apply_async(args=[user_id], countdown=7200)  # caught
