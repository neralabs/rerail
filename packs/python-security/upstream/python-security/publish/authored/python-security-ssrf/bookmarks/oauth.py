import requests
from django.conf import settings
from django.http import HttpResponseRedirect

TOKEN_URL = "https://slack.com/api/oauth.v2.access"


def slack_complete(request):
    # An OAuth code exchange: the code travels in the body to a fixed provider URL.
    result = requests.post(
        TOKEN_URL,
        data={
            "client_id": settings.SLACK_CLIENT_ID,
            "client_secret": settings.SLACK_SECRET,
            "code": request.GET["code"],
        },
        timeout=10,
    )
    return HttpResponseRedirect("/integrations/?ok=%s" % result.ok)


def github_complete(request):
    result = requests.post(
        "https://github.com/login/oauth/access_token",
        params={"code": request.GET.get("code")},
        headers={"Accept": "application/json"},
        timeout=10,
    )
    return HttpResponseRedirect("/integrations/?ok=%s" % result.ok)
