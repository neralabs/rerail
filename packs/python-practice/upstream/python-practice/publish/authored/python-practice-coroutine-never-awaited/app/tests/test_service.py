import threading

from app.notify import send_mail


def test_send_mail():
    send_mail(None)
    threading.Thread(target=send_mail, args=(None,)).start()
