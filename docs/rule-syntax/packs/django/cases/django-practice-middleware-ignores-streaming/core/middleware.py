from django.http import StreamingHttpResponse
from django.utils.deprecation import MiddlewareMixin


class Minify(MiddlewareMixin):
    def process_response(self, request, resp):
        if "text/html" in resp.get("Content-Type", ""):
            resp.content = minify(resp.content)  # caught
        return resp


class Size:
    def __init__(self, get_response):
        self.get_response = get_response

    def __call__(self, request):
        response = self.get_response(request)
        response["X-Size"] = str(len(response.content))  # caught
        return response

    async def __acall__(self, request):
        response = await self.get_response(request)
        print(response.content)  # caught
        return response


class Idempotency:
    def __call__(self, request):
        resp = self.get_response(request)
        if isinstance(resp.content, bytes):  # caught
            body = resp.content  # one finding per function: the first read
        elif hasattr(resp, "data"):
            body = resp.data
        return resp


def size_middleware(get_response):
    def middleware(request):
        response = get_response(request)
        response["X-Size"] = str(len(response.content))  # caught
        return response

    return middleware


# Guarded: each handles streaming responses or checks the response's type first.
class Guarded(MiddlewareMixin):
    def process_response(self, request, response):
        if response.streaming:
            return response
        response.content = minify(response.content)
        return response


class GuardedType:
    def __init__(self, get_response):
        self.get_response = get_response

    def __call__(self, request):
        response = self.get_response(request)
        if isinstance(response, StreamingHttpResponse):
            return response
        print(response.content)
        return response


def guarded_attr(get_response):
    def middleware(request):
        response = get_response(request)
        if hasattr(response, "content"):
            print(response.content)
        return response

    return middleware


# Not the response: request bodies and other objects have `content` too.
class Other:
    def __call__(self, request):
        other = fetch()
        print(other.content)
        return self.get_response(request)


def view(request):
    response = render(request, "x.html")
    return response.content


def minify(x):
    return x


def fetch():
    return None


def render(request, name):
    return None


# The read is in a try whose handler catches the AttributeError a streamed response raises.
class Caught:
    def __init__(self, get_response):
        self.get_response = get_response

    def __call__(self, request):
        response = self.get_response(request)
        try:
            body = response.content
        except AttributeError:
            return response
        response["X-Len"] = str(len(body))
        return response


# A handler for another error guards nothing.
class CaughtOther:
    def __call__(self, request):
        response = self.get_response(request)
        try:
            body = response.content.decode()  # caught
        except UnicodeDecodeError:
            body = ""
        return response
