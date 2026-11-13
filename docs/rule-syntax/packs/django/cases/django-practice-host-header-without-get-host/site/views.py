import logging

from django.http import HttpResponse

logger = logging.getLogger(__name__)


def reset_link(request, token):
    host = request.META["HTTP_HOST"]  # caught
    host = request.META.get("HTTP_HOST", "")  # caught
    host = request.headers["host"]  # caught
    host = request.headers.get("Host")  # caught
    host = request.META.get("HTTP_X_FORWARDED_HOST")  # caught
    host = request.headers.get("X-Forwarded-Host")  # caught
    return HttpResponse(f"https://{host}/reset/{token}")


class Page:
    def get(self, request):
        return self.request.META['HTTP_HOST']  # caught


def fine(request):
    host = request.get_host()
    agent = request.META.get("HTTP_USER_AGENT")
    agent = request.headers["User-Agent"]
    # A store: setting the header up, not reading it.
    request.META["HTTP_HOST"] = "example.com"
    del request.META["HTTP_X_FORWARDED_HOST"]
    # Logging the raw header is what a diagnostic should do.
    logger.warning(f"Unexpected origin; Host header: {request.headers.get('Host')}")
    logging.info("host %s", request.META["HTTP_HOST"])
    # A diagnostic page still reads the raw header: reported, the message says when to leave it.
    context = {"xfh": request.headers.get("X-Forwarded-Host")}  # caught
    return host, agent
