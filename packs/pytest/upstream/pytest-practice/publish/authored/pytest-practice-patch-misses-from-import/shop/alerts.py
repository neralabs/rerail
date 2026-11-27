from shop.notify import send_sms as sms


def alert(number):
    sms(number, "down")
