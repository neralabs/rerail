from shop import http


def charge(order):
    return http.post("https://pay.example.com")
