# Architecture rule examples

Design rules that maintainers of well-known open-source projects enforce in review, written as Rerail rules. No stock
linter checks them, and several projects say no tool does.

Each example shows breaking code, the rule in brief, its source, and the full rule with test cases that Rerail's test
suite runs through the engine.

To use one:

1. Open the example's **full rule** fold and copy its JSON into `draft.json`.
2. Change the names and paths to your repository's.
3. Run `rerail rules test draft.json`, then `rerail rules add draft.json` ([Testing a draft](writing-rules.md#testing-a-draft)).

Or describe the rule to `/rerail-create` and let the agent write it.

## Python

Rules that send control-silo reads, HTTP and client events through the one layer each project allows.

### Sentry: cell-silo code reads control-silo models through RPC

Users and integrations live in Sentry's control silo; a cell's database holds only its organizations' data. A
cell-silo query on `User` or `Integration` passes locally, where one database holds every table, and fails in
production. Asked to post a status check, an agent writes:

`src/sentry/preprod/vcs/status_checks/size/tasks.py`

```python
from sentry.integrations.models.integration import Integration
from sentry.models.repository import Repository
from sentry.silo.base import SiloMode
from sentry.tasks.base import instrumented_task
from sentry.taskworker.namespaces import preprod_tasks


@instrumented_task(
    name="sentry.preprod.tasks.post_status_check",
    namespace=preprod_tasks,
    silo_mode=SiloMode.CELL,
)
def post_status_check(repository_id: int) -> None:
    repository = Repository.objects.get(id=repository_id)
    integration = Integration.objects.get(id=repository.integration_id)
    installation = integration.get_installation(repository.organization_id)
    installation.post_check()
```

```rule
rule    local:cell-silo-reads-control-through-rpc                             steer
files   src/sentry/**/*.py, except testutils and migrations
forbid  Model.objects on a user model, Integration or OrganizationIntegration, under any import form or alias,
        inside a @cell_silo_endpoint class or an @instrumented_task(silo_mode=SiloMode.CELL) function
say     "$MODEL lives in the control silo, so this cell-silo code can't query it in production: fetch it through its RPC service (user_service, integration_service)."
allow   cell models such as Repository; user_service and integration_service; control-silo endpoints; a control model used only as a type
misses  cell-silo code not marked in its own file, such as helpers a cell endpoint calls; control models other than users and integrations
```

From review: [getsentry/sentry#98301](https://github.com/getsentry/sentry/pull/98301#discussion_r2307818686).

<details>
<summary>Review quote and full rule: copy into <code>draft.json</code></summary>

The review: “the `Integration` model is scoped to the control silo.” The fix it asked for: “So that we use the RPC to fetch the integration from the control silo.”

```json draft
{
  "kind": "code",
  "rule": {
    "format": "rerail.policy/v1",
    "id": "local:cell-silo-reads-control-through-rpc",
    "revision": 1,
    "mode": "steer",
    "description": "Fetch users and integrations through their RPC services in cell-silo endpoints and tasks, not by querying the control-silo models, which a cell's database doesn't hold.",
    "scope": { "include": ["src/sentry/**/*.py"], "exclude": ["src/sentry/testutils/**", "src/sentry/**/migrations/**"] },
    "clauses": [
      {
        "id": "control-model-in-cell-silo",
        "forbid": {
          "ast": {
            "language": "python",
            "utils": {
              "control-model": {
                "any": [{ "kind": "identifier" }, { "kind": "attribute" }],
                "resolves": {
                  "import": [
                    "sentry.users.models",
                    "sentry.integrations.models.Integration",
                    "sentry.integrations.models.integration.Integration",
                    "sentry.integrations.models.OrganizationIntegration",
                    "sentry.integrations.models.organization_integration.OrganizationIntegration"
                  ],
                  "prefix": true
                }
              },
              "cell-silo": {
                "kind": "decorated_definition",
                "has": {
                  "kind": "decorator",
                  "any": [
                    {
                      "has": {
                        "any": [{ "kind": "identifier" }, { "kind": "attribute" }],
                        "resolves": { "import": ["sentry.api.base.cell_silo_endpoint"] }
                      }
                    },
                    { "regex": "(?s)^@instrumented_task\\(.*\\bsilo_mode\\s*=\\s*SiloMode\\.CELL\\b" }
                  ]
                }
              }
            },
            "rule": {
              "kind": "attribute",
              "has": { "field": "object", "matches": "control-model", "pattern": "$MODEL" },
              "regex": "\\.objects$",
              "inside": { "matches": "cell-silo", "stopBy": "end" }
            }
          }
        },
        "violation": {
          "code": "CONTROL_MODEL_IN_CELL_SILO",
          "anchor": "witness.span",
          "message": "$MODEL lives in the control silo, so this cell-silo code can't query it in production: fetch it through its RPC service (user_service, integration_service)."
        }
      }
    ]
  },
  "cases": {
    "caught": [
      { "path": "src/sentry/preprod/vcs/status_checks/size/tasks.py", "content": "from sentry.integrations.models.integration import Integration\nfrom sentry.models.repository import Repository\nfrom sentry.silo.base import SiloMode\nfrom sentry.tasks.base import instrumented_task\nfrom sentry.taskworker.namespaces import preprod_tasks\n\n\n@instrumented_task(\n    name=\"sentry.preprod.tasks.post_status_check\",\n    namespace=preprod_tasks,\n    silo_mode=SiloMode.CELL,\n)\ndef post_status_check(repository_id: int) -> None:\n    repository = Repository.objects.get(id=repository_id)\n    integration = Integration.objects.get(id=repository.integration_id)\n    installation = integration.get_installation(repository.organization_id)\n    installation.post_check()\n" },
      { "path": "src/sentry/issues/endpoints/organization_reviewers.py", "content": "from rest_framework.request import Request\nfrom rest_framework.response import Response\n\nfrom sentry.api.api_owners import ApiOwner\nfrom sentry.api.base import cell_silo_endpoint\nfrom sentry.api.bases.organization import OrganizationEndpoint\nfrom sentry.users.models import User as SentryUser\n\n\n@cell_silo_endpoint\nclass OrganizationReviewersEndpoint(OrganizationEndpoint):\n    owner = ApiOwner.ISSUES\n\n    def get(self, request: Request, organization) -> Response:\n        users = SentryUser.objects.filter(id__in=request.GET.getlist(\"id\"))\n        return Response([user.email for user in users])\n" },
      { "path": "src/sentry/integrations/api/endpoints/organization_integration_status.py", "content": "from sentry.api import base\nfrom sentry.api.bases.organization import OrganizationEndpoint\n\n\n@base.cell_silo_endpoint\nclass OrganizationIntegrationStatusEndpoint(OrganizationEndpoint):\n    def get(self, request, organization, integration_id):\n        from sentry.integrations.models import OrganizationIntegration\n\n        exists = OrganizationIntegration.objects.filter(\n            organization_id=organization.id, integration_id=integration_id\n        ).exists()\n        return self.respond({\"installed\": exists})\n" }
    ],
    "passed": [
      { "path": "src/sentry/preprod/vcs/status_checks/size/tasks.py", "content": "from sentry.integrations.services.integration import integration_service\nfrom sentry.models.repository import Repository\nfrom sentry.silo.base import SiloMode\nfrom sentry.tasks.base import instrumented_task\nfrom sentry.taskworker.namespaces import preprod_tasks\n\n\n@instrumented_task(name=\"sentry.preprod.tasks.post_status_check\", namespace=preprod_tasks, silo_mode=SiloMode.CELL)\ndef post_status_check(repository_id: int) -> None:\n    repository = Repository.objects.get(id=repository_id)\n    integration = integration_service.get_integration(integration_id=repository.integration_id)\n    integration.get_installation(organization_id=repository.organization_id).post_check()\n" },
      { "path": "src/sentry/issues/endpoints/organization_reviewers.py", "content": "from rest_framework.request import Request\nfrom rest_framework.response import Response\n\nfrom sentry.api.base import cell_silo_endpoint\nfrom sentry.api.bases.organization import OrganizationEndpoint\nfrom sentry.models.organizationmember import OrganizationMember\nfrom sentry.users.services.user.service import user_service\n\n\n@cell_silo_endpoint\nclass OrganizationReviewersEndpoint(OrganizationEndpoint):\n    def get(self, request: Request, organization) -> Response:\n        member_user_ids = OrganizationMember.objects.filter(\n            organization_id=organization.id\n        ).values_list(\"user_id\", flat=True)\n        users = user_service.get_many_by_id(ids=list(member_user_ids))\n        return Response([user.email for user in users])\n" },
      { "path": "src/sentry/users/api/endpoints/user_emails.py", "content": "from sentry.api.base import control_silo_endpoint\nfrom sentry.users.api.bases.user import UserEndpoint\nfrom sentry.users.models.useremail import UserEmail\n\n\n@control_silo_endpoint\nclass UserEmailsEndpoint(UserEndpoint):\n    def get(self, request, user):\n        emails = UserEmail.objects.filter(user=user)\n        return self.respond([email.email for email in emails])\n" },
      { "path": "src/sentry/tasks/user_report.py", "content": "from sentry.models.userreport import UserReport\nfrom sentry.silo.base import SiloMode\nfrom sentry.tasks.base import instrumented_task\nfrom sentry.taskworker.namespaces import issues_tasks\nfrom sentry.users.models.user import User\n\n\n@instrumented_task(name=\"sentry.tasks.user_report.link\", namespace=issues_tasks, silo_mode=SiloMode.CELL)\ndef link_user_report(report_id: int, user: User | None = None) -> None:\n    UserReport.objects.filter(id=report_id).update(user_id=user.id if user else None)\n" }
    ]
  }
}
```

</details>

### Home Assistant: integrations talk HTTP through a library and the shared session

Home Assistant keeps device protocols in PyPI libraries and hands them its shared web session. An integration's own
`ClientSession` adds a connection pool per config entry that never closes; an inline `requests` call puts protocol
code in core. A new integration's setup often starts like this:

`homeassistant/components/pool_monitor/__init__.py`

```python
"""The Pool Monitor integration."""

from aiohttp import ClientSession
from pool_monitor import PoolMonitorClient

from homeassistant.config_entries import ConfigEntry
from homeassistant.const import CONF_HOST
from homeassistant.core import HomeAssistant


async def async_setup_entry(hass: HomeAssistant, entry: ConfigEntry) -> bool:
    """Set up Pool Monitor from a config entry."""
    client = PoolMonitorClient(entry.data[CONF_HOST], session=ClientSession())
    entry.runtime_data = await client.get_device()
    return True
```

```rule
rule    local:integration-http-through-library                                steer
files   homeassistant/components/**/*.py, except the generic rest, rest_command and scrape integrations
forbid  calling aiohttp.ClientSession(), under any import alias
say     "Give the library async_get_clientsession(hass) instead of opening a new $SESSION; use async_create_clientsession(hass) if its cookies must stay private."
forbid  calling httpx.AsyncClient() or httpx.Client()
say     "Give the library get_async_client(hass) instead of opening a new $SESSION; use create_async_httpx_client(hass) if its cookies must stay private."
forbid  requests.get/post/…, httpx.get/post/…, aiohttp.request or urllib.request.urlopen, under any alias
say     "Move this HTTP call ($CALL) into the integration's PyPI library and call the library from here; integrations hold no protocol code."
allow   async_get_clientsession and async_create_clientsession; aiohttp.ClientTimeout and ClientError; a library's own request();
        ClientSession() in a comment; homeassistant/helpers/
misses  raw HTTP over the shared session itself, such as session.get(url) written in the integration
```

From review: [home-assistant/core#65090](https://github.com/home-assistant/core/pull/65090#discussion_r870416576) and the [development checklist](https://contributors.home-assistant.io/core/development/checklist/).

<details>
<summary>Review quotes and full rule: copy into <code>draft.json</code></summary>

The review: “please use `async_get_clientsession` both here and in the config flow to reduce sessions created”. The checklist: “All communication with devices and services goes through a Python library published on PyPI. The integration itself contains no protocol code.”

```json draft
{
  "kind": "code",
  "rule": {
    "format": "rerail.policy/v1",
    "id": "local:integration-http-through-library",
    "revision": 1,
    "mode": "steer",
    "description": "Keep HTTP out of Home Assistant integrations: call the device's PyPI library and pass it Home Assistant's shared web session instead of opening one.",
    "scope": {
      "include": ["homeassistant/components/**/*.py"],
      "exclude": [
        "homeassistant/components/rest/**",
        "homeassistant/components/rest_command/**",
        "homeassistant/components/scrape/**"
      ]
    },
    "clauses": [
      {
        "id": "own-aiohttp-session",
        "forbid": {
          "ast": {
            "language": "python",
            "rule": {
              "kind": "call",
              "has": {
                "field": "function",
                "pattern": "$SESSION",
                "any": [{ "kind": "identifier" }, { "kind": "attribute" }],
                "resolves": { "import": ["aiohttp.ClientSession"] }
              }
            }
          }
        },
        "violation": {
          "code": "OWN_AIOHTTP_SESSION",
          "anchor": "witness.span",
          "message": "Give the library async_get_clientsession(hass) instead of opening a new $SESSION; use async_create_clientsession(hass) if its cookies must stay private."
        }
      },
      {
        "id": "own-httpx-client",
        "forbid": {
          "ast": {
            "language": "python",
            "rule": {
              "kind": "call",
              "has": {
                "field": "function",
                "pattern": "$SESSION",
                "any": [{ "kind": "identifier" }, { "kind": "attribute" }],
                "resolves": { "import": ["httpx.AsyncClient", "httpx.Client"] }
              }
            }
          }
        },
        "violation": {
          "code": "OWN_HTTPX_CLIENT",
          "anchor": "witness.span",
          "message": "Give the library get_async_client(hass) instead of opening a new $SESSION; use create_async_httpx_client(hass) if its cookies must stay private."
        }
      },
      {
        "id": "raw-http-call",
        "forbid": {
          "ast": {
            "language": "python",
            "rule": {
              "kind": "call",
              "has": {
                "field": "function",
                "pattern": "$CALL",
                "any": [{ "kind": "identifier" }, { "kind": "attribute" }],
                "resolves": {
                  "import": [
                    "requests.get",
                    "requests.post",
                    "requests.put",
                    "requests.patch",
                    "requests.delete",
                    "requests.head",
                    "requests.request",
                    "httpx.get",
                    "httpx.post",
                    "httpx.put",
                    "httpx.patch",
                    "httpx.delete",
                    "httpx.head",
                    "httpx.request",
                    "httpx.stream",
                    "aiohttp.request",
                    "urllib.request.urlopen"
                  ]
                }
              }
            }
          }
        },
        "violation": {
          "code": "HTTP_IN_INTEGRATION",
          "anchor": "witness.span",
          "message": "Move this HTTP call ($CALL) into the integration's PyPI library and call the library from here; integrations hold no protocol code."
        }
      }
    ]
  },
  "cases": {
    "caught": [
      { "path": "homeassistant/components/pool_monitor/__init__.py", "content": "\"\"\"The Pool Monitor integration.\"\"\"\n\nfrom aiohttp import ClientSession\nfrom pool_monitor import PoolMonitorClient\n\nfrom homeassistant.config_entries import ConfigEntry\nfrom homeassistant.const import CONF_HOST\nfrom homeassistant.core import HomeAssistant\n\n\nasync def async_setup_entry(hass: HomeAssistant, entry: ConfigEntry) -> bool:\n    \"\"\"Set up Pool Monitor from a config entry.\"\"\"\n    client = PoolMonitorClient(entry.data[CONF_HOST], session=ClientSession())\n    entry.runtime_data = await client.get_device()\n    return True\n" },
      { "path": "homeassistant/components/pool_monitor/coordinator.py", "content": "\"\"\"Coordinator for Pool Monitor.\"\"\"\n\nimport requests\n\nfrom homeassistant.helpers.update_coordinator import DataUpdateCoordinator\n\n\nclass PoolMonitorCoordinator(DataUpdateCoordinator[dict]):\n    def _fetch(self) -> dict:\n        response = requests.get(f\"http://{self.host}/api/v1/status\", timeout=10)\n        response.raise_for_status()\n        return response.json()\n\n    async def _async_update_data(self) -> dict:\n        return await self.hass.async_add_executor_job(self._fetch)\n" },
      { "path": "homeassistant/components/pool_monitor/config_flow.py", "content": "\"\"\"Config flow for Pool Monitor.\"\"\"\n\nimport httpx as http\n\nfrom homeassistant.config_entries import ConfigFlow, ConfigFlowResult\n\n\nclass PoolMonitorConfigFlow(ConfigFlow, domain=\"pool_monitor\"):\n    async def async_step_user(self, user_input=None) -> ConfigFlowResult:\n        if user_input is not None:\n            async with http.AsyncClient() as client:\n                await client.get(f\"http://{user_input['host']}/api/v1/info\")\n            return self.async_create_entry(title=\"Pool\", data=user_input)\n        return self.async_show_form(step_id=\"user\")\n" },
      { "path": "homeassistant/components/pool_monitor/sensor.py", "content": "from aiohttp import request as http_request\n\n\nasync def _read_ph(host: str) -> float:\n    async with http_request(\"GET\", f\"http://{host}/api/v1/ph\") as response:\n        return (await response.json())[\"ph\"]\n" }
    ],
    "passed": [
      { "path": "homeassistant/components/pool_monitor/__init__.py", "content": "\"\"\"The Pool Monitor integration.\"\"\"\n\nfrom pool_monitor import PoolMonitorClient\n\nfrom homeassistant.config_entries import ConfigEntry\nfrom homeassistant.const import CONF_HOST\nfrom homeassistant.core import HomeAssistant\nfrom homeassistant.helpers.aiohttp_client import async_get_clientsession\n\n\nasync def async_setup_entry(hass: HomeAssistant, entry: ConfigEntry) -> bool:\n    \"\"\"Set up Pool Monitor from a config entry.\"\"\"\n    session = async_get_clientsession(hass)\n    client = PoolMonitorClient(entry.data[CONF_HOST], session=session)\n    entry.runtime_data = await client.get_device()\n    return True\n" },
      { "path": "homeassistant/components/pool_monitor/config_flow.py", "content": "\"\"\"Config flow for Pool Monitor.\"\"\"\n\nimport aiohttp\nfrom pool_monitor import PoolMonitorClient\n\nfrom homeassistant.config_entries import ConfigFlow, ConfigFlowResult\nfrom homeassistant.helpers.aiohttp_client import async_create_clientsession\n\n\nclass PoolMonitorConfigFlow(ConfigFlow, domain=\"pool_monitor\"):\n    async def async_step_user(self, user_input=None) -> ConfigFlowResult:\n        # The device sets a login cookie, so this flow gets its own session, not ClientSession().\n        session = async_create_clientsession(self.hass)\n        client = PoolMonitorClient(user_input[\"host\"], session=session, timeout=aiohttp.ClientTimeout(total=10))\n        try:\n            await client.get(\"info\")\n        except aiohttp.ClientError:\n            return self.async_show_form(step_id=\"user\", errors={\"base\": \"cannot_connect\"})\n        return self.async_create_entry(title=\"Pool\", data=user_input)\n" },
      { "path": "homeassistant/components/pool_monitor/coordinator.py", "content": "\"\"\"Coordinator for Pool Monitor.\"\"\"\n\nfrom pool_monitor import PoolMonitorClient, request\n\nfrom homeassistant.helpers.update_coordinator import DataUpdateCoordinator\n\n\nclass PoolMonitorCoordinator(DataUpdateCoordinator[dict]):\n    client: PoolMonitorClient\n\n    async def _async_update_data(self) -> dict:\n        return await self.client.send(request(\"status\"))\n" },
      { "path": "homeassistant/helpers/aiohttp_client.py", "content": "import aiohttp\n\n\ndef _async_create_clientsession(hass, **kwargs) -> aiohttp.ClientSession:\n    return aiohttp.ClientSession(**kwargs)\n" }
    ]
  }
}
```

</details>

### Zulip: views validate; zerver/actions changes state and tells clients

In Zulip, a `do_` function in `zerver/actions/` makes a change, writes its `RealmAuditLog` row and sends the event
that updates open clients. A view that sends the event itself bypasses it, so other callers of the same change (an
endpoint, a management command, an import) send no event and clients go stale:

`zerver/views/drafts.py`

```python
from django.http import HttpRequest, HttpResponse

from zerver.lib.drafts import access_draft
from zerver.lib.response import json_success
from zerver.models import UserProfile
from zerver.tornado.django_api import send_event_on_commit


def delete_draft(
    request: HttpRequest, user_profile: UserProfile, draft_id: int
) -> HttpResponse:
    draft = access_draft(user_profile, draft_id)
    draft.delete()
    event = {"type": "drafts", "op": "remove", "draft_id": draft_id}
    send_event_on_commit(user_profile.realm, event, [user_profile.id])
    return json_success(request)
```

```rule
rule    local:views-change-state-through-actions                              steer
files   zerver/views/**/*.py, except zerver/views/development/
forbid  send_event_on_commit or send_event_rollback_unsafe from zerver.tornado.django_api, in any import form
say     "Move this change and its $SEND call into a do_ function in zerver/actions/ and call that from the view; views validate input, actions write and notify."
forbid  RealmAuditLog.objects.create() or bulk_create()
say     "Write this $LOG entry in the do_ function in zerver/actions/ that makes the change, so the change, its audit log and its event commit together."
allow   calling a do_ function; reading RealmAuditLog with filter(); send_event_on_commit named in a comment; the same calls in zerver/actions/
misses  a view that writes a model with .save() and sends no event; helpers in zerver/lib that send events
```

From the architecture doc: [Zulip directory structure](https://zulip.readthedocs.io/en/latest/subsystems/directory-structure.html).

<details>
<summary>Doc quote and full rule: copy into <code>draft.json</code></summary>

On `zerver/actions/`: “we have a policy that all code calling `send_event_on_commit` to trigger pushing data to clients must live here.”

```json draft
{
  "kind": "code",
  "rule": {
    "format": "rerail.policy/v1",
    "id": "local:views-change-state-through-actions",
    "revision": 1,
    "mode": "steer",
    "description": "Keep client events and audit-log writes out of Zulip views; a do_ function in zerver/actions makes the change, logs it and notifies clients.",
    "scope": { "include": ["zerver/views/**/*.py"], "exclude": ["zerver/views/development/**"] },
    "clauses": [
      {
        "id": "event-from-view",
        "forbid": {
          "ast": {
            "language": "python",
            "rule": {
              "kind": "call",
              "has": {
                "field": "function",
                "pattern": "$SEND",
                "any": [{ "kind": "identifier" }, { "kind": "attribute" }],
                "resolves": {
                  "import": [
                    "zerver.tornado.django_api.send_event_on_commit",
                    "zerver.tornado.django_api.send_event_rollback_unsafe"
                  ]
                }
              }
            }
          }
        },
        "violation": {
          "code": "EVENT_FROM_VIEW",
          "anchor": "witness.span",
          "message": "Move this change and its $SEND call into a do_ function in zerver/actions/ and call that from the view; views validate input, actions write and notify."
        }
      },
      {
        "id": "audit-log-from-view",
        "forbid": {
          "ast": {
            "language": "python",
            "rule": {
              "kind": "call",
              "has": {
                "field": "function",
                "kind": "attribute",
                "regex": "\\.objects\\.(?:create|bulk_create)$",
                "has": {
                  "field": "object",
                  "kind": "attribute",
                  "has": {
                    "field": "object",
                    "pattern": "$LOG",
                    "resolves": { "import": ["zerver.models.RealmAuditLog", "zerver.models.realm_audit_logs.RealmAuditLog"] }
                  }
                }
              }
            }
          }
        },
        "violation": {
          "code": "AUDIT_LOG_FROM_VIEW",
          "anchor": "witness.span",
          "message": "Write this $LOG entry in the do_ function in zerver/actions/ that makes the change, so the change, its audit log and its event commit together."
        }
      }
    ]
  },
  "cases": {
    "caught": [
      { "path": "zerver/views/drafts.py", "content": "from django.http import HttpRequest, HttpResponse\n\nfrom zerver.lib.drafts import access_draft\nfrom zerver.lib.response import json_success\nfrom zerver.models import UserProfile\nfrom zerver.tornado.django_api import send_event_on_commit\n\n\ndef delete_draft(\n    request: HttpRequest, user_profile: UserProfile, draft_id: int\n) -> HttpResponse:\n    draft = access_draft(user_profile, draft_id)\n    draft.delete()\n    event = {\"type\": \"drafts\", \"op\": \"remove\", \"draft_id\": draft_id}\n    send_event_on_commit(user_profile.realm, event, [user_profile.id])\n    return json_success(request)\n" },
      { "path": "zerver/views/realm_linkifiers.py", "content": "from django.utils.timezone import now as timezone_now\n\nfrom zerver.lib.response import json_success\nfrom zerver.models import RealmAuditLog, RealmFilter\nfrom zerver.models.realm_audit_logs import AuditLogEventType\nfrom zerver.tornado import django_api\n\n\ndef delete_linkifier(request, user_profile, filter_id: int):\n    RealmFilter.objects.filter(realm=user_profile.realm, id=filter_id).delete()\n    RealmAuditLog.objects.create(\n        realm=user_profile.realm,\n        acting_user=user_profile,\n        event_type=AuditLogEventType.REALM_LINKIFIER_REMOVED,\n        event_time=timezone_now(),\n    )\n    django_api.send_event_on_commit(user_profile.realm, {\"type\": \"realm_linkifiers\"}, [])\n    return json_success(request)\n" },
      { "path": "zerver/views/user_settings.py", "content": "from zerver.tornado.django_api import send_event_rollback_unsafe as send_event\n\n\ndef mark_tour_seen(request, user_profile):\n    user_profile.tutorial_status = \"F\"\n    user_profile.save(update_fields=[\"tutorial_status\"])\n    send_event(user_profile.realm, {\"type\": \"onboarding_steps\"}, [user_profile.id])\n" }
    ],
    "passed": [
      { "path": "zerver/views/drafts.py", "content": "from django.http import HttpRequest, HttpResponse\n\nfrom zerver.actions.drafts import do_delete_draft\nfrom zerver.lib.response import json_success\nfrom zerver.models import UserProfile\n\n\ndef delete_draft(request: HttpRequest, user_profile: UserProfile, draft_id: int) -> HttpResponse:\n    do_delete_draft(draft_id, user_profile)\n    return json_success(request)\n" },
      { "path": "zerver/views/registration.py", "content": "from zerver.models import RealmAuditLog\nfrom zerver.models.realm_audit_logs import AuditLogEventType\n\n\ndef password_was_changed(importing_user) -> bool:\n    return RealmAuditLog.objects.filter(\n        modified_user=importing_user, event_type=AuditLogEventType.USER_PASSWORD_CHANGED\n    ).exists()\n" },
      { "path": "zerver/views/events_register.py", "content": "from zerver.lib.events import do_events_register\nfrom zerver.lib.response import json_success\n\n\ndef events_register_backend(request, user_profile, event_types=None):\n    # Clients receive what send_event_on_commit() pushes through this queue.\n    ret = do_events_register(user_profile, request.client, event_types=event_types)\n    return json_success(request, data=ret)\n" },
      { "path": "zerver/actions/drafts.py", "content": "from zerver.models import Draft, UserProfile\nfrom zerver.tornado.django_api import send_event_on_commit\n\n\ndef do_delete_draft(draft_id: int, user_profile: UserProfile) -> None:\n    Draft.objects.filter(user_profile=user_profile, id=draft_id).delete()\n    event = {\"type\": \"drafts\", \"op\": \"remove\", \"draft_id\": draft_id}\n    send_event_on_commit(user_profile.realm, event, [user_profile.id])\n" }
    ]
  }
}
```

</details>

## TypeScript and Vue

Rules for layer boundaries, package imports and dependency injection in TypeScript and Vue codebases.

### Immich (NestJS): services reach the filesystem through repositories

Immich keeps technology in `src/repositories` and business logic in `src/services`, so service tests mock each repository. A service calling `node:fs` needs a real disk to test, and the storage repository is no longer the one place the server touches files.

A service that resets a file's modified time calls `node:fs` itself:

`server/src/services/asset-media.service.ts`

```ts
import { Injectable } from '@nestjs/common';
import { utimes } from 'node:fs/promises';
import { BaseService } from 'src/services/base.service';
import { UploadFile } from 'src/types';

@Injectable()
export class AssetMediaService extends BaseService {
  private async restoreModifiedAt(file: UploadFile, modifiedAt: Date) {
    await utimes(file.originalPath, new Date(), modifiedAt);
  }
}
```

```rule
rule    local:services-go-through-repositories                                steer
files   server/src/services/**/*.ts, except specs
forbid  a call to a function from fs, fs/promises, node:fs or node:fs/promises, under any import
        name or namespace (utimes, fs.rm)
say     "Services don't call fs directly: call this.storageRepository instead of $FN, adding a StorageRepository method if none fits, so service tests can mock it."
forbid  a call to a function from child_process or node:child_process, under any import name
say     "Services don't start processes directly: call this.processRepository (spawn, spawnDuplexStream) instead of $FN, adding a ProcessRepository method if none fits."
allow   fs types and constants (Stats, R_OK); node:path; a local function named readFile;
        this.storageRepository.utimes(); the repositories themselves
misses  fs reached through another module, such as a util that wraps it; require(); it also flags
        the readFileSync of index.html that api.service.ts has today
```

From review: [immich-app/immich#7010](https://github.com/immich-app/immich/pull/7010#discussion_r1484909502) and the [architecture doc](https://github.com/immich-app/immich/blob/main/docs/docs/developer/architecture.mdx).

<details>
<summary>Review quotes and full rule: copy into <code>draft.json</code></summary>

A regex can't tell the fs function `utimes(...)` from a repository method of that name; the rule follows each name to
its import.

The review: “We don't call fs methods directly in services. Can you just add this to the storage repository and use that here instead?” The architecture doc: “we aim to separate technology specific implementations (`src/repositories`) from core business logic (`src/services`).”

```json draft
{
  "kind": "code",
  "rule": {
    "format": "rerail.policy/v1",
    "id": "local:services-go-through-repositories",
    "revision": 1,
    "mode": "steer",
    "description": "Reach the filesystem and child processes from server services through the storage and process repositories, not node:fs or node:child_process; services stay testable with mocks.",
    "scope": { "include": ["server/src/services/**/*.ts"], "exclude": ["**/*.spec.ts"] },
    "clauses": [
      {
        "id": "fs-call",
        "forbid": {
          "ast": {
            "language": "typescript",
            "utils": {
              "from-fs": {
                "kind": "identifier",
                "resolves": {
                  "binding": {
                    "site": {
                      "inside": {
                        "kind": "import_statement",
                        "has": { "field": "source", "regex": "^['\"](?:node:)?fs(?:/promises)?['\"]$" },
                        "stopBy": "end"
                      }
                    }
                  }
                }
              }
            },
            "rule": {
              "kind": "call_expression",
              "has": {
                "field": "function",
                "pattern": "$FN",
                "any": [
                  { "kind": "identifier", "matches": "from-fs" },
                  { "kind": "member_expression", "has": { "field": "object", "kind": "identifier", "matches": "from-fs" } }
                ]
              }
            }
          }
        },
        "violation": {
          "code": "FS_CALL_IN_SERVICE",
          "anchor": "witness.span",
          "message": "Services don't call fs directly: call this.storageRepository instead of $FN, adding a StorageRepository method if none fits, so service tests can mock it."
        }
      },
      {
        "id": "child-process-call",
        "forbid": {
          "ast": {
            "language": "typescript",
            "utils": {
              "from-cp": {
                "kind": "identifier",
                "resolves": {
                  "binding": {
                    "site": {
                      "inside": {
                        "kind": "import_statement",
                        "has": { "field": "source", "regex": "^['\"](?:node:)?child_process['\"]$" },
                        "stopBy": "end"
                      }
                    }
                  }
                }
              }
            },
            "rule": {
              "kind": "call_expression",
              "has": {
                "field": "function",
                "pattern": "$FN",
                "any": [
                  { "kind": "identifier", "matches": "from-cp" },
                  { "kind": "member_expression", "has": { "field": "object", "kind": "identifier", "matches": "from-cp" } }
                ]
              }
            }
          }
        },
        "violation": {
          "code": "PROCESS_CALL_IN_SERVICE",
          "anchor": "witness.span",
          "message": "Services don't start processes directly: call this.processRepository (spawn, spawnDuplexStream) instead of $FN, adding a ProcessRepository method if none fits."
        }
      }
    ]
  },
  "cases": {
    "caught": [
      { "path": "server/src/services/asset-media.service.ts", "content": "import { Injectable } from '@nestjs/common';\nimport { utimes } from 'node:fs/promises';\nimport { BaseService } from 'src/services/base.service';\nimport { UploadFile } from 'src/types';\n\n@Injectable()\nexport class AssetMediaService extends BaseService {\n  private async restoreModifiedAt(file: UploadFile, modifiedAt: Date) {\n    await utimes(file.originalPath, new Date(), modifiedAt);\n  }\n}\n" },
      { "path": "server/src/services/video-frame.service.ts", "content": "import { Injectable } from '@nestjs/common';\nimport * as fs from 'node:fs/promises';\nimport { tmpdir } from 'node:os';\nimport { join } from 'node:path';\nimport { BaseService } from 'src/services/base.service';\n\n@Injectable()\nexport class VideoFrameService extends BaseService {\n  async extract(assetId: string) {\n    const tempDir = await fs.mkdtemp(join(tmpdir(), `immich-video-frames-${assetId}-`));\n    try {\n      return await this.mediaRepository.extractFrames(assetId, tempDir);\n    } finally {\n      await fs.rm(tempDir, { recursive: true, force: true });\n    }\n  }\n}\n" },
      { "path": "server/src/services/media.service.ts", "content": "import { Injectable } from '@nestjs/common';\nimport { spawn as run } from 'node:child_process';\nimport { BaseService } from 'src/services/base.service';\n\n@Injectable()\nexport class MediaService extends BaseService {\n  probeHardware() {\n    return run('vainfo', ['--display', 'drm']);\n  }\n}\n" }
    ],
    "passed": [
      { "path": "server/src/services/asset-media.service.ts", "content": "import { Injectable } from '@nestjs/common';\nimport { BaseService } from 'src/services/base.service';\nimport { UploadFile } from 'src/types';\n\n@Injectable()\nexport class AssetMediaService extends BaseService {\n  private async restoreModifiedAt(file: UploadFile, modifiedAt: Date) {\n    await this.storageRepository.utimes(file.originalPath, new Date(), modifiedAt);\n  }\n}\n" },
      { "path": "server/src/services/library.service.ts", "content": "import { Injectable } from '@nestjs/common';\nimport { R_OK } from 'node:constants';\nimport { Stats } from 'node:fs';\nimport path from 'node:path';\nimport { BaseService } from 'src/services/base.service';\n\n@Injectable()\nexport class LibraryService extends BaseService {\n  async isReadable(file: string, stat: Stats) {\n    return stat.isFile() && path.isAbsolute(file) && (await this.storageRepository.checkFileExists(file, R_OK));\n  }\n}\n" },
      { "path": "server/src/services/media.service.ts", "content": "import { Injectable } from '@nestjs/common';\nimport { BaseService } from 'src/services/base.service';\n\n@Injectable()\nexport class MediaService extends BaseService {\n  async remove(paths: string[]) {\n    const readFile = (file: string) => this.storageRepository.readFile(file);\n    await Promise.all(paths.map((file) => readFile(file)));\n    return this.processRepository.spawn('vainfo', ['--display', 'drm']);\n  }\n}\n" },
      { "path": "server/src/repositories/storage.repository.ts", "content": "import { Injectable } from '@nestjs/common';\nimport fs from 'node:fs/promises';\n\n@Injectable()\nexport class StorageRepository {\n  utimes(filepath: string, atime: Date, mtime: Date) {\n    return fs.utimes(filepath, atime, mtime);\n  }\n}\n" }
    ]
  }
}
```

</details>

### n8n (Vue): frontend modules import only the shared packages

An n8n frontend module (`packages/modules/<name>/frontend`) may import only the shared L0–L2 packages, not `@/…` (the
editor shell that loads it) or sibling modules. A shell import couples the module to shell internals, yet type checks
and builds pass once the dependency is declared. Asked to show a workflow's name in the inbox, an agent imports the shell's store:

`packages/modules/inbox/frontend/src/components/InboxHeader.vue`

```vue
<script setup lang="ts">
import { computed } from 'vue';
import { useWorkflowsStore } from '@/app/stores/workflows.store';
import { useInboxStore } from '../inbox.store';

const inboxStore = useInboxStore();
const workflowsStore = useWorkflowsStore();
const title = computed(
	() => workflowsStore.workflowsById[inboxStore.activeWorkflowId]?.name,
);
</script>

<template>
	<h2>{{ title }}</h2>
</template>
```

```rule
rule    local:frontend-modules-stay-in-bounds                                 steer
files   packages/modules/*/frontend/src/** (.ts, .vue), except tests
forbid  in a .vue component's script: importing "@/…" or "@n8n/frontend-module-…" other than the SDK,
        with import, export … from or import()
say     "A frontend module can't import $MODULE: modules use L0–L2 packages only. Import a subpath such as @n8n/stores/settings.store, or reach the shell through a capability."
forbid  the same imports in a .ts file
say     "A frontend module can't import $MODULE: modules use L0–L2 packages only. Import a subpath such as @n8n/stores/settings.store, or reach the shell through a capability."
forbid  in a *.module.ts descriptor: a use…() call at the top level of the file
say     "$CALL() at the top of a descriptor runs when main.ts imports it, before the shell installs Pinia: call it inside a route guard, a handler or setup."
allow   @n8n/stores/… and the other shared packages; @n8n/frontend-module-sdk; "@/…" in template
        text; use…() inside a guard, a handler or a store's setup; the shell's own files
misses  a sibling module listed in package.json but never imported; a store read at the top of a
        file the descriptor imports
```

From the architecture doc: [n8n-io/n8n frontend-module-guide.md](https://github.com/n8n-io/n8n/blob/master/packages/%40n8n/module-cli/frontend-module-guide.md#imports-and-boundaries).

<details>
<summary>Doc quotes and full rule: copy into <code>draft.json</code></summary>

The guide: “Never import another `@n8n/frontend-module-*`. Never import `@/…`, because `@/…` is the shell.” and “Until that rule lands, the boundary is the responsibility of the reviewer.”

```json draft
{
  "kind": "code",
  "rule": {
    "format": "rerail.policy/v1",
    "id": "local:frontend-modules-stay-in-bounds",
    "revision": 1,
    "mode": "steer",
    "description": "Keep n8n frontend modules on the shared packages: no imports of the editor shell or of another module, and no store reads at the top of a descriptor.",
    "scope": {
      "include": ["packages/modules/*/frontend/src/**/*.ts", "packages/modules/*/frontend/src/**/*.vue"],
      "exclude": ["**/*.test.ts", "**/__tests__/**"]
    },
    "clauses": [
      {
        "id": "shell-import-vue",
        "forbid": {
          "ast": {
            "language": "vue",
            "embedded": "script-block",
            "rule": {
              "kind": "string",
              "pattern": "$MODULE",
              "regex": "^['\"](?:@/|@n8n/frontend-module-)",
              "not": { "regex": "^['\"]@n8n/frontend-module-sdk(?:/|['\"])" },
              "any": [
                { "inside": { "field": "source", "any": [{ "kind": "import_statement" }, { "kind": "export_statement" }] } },
                {
                  "inside": {
                    "kind": "arguments",
                    "inside": { "kind": "call_expression", "has": { "field": "function", "kind": "import" } }
                  }
                }
              ]
            }
          }
        },
        "violation": {
          "code": "MODULE_IMPORTS_SHELL",
          "anchor": "witness.span",
          "message": "A frontend module can't import $MODULE: modules use L0–L2 packages only. Import a subpath such as @n8n/stores/settings.store, or reach the shell through a capability."
        }
      },
      {
        "id": "shell-import-ts",
        "forbid": {
          "ast": {
            "language": "typescript",
            "rule": {
              "kind": "string",
              "pattern": "$MODULE",
              "regex": "^['\"](?:@/|@n8n/frontend-module-)",
              "not": { "regex": "^['\"]@n8n/frontend-module-sdk(?:/|['\"])" },
              "any": [
                { "inside": { "field": "source", "any": [{ "kind": "import_statement" }, { "kind": "export_statement" }] } },
                {
                  "inside": {
                    "kind": "arguments",
                    "inside": { "kind": "call_expression", "has": { "field": "function", "kind": "import" } }
                  }
                }
              ]
            }
          }
        },
        "violation": {
          "code": "MODULE_IMPORTS_SHELL",
          "anchor": "witness.span",
          "message": "A frontend module can't import $MODULE: modules use L0–L2 packages only. Import a subpath such as @n8n/stores/settings.store, or reach the shell through a capability."
        }
      },
      {
        "id": "store-at-descriptor-top",
        "forbid": {
          "ast": {
            "language": "typescript",
            "file": { "path": { "regex": "\\.module\\.ts$" } },
            "rule": {
              "kind": "call_expression",
              "has": { "field": "function", "kind": "identifier", "regex": "^use[A-Z]", "pattern": "$CALL" },
              "not": {
                "inside": {
                  "any": [
                    { "kind": "arrow_function" },
                    { "kind": "function_expression" },
                    { "kind": "function_declaration" },
                    { "kind": "method_definition" },
                    { "kind": "generator_function_declaration" }
                  ],
                  "stopBy": "end"
                }
              }
            }
          }
        },
        "violation": {
          "code": "STORE_AT_DESCRIPTOR_TOP",
          "anchor": "witness.span",
          "message": "$CALL() at the top of a descriptor runs when main.ts imports it, before the shell installs Pinia: call it inside a route guard, a handler or setup."
        }
      }
    ]
  },
  "cases": {
    "caught": [
      { "path": "packages/modules/inbox/frontend/src/components/InboxHeader.vue", "content": "<script setup lang=\"ts\">\nimport { computed } from 'vue';\nimport { useWorkflowsStore } from '@/app/stores/workflows.store';\nimport { useInboxStore } from '../inbox.store';\n\nconst inboxStore = useInboxStore();\nconst workflowsStore = useWorkflowsStore();\nconst title = computed(\n\t() => workflowsStore.workflowsById[inboxStore.activeWorkflowId]?.name,\n);\n</script>\n\n<template>\n\t<h2>{{ title }}</h2>\n</template>\n" },
      { "path": "packages/modules/apps/frontend/src/composables/useAppInstances.ts", "content": "import { computed } from 'vue';\nimport { useInstanceRegistryStore } from '@n8n/frontend-module-instance-registry';\n\nexport function useAppInstances() {\n\tconst registry = useInstanceRegistryStore();\n\treturn computed(() => registry.instances.filter((instance) => instance.hasApps));\n}\n" },
      { "path": "packages/modules/otel/frontend/src/otel.routes.ts", "content": "import type { RouteRecordRaw } from 'vue-router';\n\nexport const otelRoutes: RouteRecordRaw[] = [\n\t{\n\t\tname: 'otel-settings',\n\t\tpath: 'otel',\n\t\tcomponent: async () => await import('@/features/settings/views/SettingsOtelView.vue'),\n\t},\n];\n" },
      { "path": "packages/modules/otel/frontend/src/otel.module.ts", "content": "import { defineFrontendModule } from '@n8n/frontend-module-sdk';\nimport { useSettingsStore } from '@n8n/stores/settings.store';\nimport { OTEL_SETTINGS_VIEW } from './otel.constants';\n\nconst settingsStore = useSettingsStore();\nconst OtelSettingsView = async () => await import('./views/OtelSettingsView.vue');\n\nexport const OtelModule = defineFrontendModule({\n\tid: 'otel',\n\tname: 'OpenTelemetry',\n\troutes: [\n\t\t{\n\t\t\tname: OTEL_SETTINGS_VIEW,\n\t\t\tpath: 'otel',\n\t\t\tcomponent: OtelSettingsView,\n\t\t\tbeforeEnter: () => settingsStore.isModuleActive('otel'),\n\t\t},\n\t],\n});\n" }
    ],
    "passed": [
      { "path": "packages/modules/inbox/frontend/src/components/InboxHeader.vue", "content": "<script setup lang=\"ts\">\nimport { computed } from 'vue';\nimport { useI18n } from '@n8n/i18n';\nimport { useInboxStore } from '../inbox.store';\n\nconst i18n = useI18n();\nconst inboxStore = useInboxStore();\nconst title = computed(() => inboxStore.activeWorkflowName ?? i18n.baseText('generic.unknown'));\n</script>\n\n<template>\n\t<h2>{{ title }}</h2>\n\t<p>See @/app/stores in the editor for the shell's stores.</p>\n</template>\n" },
      { "path": "packages/modules/otel/frontend/src/otel.module.ts", "content": "import { defineFrontendModule } from '@n8n/frontend-module-sdk';\nimport { useSettingsStore } from '@n8n/stores/settings.store';\nimport { OTEL_SETTINGS_VIEW } from './otel.constants';\n\nconst OtelSettingsView = async () => await import('./views/OtelSettingsView.vue');\n\nexport const OtelModule = defineFrontendModule({\n\tid: 'otel',\n\tname: 'OpenTelemetry',\n\troutes: [\n\t\t{\n\t\t\tname: OTEL_SETTINGS_VIEW,\n\t\t\tpath: 'otel',\n\t\t\tcomponent: OtelSettingsView,\n\t\t\tbeforeEnter: () => useSettingsStore().isModuleActive('otel'),\n\t\t},\n\t],\n});\n" },
      { "path": "packages/modules/otel/frontend/src/otel.store.ts", "content": "import { defineStore } from 'pinia';\nimport { ref } from 'vue';\nimport { capabilityRegistry, capabilities } from '@n8n/frontend-module-sdk';\nimport { useRootStore } from '@n8n/stores/useRootStore';\nimport { getOtelSettings } from './otel.api';\n\nexport const useOtelStore = defineStore('otel', () => {\n\tconst rootStore = useRootStore();\n\tconst endpoint = ref('');\n\tconst load = async () => {\n\t\tendpoint.value = (await getOtelSettings(rootStore.restApiContext)).endpoint;\n\t};\n\tconst openSettings = () => capabilityRegistry.use(capabilities.openSettings)('otel');\n\treturn { endpoint, load, openSettings };\n});\n" },
      { "path": "packages/frontend/editor-ui/src/app/modules.manifest.ts", "content": "import { InstanceRegistryModule } from '@n8n/frontend-module-instance-registry';\nimport { useUIStore } from '@/app/stores/ui.store';\n\nexport const modules = [InstanceRegistryModule];\nexport const ui = () => useUIStore();\n" }
    ]
  }
}
```

</details>

### VS Code: services come in through the constructor

VS Code builds its service graph from constructor parameters. `@IEditorService` tells the instantiation service what a class needs, so it creates dependencies first and reports cycles. Holding `IInstantiationService` and later calling `invokeFunction(accessor => accessor.get(...))` hides the dependency and creates the service when that line first runs.

A chat widget that opens an editor resolves the editor service when it runs:

`src/vs/workbench/contrib/chat/browser/attachments/chatAttachmentWidget.ts`

```ts
import { URI } from '../../../../../base/common/uri.js';
import {
	IInstantiationService,
} from '../../../../../platform/instantiation/common/instantiation.js';
import { IEditorService } from '../../../../services/editor/common/editorService.js';

export class ChatAttachmentWidget {
	constructor(
		private readonly resource: URI,
		@IInstantiationService
		private readonly instantiationService: IInstantiationService,
	) { }

	open(): void {
		const editorService = this.instantiationService.invokeFunction(
			accessor => accessor.get(IEditorService),
		);
		editorService.openEditor({ resource: this.resource });
	}
}
```

```rule
rule    local:inject-services-in-constructors                                 steer
files   src/vs/workbench/{contrib,services}, src/vs/sessions/{contrib,services}, src/vs/editor/contrib
        (.ts), except test folders
forbid  inside a class: X.get(IService) where X is the parameter of a callback passed to
        invokeFunction(…), under any parameter name
say     "Inject $SVC as a constructor parameter (@$SVC) instead of resolving it through invokeFunction; if that makes a cycle, pass it in from the code that wires this class."
allow   constructor injection; accessor.get() in an Action2's run(accessor) or a registered
        command handler; createInstance(); an invokeFunction callback that reads no service
misses  invokeFunction(helper, …) with a named helper that calls accessor.get() itself; it also
        flags calls on main that predate the guideline, such as mcpRegistry.ts
```

From the architecture docs: [microsoft/vscode coding guidelines](https://github.com/microsoft/vscode/blob/main/.github/instructions/coding-guidelines.instructions.md#L66) and the [Source Code Organization wiki](https://github.com/microsoft/vscode/wiki/Source-Code-Organization#dependency-injection).

<details>
<summary>Doc quotes and full rule: copy into <code>draft.json</code></summary>

A regex for `accessor.get(` would also flag every action's `run(accessor)`, where it is the documented way in; the rule
looks only inside callbacks passed to `invokeFunction`.

The coding guidelines: “Declare service dependencies in constructors via DI — never access services through `IInstantiationService` elsewhere.” The wiki: “Services get to its clients via `constructor injection`.”

```json draft
{
  "kind": "code",
  "rule": {
    "format": "rerail.policy/v1",
    "id": "local:inject-services-in-constructors",
    "revision": 1,
    "mode": "steer",
    "description": "Declare the services a VS Code class needs as constructor parameters, not by resolving them later through the instantiation service's invokeFunction.",
    "scope": {
      "include": [
        "src/vs/workbench/contrib/**/*.ts",
        "src/vs/workbench/services/**/*.ts",
        "src/vs/sessions/contrib/**/*.ts",
        "src/vs/sessions/services/**/*.ts",
        "src/vs/editor/contrib/**/*.ts"
      ],
      "exclude": ["**/test/**"]
    },
    "clauses": [
      {
        "id": "service-from-invoke-function",
        "forbid": {
          "ast": {
            "language": "typescript",
            "utils": {
              "invoke-callback": {
                "any": [{ "kind": "arrow_function" }, { "kind": "function_expression" }],
                "inside": {
                  "kind": "arguments",
                  "inside": {
                    "kind": "call_expression",
                    "has": { "field": "function", "kind": "member_expression", "has": { "field": "property", "regex": "^invokeFunction$" } }
                  }
                }
              }
            },
            "rule": {
              "kind": "call_expression",
              "all": [
                {
                  "has": {
                    "field": "function",
                    "kind": "member_expression",
                    "all": [
                      { "has": { "field": "property", "regex": "^get$" } },
                      {
                        "has": {
                          "field": "object",
                          "kind": "identifier",
                          "resolves": {
                            "binding": {
                              "site": {
                                "inside": {
                                  "matches": "invoke-callback",
                                  "stopBy": {
                                    "any": [
                                      { "kind": "arrow_function" },
                                      { "kind": "function_expression" },
                                      { "kind": "function_declaration" },
                                      { "kind": "method_definition" },
                                      { "kind": "generator_function_declaration" }
                                    ]
                                  }
                                }
                              }
                            }
                          }
                        }
                      }
                    ]
                  }
                },
                { "has": { "field": "arguments", "has": { "kind": "identifier", "pattern": "$SVC" } } }
              ],
              "inside": { "kind": "class_body", "stopBy": "end" }
            }
          }
        },
        "violation": {
          "code": "SERVICE_FROM_INVOKE_FUNCTION",
          "anchor": "witness.span",
          "message": "Inject $SVC as a constructor parameter (@$SVC) instead of resolving it through invokeFunction; if that makes a cycle, pass it in from the code that wires this class."
        }
      }
    ]
  },
  "cases": {
    "caught": [
      { "path": "src/vs/workbench/contrib/chat/browser/attachments/chatAttachmentWidget.ts", "content": "import { URI } from '../../../../../base/common/uri.js';\nimport {\n\tIInstantiationService,\n} from '../../../../../platform/instantiation/common/instantiation.js';\nimport { IEditorService } from '../../../../services/editor/common/editorService.js';\n\nexport class ChatAttachmentWidget {\n\tconstructor(\n\t\tprivate readonly resource: URI,\n\t\t@IInstantiationService\n\t\tprivate readonly instantiationService: IInstantiationService,\n\t) { }\n\n\topen(): void {\n\t\tconst editorService = this.instantiationService.invokeFunction(\n\t\t\taccessor => accessor.get(IEditorService),\n\t\t);\n\t\teditorService.openEditor({ resource: this.resource });\n\t}\n}\n" },
      { "path": "src/vs/sessions/services/configuration/browser/configurationService.ts", "content": "import { IInstantiationService, ServicesAccessor } from '../../../../platform/instantiation/common/instantiation.js';\nimport { INotificationService, Severity } from '../../../../platform/notification/common/notification.js';\nimport { IEditorService } from '../../../../workbench/services/editor/common/editorService.js';\n\nexport class SessionsConfigurationService {\n\tconstructor(@IInstantiationService private readonly instantiationService: IInstantiationService) { }\n\n\tprivate reportWriteError(error: Error, settingsResource: URI): void {\n\t\tthis.instantiationService.invokeFunction((accessor: ServicesAccessor) => {\n\t\t\tconst notificationService = accessor.get(INotificationService);\n\t\t\tconst editorService = accessor.get(IEditorService);\n\t\t\tnotificationService.prompt(Severity.Error, error.message, [{\n\t\t\t\tlabel: 'Open Settings',\n\t\t\t\trun: () => editorService.openEditor({ resource: settingsResource }),\n\t\t\t}]);\n\t\t});\n\t}\n}\n" },
      { "path": "src/vs/editor/contrib/hover/browser/contentHoverController.ts", "content": "export class ContentHoverController {\n\tconstructor(@IInstantiationService private readonly _instantiationService: IInstantiationService) { }\n\n\tprivate get _keybindingService(): IKeybindingService {\n\t\treturn this._instantiationService.invokeFunction(a => a.get(IKeybindingService));\n\t}\n}\n" }
    ],
    "passed": [
      { "path": "src/vs/workbench/contrib/chat/browser/attachments/chatAttachmentWidget.ts", "content": "import { URI } from '../../../../../base/common/uri.js';\nimport { IEditorService } from '../../../../services/editor/common/editorService.js';\n\nexport class ChatAttachmentWidget {\n\tconstructor(\n\t\tprivate readonly resource: URI,\n\t\t@IEditorService private readonly editorService: IEditorService,\n\t) { }\n\n\topen(): void {\n\t\tthis.editorService.openEditor({ resource: this.resource });\n\t}\n}\n" },
      { "path": "src/vs/workbench/contrib/chat/browser/actions/chatOpenAttachmentAction.ts", "content": "import { Action2 } from '../../../../../platform/actions/common/actions.js';\nimport { CommandsRegistry } from '../../../../../platform/commands/common/commands.js';\nimport { ServicesAccessor } from '../../../../../platform/instantiation/common/instantiation.js';\nimport { IEditorService } from '../../../../services/editor/common/editorService.js';\n\nexport class OpenAttachmentAction extends Action2 {\n\tasync run(accessor: ServicesAccessor, resource: URI): Promise<void> {\n\t\tconst editorService = accessor.get(IEditorService);\n\t\tawait editorService.openEditor({ resource });\n\t}\n}\n\nCommandsRegistry.registerCommand('chat.openAttachment', (accessor, resource: URI) => accessor.get(IEditorService).openEditor({ resource }));\n" },
      { "path": "src/vs/sessions/services/configuration/browser/configurationService.ts", "content": "import { IInstantiationService } from '../../../../platform/instantiation/common/instantiation.js';\nimport { ConfigurationWriteErrorReporter } from './configurationWriteErrorReporter.js';\n\nexport class SessionsConfigurationService {\n\tprivate errorReporter: ConfigurationWriteErrorReporter | undefined;\n\n\tconstructor(@IInstantiationService private readonly instantiationService: IInstantiationService) { }\n\n\tprivate reportWriteError(error: Error, settingsResource: URI): void {\n\t\tthis.errorReporter ??= this.instantiationService.createInstance(ConfigurationWriteErrorReporter);\n\t\tthis.errorReporter.report(error, settingsResource);\n\t}\n\n\tprivate lookup(id: string): string | undefined {\n\t\treturn this.instantiationService.invokeFunction(() => this.cache.get(id));\n\t}\n}\n" },
      { "path": "src/vs/workbench/contrib/chat/test/browser/chatAttachmentWidget.test.ts", "content": "suite('ChatAttachmentWidget', () => {\n\ttest('opens the resource', () => {\n\t\tconst editorService = instantiationService.invokeFunction(accessor => accessor.get(IEditorService));\n\t\tassert.ok(editorService);\n\t});\n});\n" }
    ]
  }
}
```

</details>

## Rust

Rules that keep IO, file access and environment-variable names in the one place each project allows.

### rust-analyzer: the ide and hir crates do no IO

rust-analyzer's `ide`, `hir` and `base-db` crates compute answers only from inputs the server has loaded, so analysis
can't fail on the outside world and tests stay in-process. Only the `rust-analyzer` crate, which talks to the editor,
touches files, processes and the network. Asked for an SVG of the crate graph, an agent runs `dot` where the graph is
built:

`crates/ide/src/view_crate_graph.rs`

```rust
use std::{fs, process::Command};

use ide_db::RootDatabase;

pub(crate) fn view_crate_graph(db: &RootDatabase) -> Result<String, String> {
    let path = std::env::temp_dir().join("crate-graph.dot");
    fs::write(&path, render_dot(db)).map_err(|e| e.to_string())?;
    let output = Command::new("dot")
        .arg("-Tsvg")
        .arg(&path)
        .output()
        .map_err(|e| e.to_string())?;
    String::from_utf8(output.stdout).map_err(|e| e.to_string())
}
```

```rule
rule    local:no-io-in-ide-and-hir                                            steer
files   crates/ide*, hir* and base-db sources, except tests and #[cfg(test)] modules
forbid  a call into std::fs, std::process::Command, std::net or toolchain::command, or to
        std::env::current_dir, set_current_dir or temp_dir, through any use or alias
say     "ide, hir and base-db do no IO: move $CALL into the rust-analyzer crate, which owns files, processes and the network, and pass its result in or return data for it to use."
allow   returning the .dot text for the rust-analyzer crate to render; the same call in
        crates/rust-analyzer; #[cfg(test)] modules; a local mod process; env::var_os dev flags
misses  methods on a Child or File passed in; calls written inside macro arguments
```

From review: [rust-lang/rust-analyzer#8801](https://github.com/rust-lang/rust-analyzer/pull/8801#discussion_r630570615), its follow-up [#8807](https://github.com/rust-lang/rust-analyzer/pull/8807) and the [architecture doc](https://github.com/rust-lang/rust-analyzer/blob/master/docs/book/src/contributing/architecture.md#error-handling).

<details>
<summary>Review quotes and full rule: copy into <code>draft.json</code></summary>

The review: “It seems that the layering is slightly off here, we generally don’t do IO, including IPC, in the ide layer and below.” The follow-up moved the `dot` call into the rust-analyzer crate. The architecture doc: “Only parts touching LSP are allowed to do IO.”

```json draft
{
  "kind": "code",
  "rule": {
    "format": "rerail.policy/v1",
    "id": "local:no-io-in-ide-and-hir",
    "revision": 1,
    "mode": "steer",
    "description": "Keep file, process and network access out of rust-analyzer's ide, hir and base-db crates; only the rust-analyzer crate, which talks to the editor, does IO.",
    "scope": {
      "include": [
        "crates/ide/src/**/*.rs",
        "crates/ide-*/src/**/*.rs",
        "crates/hir/src/**/*.rs",
        "crates/hir-*/src/**/*.rs",
        "crates/base-db/src/**/*.rs"
      ],
      "exclude": ["**/tests.rs", "**/tests/**", "**/test_data/**"]
    },
    "clauses": [
      {
        "id": "io-call",
        "forbid": {
          "ast": {
            "language": "rust",
            "rule": {
              "kind": "call_expression",
              "has": {
                "field": "function",
                "any": [{ "kind": "scoped_identifier" }, { "kind": "identifier" }],
                "pattern": "$CALL",
                "resolves": {
                  "import": [
                    "std::fs",
                    "std::process::Command",
                    "std::net",
                    "std::env::current_dir",
                    "std::env::set_current_dir",
                    "std::env::temp_dir",
                    "toolchain::command"
                  ],
                  "prefix": true
                }
              },
              "not": {
                "inside": {
                  "kind": "mod_item",
                  "follows": { "kind": "attribute_item", "regex": "^#\\[cfg\\(test\\)\\]$" },
                  "stopBy": "end"
                }
              }
            }
          }
        },
        "violation": {
          "code": "IO_IN_IDE_LAYER",
          "anchor": "witness.span",
          "message": "ide, hir and base-db do no IO: move $CALL into the rust-analyzer crate, which owns files, processes and the network, and pass its result in or return data for it to use."
        }
      }
    ]
  },
  "cases": {
    "caught": [
      { "path": "crates/ide/src/view_crate_graph.rs", "content": "use std::{fs, process::Command};\n\nuse ide_db::RootDatabase;\n\npub(crate) fn view_crate_graph(db: &RootDatabase) -> Result<String, String> {\n    let path = std::env::temp_dir().join(\"crate-graph.dot\");\n    fs::write(&path, render_dot(db)).map_err(|e| e.to_string())?;\n    let output = Command::new(\"dot\")\n        .arg(\"-Tsvg\")\n        .arg(&path)\n        .output()\n        .map_err(|e| e.to_string())?;\n    String::from_utf8(output.stdout).map_err(|e| e.to_string())\n}\n" },
      { "path": "crates/hir-expand/src/builtin/fn_macro.rs", "content": "fn include_str_expand(\n    db: &dyn SourceDatabase,\n    arg_id: MacroCallId,\n    tt: &tt::TopSubtree,\n    call_site: Span,\n) -> ExpandResult<tt::TopSubtree> {\n    let (path, input_span) = match parse_string(tt, false) {\n        Ok(it) => it,\n        Err(e) => return ExpandResult::new(tt::TopSubtree::empty(call_site.into()), e),\n    };\n    let text = match relative_file(db, arg_id, path.as_str(), true, input_span) {\n        Ok(file_id) => db.file_text(file_id.file_id(db)).text(db).to_string(),\n        // Excluded files aren't loaded: read them from disk instead.\n        Err(_) => std::fs::read_to_string(path.as_str()).unwrap_or_default(),\n    };\n    ExpandResult::ok(quote!(call_site =>#text))\n}\n" },
      { "path": "crates/ide/src/expand_macro.rs", "content": "use std::{io::Write, process::Stdio};\n\nuse ide_db::{FxHashMap, RootDatabase, base_db::Crate};\nuse toolchain::{Tool, command as tool_command};\n\nfn format_expansion(db: &RootDatabase, krate: Crate, expansion: &str) -> Option<String> {\n    let mut rustfmt = tool_command(Tool::Rustfmt.path(), \".\", &FxHashMap::default());\n    rustfmt.arg(\"--edition\").arg(krate.data(db).edition.to_string());\n    let mut child = rustfmt.stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().ok()?;\n    child.stdin.take()?.write_all(expansion.as_bytes()).ok()?;\n    String::from_utf8(child.wait_with_output().ok()?.stdout).ok()\n}\n" }
    ],
    "passed": [
      { "path": "crates/ide/src/view_crate_graph.rs", "content": "use dot::{Id, LabelText};\nuse ide_db::{RootDatabase, base_db::all_crates};\n\n// Returns the crate graph in .dot format; the client renders it.\npub(crate) fn view_crate_graph(db: &RootDatabase, full: bool) -> String {\n    let graph = DotCrateGraph::new(db, all_crates(db), full);\n    let mut dot = Vec::new();\n    dot::render(&graph, &mut dot).unwrap();\n    String::from_utf8(dot).unwrap()\n}\n" },
      { "path": "crates/ide/src/hover/render.rs", "content": "use std::env;\n\nuse hir::HirDisplay;\n\npub(super) fn closure_ty(sema: &Semantics<'_, RootDatabase>, closure: hir::Closure) -> Option<String> {\n    let mut markup = closure.display_with_impl(sema.db, DisplayTarget::default()).to_string();\n    if env::var_os(\"RA_DEV\").is_some() {\n        format_to!(markup, \"\\n{:?}\", closure);\n    }\n    Some(markup)\n}\n" },
      { "path": "crates/rust-analyzer/src/handlers/request.rs", "content": "pub(crate) fn handle_view_crate_graph(\n    snap: GlobalStateSnapshot,\n    params: ViewCrateGraphParams,\n) -> anyhow::Result<String> {\n    let dot = snap.analysis.view_crate_graph(params.full)?;\n    let output = toolchain::command(\"dot\", snap.config.root_path(), &FxHashMap::default())\n        .arg(\"-Tsvg\")\n        .stdin(std::process::Stdio::piped())\n        .output()?;\n    let _ = dot;\n    Ok(String::from_utf8(output.stdout)?)\n}\n" },
      { "path": "crates/ide/src/file_structure.rs", "content": "pub(crate) fn file_structure(file: &SourceFile) -> Vec<StructureNode> {\n    structure(file.syntax())\n}\n\n#[cfg(test)]\nmod tests {\n    use std::fs;\n\n    #[test]\n    fn structure_of_big_file() {\n        let text = fs::read_to_string(\"test_data/big.rs\").unwrap();\n        let file = SourceFile::parse(&text, Edition::CURRENT).tree();\n        assert!(!super::file_structure(&file).is_empty());\n    }\n}\n" },
      { "path": "crates/ide/src/highlight_related.rs", "content": "mod process {\n    pub(super) fn exit_points(body: &ast::BlockExpr) -> Vec<TextRange> {\n        body.syntax().descendants().filter_map(ast::ReturnExpr::cast).map(|it| it.syntax().text_range()).collect()\n    }\n}\n\npub(crate) fn highlight_exit_points(body: &ast::BlockExpr) -> Vec<TextRange> {\n    process::exit_points(body)\n}\n" }
    ]
  }
}
```

</details>

### Zed: project files go through Fs, not std::fs

Zed edits local, SSH and collaboration projects, and tests against an in-memory `FakeFs`, because file access goes
through the `Fs` trait and the `Worktree`. A `std::fs` call reads the local disk on whatever thread it runs on, does
nothing useful for a remote project, and escapes `FakeFs` in tests. Asked to paste an image into a Markdown file, an
agent writes it straight to disk:

`crates/editor/src/clipboard.rs`

```rust
use std::fs;

use gpui::{Context, Image, Window};

impl Editor {
    fn paste_image(
        &mut self,
        image: &Image,
        dir: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        let file_name = format!("image.{}", image.format.extension());
        fs::write(dir.join(&file_name), image.bytes())?;
        self.insert(&format!("![]({file_name})"), window, cx);
        Ok(())
    }
}
```

```rule
rule    local:project-files-through-fs                                        steer
files   crates/editor, project, workspace, sidebar, terminal_view and *_ui sources, except tests
forbid  a call into std::fs or smol::fs through any use or alias, outside #[cfg(test)] modules,
        unless an argument is a path from Zed's paths crate
say     "Replace $CALL with the project's Fs or a Worktree method, so this works on remote projects and on FakeFs in tests. Keep it only for files no project owns, like Zed's own data."
allow   Fs and Worktree methods; Zed's own fs crate (fs::RemoveOptions); std::fs in crates/fs;
        fs::create_dir_all(snippets_dir()) and other paths:: directories; test modules
misses  Path::exists() and other std::path methods that touch the disk; a paths:: directory
        stored in a variable first. Still matches files outside any project, such as /proc;
        the message tells the agent to keep those
```

From review: [zed-industries/zed#58588](https://github.com/zed-industries/zed/pull/58588#discussion_r3558128265).

<details>
<summary>Review quote and full rule: copy into <code>draft.json</code></summary>

The review: “Could we go through `Worktree::create_entry` instead of `std::fs::write`? That would avoid blocking here and make this work on remote projects.”

```json draft
{
  "kind": "code",
  "rule": {
    "format": "rerail.policy/v1",
    "id": "local:project-files-through-fs",
    "revision": 1,
    "mode": "steer",
    "description": "Read and write files in Zed's editor, project, workspace and UI crates through the Fs trait or the Worktree, not std::fs or smol::fs; direct calls skip remote projects and FakeFs.",
    "scope": {
      "include": [
        "crates/editor/src/**/*.rs",
        "crates/project/src/**/*.rs",
        "crates/workspace/src/**/*.rs",
        "crates/sidebar/src/**/*.rs",
        "crates/terminal_view/src/**/*.rs",
        "crates/*_ui/src/**/*.rs"
      ],
      "exclude": ["**/*_tests.rs", "**/tests/**", "**/src/test.rs"]
    },
    "clauses": [
      {
        "id": "direct-fs-call",
        "forbid": {
          "ast": {
            "language": "rust",
            "rule": {
              "kind": "call_expression",
              "has": {
                "field": "function",
                "any": [{ "kind": "scoped_identifier" }, { "kind": "identifier" }],
                "pattern": "$CALL",
                "resolves": { "import": ["std::fs", "smol::fs"], "prefix": true }
              },
              "not": {
                "any": [
                  {
                    "inside": {
                      "kind": "mod_item",
                      "follows": { "kind": "attribute_item", "regex": "^#\\[cfg\\(test\\)\\]$" },
                      "stopBy": "end"
                    }
                  },
                  {
                    "has": {
                      "field": "arguments",
                      "has": {
                        "any": [{ "kind": "scoped_identifier" }, { "kind": "identifier" }],
                        "resolves": { "import": ["paths"], "prefix": true },
                        "stopBy": "end"
                      }
                    }
                  }
                ]
              }
            }
          }
        },
        "violation": {
          "code": "DIRECT_FS_CALL",
          "anchor": "witness.span",
          "message": "Replace $CALL with the project's Fs or a Worktree method, so this works on remote projects and on FakeFs in tests. Keep it only for files no project owns, like Zed's own data."
        }
      }
    ]
  },
  "cases": {
    "caught": [
      { "path": "crates/editor/src/clipboard.rs", "content": "use std::fs;\n\nuse gpui::{Context, Image, Window};\n\nimpl Editor {\n    fn paste_image(\n        &mut self,\n        image: &Image,\n        dir: &Path,\n        window: &mut Window,\n        cx: &mut Context<Self>,\n    ) -> anyhow::Result<()> {\n        let file_name = format!(\"image.{}\", image.format.extension());\n        fs::write(dir.join(&file_name), image.bytes())?;\n        self.insert(&format!(\"![]({file_name})\"), window, cx);\n        Ok(())\n    }\n}\n" },
      { "path": "crates/project/src/task_inventory.rs", "content": "use smol::fs as async_fs;\n\npub(crate) async fn load_worktree_tasks(abs_path: PathBuf) -> anyhow::Result<TaskTemplates> {\n    let contents = async_fs::read_to_string(&abs_path).await.context(\"reading tasks.json\")?;\n    parse_json_with_comments(&contents)\n}\n" },
      { "path": "crates/agent_ui/src/thread_worktree_archive.rs", "content": "async fn remove_root(root: PathBuf, cx: &mut AsyncApp) -> anyhow::Result<()> {\n    cx.background_spawn(async move {\n        match std::fs::remove_dir_all(&root) {\n            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),\n            result => result.map_err(anyhow::Error::from),\n        }\n    })\n    .await\n}\n" }
    ],
    "passed": [
      { "path": "crates/editor/src/clipboard.rs", "content": "use gpui::{Context, Image, Window};\n\nimpl Editor {\n    fn paste_image(&mut self, image: &Image, worktree: Entity<Worktree>, window: &mut Window, cx: &mut Context<Self>) {\n        let path = RelPath::unix(&format!(\"image.{}\", image.format.extension())).unwrap().into_arc();\n        let create = worktree.update(cx, |worktree, cx| {\n            worktree.create_entry(path.clone(), false, Some(image.bytes().to_vec()), cx)\n        });\n        self.insert(&format!(\"![]({})\", path.as_unix_str()), window, cx);\n        create.detach_and_log_err(cx);\n    }\n}\n" },
      { "path": "crates/agent_ui/src/thread_worktree_archive.rs", "content": "use fs::{Fs, RemoveOptions};\n\nasync fn remove_root(fs: Arc<dyn Fs>, root: PathBuf) -> anyhow::Result<()> {\n    fs.remove_dir(&root, RemoveOptions { recursive: true, ignore_if_not_exists: true }).await\n}\n" },
      { "path": "crates/agent_ui/src/mention_set.rs", "content": "pub(crate) fn load_image(project: &Entity<Project>, path: Arc<Path>, cx: &mut App) -> Task<anyhow::Result<Vec<u8>>> {\n    let fs = project.read(cx).fs().clone();\n    cx.background_spawn(async move { fs.load_bytes(&path).await })\n}\n\n#[cfg(test)]\nmod tests {\n    use std::fs;\n\n    #[gpui::test]\n    async fn loads_png(cx: &mut TestAppContext) {\n        let dir = tempfile::tempdir().unwrap();\n        fs::write(dir.path().join(\"a.png\"), PNG_BYTES).unwrap();\n    }\n}\n" },
      { "path": "crates/snippets_ui/src/snippets_ui.rs", "content": "use std::{borrow::Borrow, fs};\n\nuse paths::snippets_dir;\n\nfn open_snippets_folder(workspace: &mut Workspace, cx: &mut Context<Workspace>) {\n    fs::create_dir_all(snippets_dir()).notify_err(workspace, cx);\n    cx.open_with_system(snippets_dir().borrow());\n}\n" },
      { "path": "crates/fs/src/fs.rs", "content": "#[async_trait::async_trait]\nimpl Fs for RealFs {\n    async fn remove_dir(&self, path: &Path, options: RemoveOptions) -> Result<()> {\n        let path = path.to_owned();\n        self.executor.spawn(async move { std::fs::remove_dir_all(&path) }).await?;\n        Ok(())\n    }\n}\n" }
    ]
  }
}
```

</details>

### uv: every environment variable is declared once, in EnvVars

uv declares every environment variable it reads as a constant on `EnvVars` in the `uv-static` crate, and generates its
environment-variable reference from that list. A string-literal name works but is missing from the reference, and a
typo fails silently. Adding a fallback region for S3 request signing, an agent spells the second variable inline:

`crates/uv-auth/src/providers.rs`

```rust
use std::env;
use std::sync::LazyLock;

use uv_static::EnvVars;

/// The AWS region used to sign S3 requests, if one is configured.
static S3_REGION: LazyLock<Option<String>> = LazyLock::new(|| {
    env::var(EnvVars::AWS_REGION)
        .or_else(|_| env::var("AWS_DEFAULT_REGION"))
        .ok()
});
```

```rule
rule    local:env-vars-through-envvars                                        steer
files   crates/**/*.rs, except uv-static, uv-bench, uv-test and build scripts
forbid  std::env::var, var_os, set_var or remove_var, through any use or alias, with a literal name
say     "Name $NAME through its uv_static::EnvVars constant, adding one with a doc comment if it's missing, so the variable is declared once and listed in the environment reference."
forbid  a clap #[arg(...)] or #[clap(...)] attribute whose env = is a string literal
say     "Write this argument's env as an EnvVars constant (env = EnvVars::UV_...), adding one in uv-static if it's missing, so the variable is declared and documented once."
allow   env::var(EnvVars::AWS_DEFAULT_REGION); a name held in a variable, as cache-keys reads it;
        "UV_CACHE_DIR" in a message; the constants in uv-static; env = EnvVars::UV_...
misses  Command::env("NAME", ...) and env_remove, which are methods; a name built with
        format! or passed in through a helper
```

From review: [astral-sh/uv#15925](https://github.com/astral-sh/uv/pull/15925#discussion_r2359013746) and the [`EnvVars` doc comment](https://github.com/astral-sh/uv/blob/main/crates/uv-static/src/env_vars.rs).

<details>
<summary>Review quotes and full rule: copy into <code>draft.json</code></summary>

The review: “Should be in `EnvVars`”, on `std::env::var("AWS_DEFAULT_REGION")`; the merge reads `EnvVars::AWS_DEFAULT_REGION`. The doc comment says `EnvVars` “Declares all environment variable used throughout `uv` and its crates.”

```json draft
{
  "kind": "code",
  "rule": {
    "format": "rerail.policy/v1",
    "id": "local:env-vars-through-envvars",
    "revision": 1,
    "mode": "steer",
    "description": "Name every environment variable uv reads or sets through a constant in uv-static's EnvVars, not a string literal; EnvVars is the one list the environment reference is generated from.",
    "scope": {
      "include": ["crates/**/*.rs"],
      "exclude": ["crates/uv-static/**", "crates/uv-bench/**", "crates/uv-test/**", "**/build.rs"]
    },
    "clauses": [
      {
        "id": "literal-env-call",
        "forbid": {
          "ast": {
            "language": "rust",
            "rule": {
              "kind": "call_expression",
              "has": {
                "field": "function",
                "any": [{ "kind": "scoped_identifier" }, { "kind": "identifier" }],
                "resolves": {
                  "import": ["std::env::var", "std::env::var_os", "std::env::set_var", "std::env::remove_var"]
                }
              },
              "all": [
                {
                  "has": {
                    "field": "arguments",
                    "has": { "kind": "string_literal", "nthChild": 1, "pattern": "$NAME" }
                  }
                }
              ]
            }
          }
        },
        "violation": {
          "code": "LITERAL_ENV_VAR",
          "anchor": "witness.span",
          "message": "Name $NAME through its uv_static::EnvVars constant, adding one with a doc comment if it's missing, so the variable is declared once and listed in the environment reference."
        }
      },
      {
        "id": "literal-clap-env",
        "forbid": {
          "ast": {
            "language": "rust",
            "rule": { "kind": "attribute_item", "regex": "^#\\[(?:arg|clap)\\((?s:.*)\\benv\\s*=\\s*\"" }
          }
        },
        "violation": {
          "code": "LITERAL_CLAP_ENV",
          "anchor": "witness.span",
          "message": "Write this argument's env as an EnvVars constant (env = EnvVars::UV_...), adding one in uv-static if it's missing, so the variable is declared and documented once."
        }
      }
    ]
  },
  "cases": {
    "caught": [
      { "path": "crates/uv-auth/src/providers.rs", "content": "use std::env;\nuse std::sync::LazyLock;\n\nuse uv_static::EnvVars;\n\n/// The AWS region used to sign S3 requests, if one is configured.\nstatic S3_REGION: LazyLock<Option<String>> = LazyLock::new(|| {\n    env::var(EnvVars::AWS_REGION)\n        .or_else(|_| env::var(\"AWS_DEFAULT_REGION\"))\n        .ok()\n});\n" },
      { "path": "crates/uv/src/printer.rs", "content": "use std::env::var_os as env_var;\n\npub fn progress_enabled(printer: Printer) -> bool {\n    if env_var(\"UV_NO_PROGRESS\").is_some() {\n        return false;\n    }\n    !matches!(printer, Printer::Quiet | Printer::Silent)\n}\n" },
      { "path": "crates/uv-cli/src/lib.rs", "content": "#[derive(Args)]\npub struct PipCompileArgs {\n    /// The platform for which requirements should be resolved.\n    #[arg(long, env = \"UV_PYTHON_PLATFORM\")]\n    pub python_platform: Option<TargetTriple>,\n}\n" }
    ],
    "passed": [
      { "path": "crates/uv-auth/src/providers.rs", "content": "use std::env;\nuse std::sync::LazyLock;\n\nuse uv_static::EnvVars;\n\n/// The AWS region used to sign S3 requests, if one is configured.\nstatic S3_REGION: LazyLock<Option<String>> = LazyLock::new(|| {\n    env::var(EnvVars::AWS_REGION)\n        .or_else(|_| env::var(EnvVars::AWS_DEFAULT_REGION))\n        .ok()\n});\n" },
      { "path": "crates/uv-cache-info/src/cache_info.rs", "content": "/// Read the environment variables a `cache-keys` entry names, e.g. `{ env = \"MACOSX_DEPLOYMENT_TARGET\" }`.\nfn env_cache_key(key: &CacheKey) -> Option<String> {\n    match key {\n        CacheKey::Env { env } => std::env::var(env).ok(),\n        _ => None,\n    }\n}\n" },
      { "path": "crates/uv-cli/src/lib.rs", "content": "#[derive(Args)]\npub struct PipCompileArgs {\n    /// The platform for which requirements should be resolved.\n    #[arg(long, env = EnvVars::UV_PYTHON_PLATFORM)]\n    pub python_platform: Option<TargetTriple>,\n}\n" },
      { "path": "crates/uv-cache/src/lib.rs", "content": "pub fn init(cache_dir: Option<&Path>) -> Result<Cache, Error> {\n    if cache_dir.is_none() && std::env::var_os(EnvVars::XDG_CACHE_HOME).is_none() {\n        warn_user_once!(\"Set `UV_CACHE_DIR` or `XDG_CACHE_HOME` to choose where uv caches downloads\");\n    }\n    Cache::from_settings(false, cache_dir.map(Path::to_path_buf))\n}\n" },
      { "path": "crates/uv-static/src/env_vars.rs", "content": "impl EnvVars {\n    /// The default AWS region to use when signing S3 requests, if `AWS_REGION` is not set.\n    #[attr_added_in(\"0.8.21\")]\n    pub const AWS_DEFAULT_REGION: &'static str = \"AWS_DEFAULT_REGION\";\n}\n" }
    ]
  }
}
```

</details>

## PHP and SQL

Rules that keep apps on public APIs, settings in config files, and slow index builds out of startup.

### Nextcloud: apps use the public OCP API, never the server's private OC classes

Nextcloud apps run on whatever server version the admin installs. `OC\` classes are the server's private implementation and can change between releases, so an app using one can break on upgrade. The app store accepts only apps that use the public `OCP\` API.

Needing to check an uploaded file name, an agent finds the server's own helper:

`lib/Controller/ApiController.php`

```php
<?php

namespace OCA\Forms\Controller;

use OC\Files\Filesystem;
use OCP\AppFramework\OCSController;

class ApiController extends OCSController {
	public function uploadFiles(int $formId, string $name): void {
		if (Filesystem::isFileBlacklisted($name)) {
			throw new \InvalidArgumentException('Invalid file name');
		}
	}
}
```

```rule
rule    local:apps-use-public-api                                             steer
files   an app's lib/, templates/ and appinfo/ (.php), except tests and vendor
forbid  a class or name that resolves into OC\… or to \OC, OC_Util and the other legacy OC_ classes,
        through use, group use, alias or a fully qualified name
say     "$NAME is private server code (the OC namespace) that can change in any release: use the OCP equivalent, injected in classes, e.g. IFilenameValidator for Filesystem, OCP\Util for OC_Util."
allow   OCP\… (including \OCP\Util); the app's own OCA\Forms\…; other apps' published OCA\… APIs such as
        OCA\Circles; OC\… in comments and strings
misses  a private service fetched by string, such as $container->get('OC\Files\View')
```

From review: [nextcloud/forms#2040](https://github.com/nextcloud/forms/pull/2040#discussion_r1547820774) and the [app store rules](https://docs.nextcloud.com/server/stable/developer_manual/app_publishing_maintenance/publishing.html#be-technically-sound).

<details>
<summary>Review quotes and full rule: copy into <code>draft.json</code></summary>

The review: “`OC` is private and should not be used in apps. Only use `OCP` API.” The app store rules: “Apps can only use the public Nextcloud API.”

```json draft
{
  "kind": "code",
  "rule": {
    "format": "rerail.policy/v1",
    "id": "local:apps-use-public-api",
    "revision": 1,
    "mode": "steer",
    "description": "Use only Nextcloud's public OCP API in the app, never the server's private OC classes; private classes change without notice between releases.",
    "scope": { "include": ["lib/**/*.php", "templates/**/*.php", "appinfo/**/*.php"], "exclude": ["tests/**", "vendor/**"] },
    "clauses": [
      {
        "id": "private-server-class",
        "forbid": {
          "ast": {
            "language": "php",
            "rule": {
              "any": [{ "kind": "qualified_name" }, { "kind": "name", "not": { "inside": { "kind": "qualified_name" } } }],
              "pattern": "$NAME",
              "resolves": {
                "import": ["OC", "OC_App", "OC_Defaults", "OC_Helper", "OC_Hook", "OC_Template", "OC_User", "OC_Util"],
                "prefix": true
              }
            }
          }
        },
        "violation": {
          "code": "PRIVATE_SERVER_API",
          "anchor": "witness.span",
          "message": "$NAME is private server code (the OC namespace) that can change in any release: use the OCP equivalent, injected in classes, e.g. IFilenameValidator for Filesystem, OCP\\Util for OC_Util."
        }
      }
    ]
  },
  "cases": {
    "caught": [
      { "path": "lib/Controller/ApiController.php", "content": "<?php\n\nnamespace OCA\\Forms\\Controller;\n\nuse OC\\Files\\Filesystem;\nuse OCP\\AppFramework\\OCSController;\n\nclass ApiController extends OCSController {\n\tpublic function uploadFiles(int $formId, string $name): void {\n\t\tif (Filesystem::isFileBlacklisted($name)) {\n\t\t\tthrow new \\InvalidArgumentException('Invalid file name');\n\t\t}\n\t}\n}\n" },
      { "path": "lib/Service/ConfigService.php", "content": "<?php\n\nnamespace OCA\\Forms\\Service;\n\nuse OC\\{Files\\View, User\\Session as UserSession};\n\nclass ConfigService {\n\tpublic function __construct(private UserSession $session) {\n\t}\n\n\tpublic function getVersion(): string {\n\t\treturn \\OC_Util::getVersionString() . \\OC::$server->getSystemConfig()->getValue('instanceid');\n\t}\n}\n" },
      { "path": "templates/main.php", "content": "<?php\n\\OCP\\Util::addScript('forms', 'forms-main');\n?>\n<div id=\"content\" data-version=\"<?php p(OC_Util::getVersionString()); ?>\"></div>\n" }
    ],
    "passed": [
      { "path": "lib/Controller/ApiController.php", "content": "<?php\n\nnamespace OCA\\Forms\\Controller;\n\nuse OCP\\AppFramework\\OCSController;\nuse OCP\\Files\\IFilenameValidator;\nuse OCP\\IRequest;\n\nclass ApiController extends OCSController {\n\tpublic function __construct(string $appName, IRequest $request, private IFilenameValidator $filenameValidator) {\n\t\tparent::__construct($appName, $request);\n\t}\n\n\tpublic function uploadFiles(int $formId, string $name): void {\n\t\tif (!$this->filenameValidator->isFilenameValid($name)) {\n\t\t\tthrow new \\InvalidArgumentException('Invalid file name');\n\t\t}\n\t}\n}\n" },
      { "path": "lib/Service/CirclesService.php", "content": "<?php\n\nnamespace OCA\\Forms\\Service;\n\nuse OCA\\Circles\\CirclesManager;\nuse OCA\\Forms\\Db\\FormMapper;\nuse OCP\\IConfig;\n\n// CirclesManager is the API Circles publishes for other apps, unlike OC\\Files\\Filesystem.\nclass CirclesService {\n\tpublic function __construct(private CirclesManager $circles, private FormMapper $forms, private IConfig $config) {\n\t}\n}\n" },
      { "path": "templates/main.php", "content": "<?php\n\\OCP\\Util::addScript('forms', 'forms-main');\n?>\n<div id=\"content\" data-version=\"<?php p(implode('.', \\OCP\\Util::getVersion())); ?>\"></div>\n" },
      { "path": "tests/Unit/Controller/ApiControllerTest.php", "content": "<?php\n\nnamespace OCA\\Forms\\Tests\\Unit\\Controller;\n\nuse OC\\Files\\Filesystem;\n" }
    ]
  }
}
```

</details>

### Laravel: env() only in config files; read settings through config()

In production Laravel runs `php artisan config:cache` and stops loading `.env`. An `env()` call outside `config/` then sees only the server's real environment variables, usually `null`, though it works on a developer machine. Put the value in a `config/` file and read it with `config()`.

Adding a Patreon client, an agent reads the token where it uses it:

`app/Services/Patreon/PatreonApi.php`

```php
<?php

namespace App\Services\Patreon;

use Illuminate\Support\Facades\Http;

class PatreonApi
{
    public function campaigns(): array
    {
        return Http::withToken(env('PATREON_TOKEN'))
            ->get('https://www.patreon.com/api/oauth2/v2/campaigns')
            ->json('data');
    }
}
```

```rule
rule    local:env-only-in-config                                              steer
files   app/, routes/, database/ and resources/views/ (.php and .blade.php); config/ is out of scope
forbid  a call to the env() helper, written env() or \env()
say     "Read this setting with config('file.key') instead of $CALL(): once config is cached, .env isn't loaded and $CALL() returns null. Add the key to a file in config/, which may call env()."
forbid  a static call on Illuminate\Support\Env, under any alias
say     "$CLASS::$METHOD() reads .env, which isn't loaded once config is cached: read this setting with config('file.key'), adding the key to a file in config/ that calls env()."
forbid  env() in a Blade view, in {{ }} echoes and directives such as @if
say     "Use config('file.key') in this view instead of $CALL(): once config is cached, .env isn't loaded and $CALL() returns null. Add the key to a file in config/, which may call env()."
allow   config('services.…'); app()->environment(); $this->option('env'); another class named Env;
        env( in comments and strings; env() in config/
misses  getenv() and $_ENV, which lose .env the same way; it also flags a namespaced function of your own named env()
```

From review: [spatie/spatie.be#35](https://github.com/spatie/spatie.be/pull/35#discussion_r226855944) and the [Laravel docs](https://laravel.com/docs/12.x/configuration#configuration-caching).

<details>
<summary>Review quotes and full rule: copy into <code>draft.json</code></summary>

The review: “The `env` function may only be used in config files.” The Laravel docs, on configuration caching: “you should be sure that you are only calling the `env` function from within your configuration files”.

```json draft
{
  "kind": "code",
  "rule": {
    "format": "rerail.policy/v1",
    "id": "local:env-only-in-config",
    "revision": 1,
    "mode": "steer",
    "description": "Call env only in config files and read settings everywhere else through config; once config is cached, env returns null for values from .env.",
    "scope": {
      "include": ["app/**/*.php", "routes/**/*.php", "database/**/*.php", "resources/views/**/*.php"],
      "exclude": ["tests/**"]
    },
    "clauses": [
      {
        "id": "env-call",
        "forbid": {
          "ast": {
            "language": "php",
            "rule": {
              "kind": "function_call_expression",
              "has": {
                "field": "function",
                "pattern": "$CALL",
                "any": [{ "kind": "name", "regex": "^env$" }, { "resolves": { "import": ["env"] } }]
              }
            }
          }
        },
        "violation": {
          "code": "ENV_OUTSIDE_CONFIG",
          "anchor": "witness.span",
          "message": "Read this setting with config('file.key') instead of $CALL(): once config is cached, .env isn't loaded and $CALL() returns null. Add the key to a file in config/, which may call env()."
        }
      },
      {
        "id": "env-class",
        "forbid": {
          "ast": {
            "language": "php",
            "rule": {
              "kind": "scoped_call_expression",
              "all": [
                {
                  "has": { "field": "scope", "pattern": "$CLASS", "resolves": { "import": ["Illuminate\\Support\\Env"] } }
                },
                { "has": { "field": "name", "pattern": "$METHOD" } }
              ]
            }
          }
        },
        "violation": {
          "code": "ENV_CLASS_OUTSIDE_CONFIG",
          "anchor": "witness.span",
          "message": "$CLASS::$METHOD() reads .env, which isn't loaded once config is cached: read this setting with config('file.key'), adding the key to a file in config/ that calls env()."
        }
      },
      {
        "id": "env-in-view",
        "forbid": {
          "ast": {
            "language": "blade",
            "embedded": "compiled-view",
            "rule": {
              "kind": "function_call_expression",
              "has": {
                "field": "function",
                "pattern": "$CALL",
                "any": [{ "kind": "name", "regex": "^env$" }, { "resolves": { "import": ["env"] } }]
              }
            }
          }
        },
        "violation": {
          "code": "ENV_IN_VIEW",
          "anchor": "witness.span",
          "message": "Use config('file.key') in this view instead of $CALL(): once config is cached, .env isn't loaded and $CALL() returns null. Add the key to a file in config/, which may call env()."
        }
      }
    ]
  },
  "cases": {
    "caught": [
      { "path": "app/Services/Patreon/PatreonApi.php", "content": "<?php\n\nnamespace App\\Services\\Patreon;\n\nuse Illuminate\\Support\\Facades\\Http;\n\nclass PatreonApi\n{\n    public function campaigns(): array\n    {\n        return Http::withToken(env('PATREON_TOKEN'))\n            ->get('https://www.patreon.com/api/oauth2/v2/campaigns')\n            ->json('data');\n    }\n}\n" },
      { "path": "app/Providers/GitHubServiceProvider.php", "content": "<?php\n\nnamespace App\\Providers;\n\nuse Github\\Client;\nuse Illuminate\\Support\\Env as DotEnv;\nuse Illuminate\\Support\\ServiceProvider;\n\nclass GitHubServiceProvider extends ServiceProvider\n{\n    public function register(): void\n    {\n        $this->app->singleton(Client::class, function () {\n            $client = new Client();\n            $client->authenticate(DotEnv::get('GITHUB_TOKEN'), null, Client::AUTH_ACCESS_TOKEN);\n\n            return $client;\n        });\n    }\n}\n" },
      { "path": "routes/web.php", "content": "<?php\n\nuse Illuminate\\Support\\Facades\\Route;\n\nif (\\env('SHOW_HORIZON', false)) {\n    Route::view('/horizon-status', 'horizon.status');\n}\n" },
      { "path": "resources/views/front/layouts/app.blade.php", "content": "<head>\n    <title>{{ $title }}</title>\n    @if (env('FATHOM_SITE_ID'))\n        <script src=\"https://cdn.usefathom.com/script.js\" data-site=\"{{ env('FATHOM_SITE_ID') }}\" defer></script>\n    @endif\n</head>\n" }
    ],
    "passed": [
      { "path": "app/Services/Patreon/PatreonApi.php", "content": "<?php\n\nnamespace App\\Services\\Patreon;\n\nuse Illuminate\\Support\\Facades\\Http;\n\nclass PatreonApi\n{\n    public function campaigns(): array\n    {\n        return Http::withToken(config('services.patreon.token'))\n            ->get('https://www.patreon.com/api/oauth2/v2/campaigns')\n            ->json('data');\n    }\n}\n" },
      { "path": "app/Console/Commands/ImportPatreonPledgers.php", "content": "<?php\n\nnamespace App\\Console\\Commands;\n\nuse Illuminate\\Console\\Command;\n\nclass ImportPatreonPledgers extends Command\n{\n    protected $signature = 'import:patreon-pledgers {--env=}';\n\n    public function handle(): void\n    {\n        // Production only: never call env('PATREON_TOKEN') here.\n        if (! app()->environment('production') && $this->option('env') !== 'production') {\n            return;\n        }\n\n        $this->info('Importing pledgers from Patreon...');\n    }\n}\n" },
      { "path": "app/Support/Deploy.php", "content": "<?php\n\nnamespace App\\Support;\n\nuse App\\Enums\\Env;\n\nclass Deploy\n{\n    public function target(string $name): Env\n    {\n        return Env::from($name);\n    }\n}\n" },
      { "path": "resources/views/front/layouts/app.blade.php", "content": "<head>\n    <title>{{ $title }}</title>\n    @if (config('services.fathom.site_id'))\n        <script src=\"https://cdn.usefathom.com/script.js\" data-site=\"{{ config('services.fathom.site_id') }}\" defer></script>\n    @endif\n</head>\n" },
      { "path": "config/services.php", "content": "<?php\n\nreturn [\n    'patreon' => [\n        'token' => env('PATREON_TOKEN'),\n    ],\n];\n" }
    ]
  }
}
```

</details>

### Synapse (PostgreSQL): index an existing table in a background update, not a schema delta

Synapse applies schema deltas at startup and serves nothing until they finish. `CREATE INDEX` on a table that already holds a large server's rows keeps the homeserver down while the index builds; `DROP INDEX` takes a heavy table lock. A table the same delta creates is empty and safe to index.

Asked to speed up a query, an agent writes the obvious delta:

`synapse/storage/schema/main/delta/93/02_event_push_actions_thread_idx.sql`

```sql
--
-- This file is licensed under the Affero General Public License (AGPL) version 3.
--
-- Copyright (C) 2026 New Vector, Ltd
--

-- So we can count unread notifications per thread.
CREATE INDEX IF NOT EXISTS event_push_actions_thread_id
    ON event_push_actions(user_id, room_id, thread_id);
```

```rule
rule    local:index-existing-tables-in-background                             steer
files   synapse/storage/schema/*/delta/**/*.sql
forbid  CREATE INDEX on a table that no CREATE TABLE in the same file creates
say     "Create this index on $TABLE in a background update: insert a background_updates row here and register it with register_background_index_update. A delta blocks startup while it builds."
forbid  DROP INDEX in a delta
say     "Drop $INDEX in a background update, not a delta: on a large table it takes a heavy lock while startup waits. Pass it as replaces_index to register_background_index_update to swap indexes."
allow   indexes on a table the same file creates; INSERT INTO background_updates; ALTER TABLE … ADD COLUMN;
        full schema dumps
misses  .sql.postgres, .sql.sqlite and Python deltas; it flags an index on a table another delta file of the
        same version creates, so create a table and its indexes in one file
```

From the architecture doc: [docs/development/database_schema.md](https://github.com/element-hq/synapse/blob/develop/docs/development/database_schema.md#worked-examples-of-gradual-migrations) and review: [element-hq/synapse#18440](https://github.com/element-hq/synapse/pull/18440#discussion_r2089278208).

<details>
<summary>Review quotes and full rule: copy into <code>draft.json</code></summary>

The doc: “including adding columns, indices or `NOT NULL` constraints to non-empty tables — such a migration should be done as a background update”. The review: “for new tables, it's fine to create the index right now”.

```json draft
{
  "kind": "code",
  "rule": {
    "format": "rerail.policy/v1",
    "id": "local:index-existing-tables-in-background",
    "revision": 1,
    "mode": "steer",
    "description": "Create indexes on existing tables, and drop indexes, through background updates instead of schema delta files; a delta blocks Synapse's startup until it finishes.",
    "scope": { "include": ["synapse/storage/schema/*/delta/**/*.sql"], "exclude": [] },
    "clauses": [
      {
        "id": "create-index-on-existing-table",
        "forbid": {
          "ast": {
            "language": "sql",
            "rule": {
              "kind": "create_index",
              "has": { "kind": "object_reference", "has": { "field": "name", "pattern": "$TABLE" } },
              "not": {
                "inside": {
                  "kind": "program",
                  "stopBy": "end",
                  "has": {
                    "kind": "create_table",
                    "stopBy": "end",
                    "has": { "kind": "object_reference", "has": { "field": "name", "pattern": "$TABLE" } }
                  }
                }
              }
            }
          }
        },
        "violation": {
          "code": "INDEX_IN_DELTA",
          "anchor": "witness.span",
          "message": "Create this index on $TABLE in a background update: insert a background_updates row here and register it with register_background_index_update. A delta blocks startup while it builds."
        }
      },
      {
        "id": "drop-index",
        "forbid": {
          "ast": { "language": "sql", "rule": { "kind": "drop_index", "has": { "field": "name", "pattern": "$INDEX" } } }
        },
        "violation": {
          "code": "DROP_INDEX_IN_DELTA",
          "anchor": "witness.span",
          "message": "Drop $INDEX in a background update, not a delta: on a large table it takes a heavy lock while startup waits. Pass it as replaces_index to register_background_index_update to swap indexes."
        }
      }
    ]
  },
  "cases": {
    "caught": [
      { "path": "synapse/storage/schema/main/delta/93/02_event_push_actions_thread_idx.sql", "content": "--\n-- This file is licensed under the Affero General Public License (AGPL) version 3.\n--\n-- Copyright (C) 2026 New Vector, Ltd\n--\n\n-- So we can count unread notifications per thread.\nCREATE INDEX IF NOT EXISTS event_push_actions_thread_id\n    ON event_push_actions(user_id, room_id, thread_id);\n" },
      { "path": "synapse/storage/schema/main/delta/93/03_receipts_idx.sql", "content": "--\n-- This file is licensed under the Affero General Public License (AGPL) version 3.\n--\n-- Copyright (C) 2026 New Vector, Ltd\n--\n\nDROP INDEX IF EXISTS receipts_linearized_user;\nCREATE INDEX receipts_linearized_user ON receipts_linearized (user_id, room_id);\n" },
      { "path": "synapse/storage/schema/main/delta/93/04_sticky_events.sql", "content": "--\n-- This file is licensed under the Affero General Public License (AGPL) version 3.\n--\n-- Copyright (C) 2026 New Vector, Ltd\n--\n\nCREATE TABLE sticky_events (\n    room_id TEXT NOT NULL,\n    event_id TEXT NOT NULL,\n    expires_at BIGINT NOT NULL\n);\n\nCREATE INDEX sticky_events_expires_at ON sticky_events (expires_at);\nCREATE INDEX events_sticky_room ON events (room_id, stream_ordering);\n" }
    ],
    "passed": [
      { "path": "synapse/storage/schema/main/delta/93/02_event_push_actions_thread_idx.sql", "content": "--\n-- This file is licensed under the Affero General Public License (AGPL) version 3.\n--\n-- Copyright (C) 2026 New Vector, Ltd\n--\n\nINSERT INTO background_updates (ordering, update_name, progress_json) VALUES\n  (9302, 'event_push_actions_thread_id_index', '{}');\n" },
      { "path": "synapse/storage/schema/main/delta/93/05_delayed_events.sql", "content": "--\n-- This file is licensed under the Affero General Public License (AGPL) version 3.\n--\n-- Copyright (C) 2026 New Vector, Ltd\n--\n\nCREATE TABLE delayed_events (\n    delay_id TEXT NOT NULL,\n    user_localpart TEXT NOT NULL,\n    send_ts BIGINT NOT NULL,\n    PRIMARY KEY (user_localpart, delay_id)\n);\n\nCREATE INDEX delayed_events_send_ts ON delayed_events (send_ts);\n" },
      { "path": "synapse/storage/schema/main/delta/93/06_rooms_creator.sql", "content": "--\n-- This file is licensed under the Affero General Public License (AGPL) version 3.\n--\n-- Copyright (C) 2026 New Vector, Ltd\n--\n\n-- Nullable with no default: PostgreSQL adds it without rewriting the table.\nALTER TABLE rooms ADD COLUMN creator TEXT;\n" },
      { "path": "synapse/storage/schema/main/full_schemas/72/full.sql", "content": "CREATE INDEX x ON y (z);\n" }
    ]
  }
}
```

</details>

## React

Two rules for an app built from `<PanelFrame>` panels: one follows imports across files, one checks the enclosing function.

### Panels never nest

Every panel renders inside `<PanelFrame>`, which draws the title bar and close button and opens the analytics scope.
Nested frames draw two title bars and count the view twice. Asked to show the customer in the order panel, an agent
reuses `CustomerPanel`, which brings its own frame:

`src/panels/OrdersPanel.tsx`

```tsx
import { PanelFrame } from '@/ui/panel';
import { CustomerPanel } from '@/panels/CustomerPanel';

export function OrdersPanel({ order }: Props) {
  return (
    <PanelFrame title="Order">
      <CustomerPanel id={order.customerId} />
    </PanelFrame>
  );
}
```

```rule
rule    local:no-nested-panels                                                steer
files   src/**/*.tsx, except tests and stories
forbid  <PanelFrame> anywhere inside another <PanelFrame>
say     "Panels don't nest: render this content without its own PanelFrame, or open it as a separate panel next to this one."
forbid  in src/panels/: rendering <X /> where X is imported from "@/panels/…"
say     "$PANEL is a panel, and panels don't nest: open it next to this one from the page, or move the part you need out of src/panels into a shared component."
allow   frames side by side; panels side by side on a page; ordinary components inside a panel
misses  a panel imported by a relative path (./CustomerPanel)
```

<details>
<summary>How it works and full rule: copy into <code>draft.json</code></summary>

The rule never sees inside `CustomerPanel`. It treats it as a panel because the name is imported from `@/panels/`, so
the folder convention carries the nesting across files.

```json draft
{
  "kind": "code",
  "rule": {
    "format": "rerail.policy/v1",
    "id": "local:no-nested-panels",
    "revision": 1,
    "mode": "steer",
    "description": "Don't nest panels: no PanelFrame inside another, and no panel component rendered from inside a panel.",
    "scope": { "include": ["src/**/*.tsx"], "exclude": ["**/*.test.tsx", "**/*.stories.tsx"] },
    "clauses": [
      {
        "id": "frame-in-frame",
        "forbid": {
          "ast": {
            "language": "typescript",
            "utils": {
              "panel-frame": {
                "any": [
                  { "kind": "jsx_element", "has": { "field": "open_tag", "has": { "field": "name", "regex": "^PanelFrame$" } } },
                  { "kind": "jsx_self_closing_element", "has": { "field": "name", "regex": "^PanelFrame$" } }
                ]
              }
            },
            "rule": {
              "matches": "panel-frame",
              "inside": { "matches": "panel-frame", "stopBy": "end" }
            }
          }
        },
        "violation": {
          "code": "NESTED_PANEL_FRAME",
          "anchor": "witness.span",
          "message": "Panels don't nest: render this content without its own PanelFrame, or open it as a separate panel next to this one."
        }
      },
      {
        "id": "panel-in-panel",
        "forbid": {
          "ast": {
            "language": "typescript",
            "file": { "path": { "regex": "^src/panels/" } },
            "rule": {
              "any": [{ "kind": "jsx_opening_element" }, { "kind": "jsx_self_closing_element" }],
              "has": {
                "field": "name",
                "kind": "identifier",
                "pattern": "$PANEL",
                "resolves": {
                  "binding": {
                    "site": {
                      "inside": {
                        "kind": "import_statement",
                        "has": { "field": "source", "regex": "^['\"]@/panels/" },
                        "stopBy": "end"
                      }
                    }
                  }
                }
              }
            }
          }
        },
        "violation": {
          "code": "PANEL_IN_PANEL",
          "anchor": "witness.span",
          "message": "$PANEL is a panel, and panels don't nest: open it next to this one from the page, or move the part you need out of src/panels into a shared component."
        }
      }
    ]
  },
  "cases": {
    "caught": [
      { "path": "src/pages/Orders.tsx", "content": "import { PanelFrame } from '@/ui/panel';\n\nexport function Orders() {\n  return (\n    <PanelFrame title=\"Orders\">\n      <OrderList />\n      <PanelFrame title=\"Details\">\n        <OrderDetails />\n      </PanelFrame>\n    </PanelFrame>\n  );\n}\n" },
      { "path": "src/pages/Orders.tsx", "content": "import { PanelFrame } from '@/ui/panel';\n\nexport function Orders() {\n  return (\n    <PanelFrame title=\"Orders\">\n      {open && <PanelFrame title=\"Empty\" />}\n    </PanelFrame>\n  );\n}\n" },
      { "path": "src/panels/OrdersPanel.tsx", "content": "import { PanelFrame } from '@/ui/panel';\nimport { CustomerPanel } from '@/panels/CustomerPanel';\n\nexport function OrdersPanel({ order }: Props) {\n  return (\n    <PanelFrame title=\"Order\">\n      <CustomerPanel id={order.customerId} />\n    </PanelFrame>\n  );\n}\n" }
    ],
    "passed": [
      { "path": "src/pages/Orders.tsx", "content": "import { PanelFrame } from '@/ui/panel';\n\nexport function Orders() {\n  return (\n    <>\n      <PanelFrame title=\"Orders\"><OrderList /></PanelFrame>\n      <PanelFrame title=\"Details\"><OrderDetails /></PanelFrame>\n    </>\n  );\n}\n" },
      { "path": "src/pages/Orders.tsx", "content": "import { OrdersPanel } from '@/panels/OrdersPanel';\nimport { CustomerPanel } from '@/panels/CustomerPanel';\n\nexport function Orders() {\n  return (\n    <>\n      <OrdersPanel />\n      <CustomerPanel />\n    </>\n  );\n}\n" },
      { "path": "src/panels/OrdersPanel.tsx", "content": "import { PanelFrame } from '@/ui/panel';\nimport { CustomerSummary } from '@/components/CustomerSummary';\n\nfunction OrderLines({ lines }: { lines: Line[] }) {\n  return <ul>{lines.map(line => <li key={line.id}>{line.name}</li>)}</ul>;\n}\n\nexport function OrdersPanel({ order }: Props) {\n  return (\n    <PanelFrame title=\"Order\">\n      <CustomerSummary id={order.customerId} />\n      <OrderLines lines={order.lines} />\n    </PanelFrame>\n  );\n}\n" }
    ]
  }
}
```

</details>

### A hook that only works inside its wrapper

`usePanel()` reads the context `<PanelFrame>` provides, so it works only inside a frame. Called in the component that
renders the frame, it runs before the frame exists and gets no panel. Nothing fails until someone opens the panel:

`src/panels/OrdersPanel.tsx`

```tsx
import { PanelFrame, usePanel } from '@/ui/panel';

export function OrdersPanel() {
  const panel = usePanel();
  return (
    <PanelFrame title="Orders">
      <button onClick={panel.close}>Close</button>
    </PanelFrame>
  );
}
```

```rule
rule    local:use-panel-below-panel-frame                                     steer
files   src/**/*.ts, src/**/*.tsx, except tests, stories and src/ui/panel/
forbid  usePanel() whose nearest enclosing function renders <PanelFrame>
say     "usePanel() here runs above the PanelFrame this component renders, so it gets no panel: call it in a child component rendered inside the frame."
forbid  usePanel() in a file outside src/panels/
say     "Only components under src/panels render inside a PanelFrame: move this component there, or pass it what it needs from usePanel() as props."
allow   a child component in the same file that calls usePanel(); hooks under src/panels/ built on usePanel()
misses  a component under src/panels/ that some page renders without a frame
```

<details>
<summary>How it works and full rule: copy into <code>draft.json</code></summary>

"Nearest enclosing function" is `"stopBy": { "matches": "function" }`. The search for the frame stops at the first
function around the call, so a child component declared in the same file is not blamed for its parent.

Whether every page renders `src/panels/` components inside a frame needs the render tree, which no rule sees. Keep
`usePanel()` throwing when it finds no frame.

```json draft
{
  "kind": "code",
  "rule": {
    "format": "rerail.policy/v1",
    "id": "local:use-panel-below-panel-frame",
    "revision": 1,
    "mode": "steer",
    "description": "Call usePanel only in components rendered inside a PanelFrame, which live under src/panels, never in the component that renders the frame.",
    "scope": { "include": ["src/**/*.ts", "src/**/*.tsx"], "exclude": ["**/*.test.*", "**/*.stories.tsx", "src/ui/panel/**"] },
    "clauses": [
      {
        "id": "beside-its-frame",
        "forbid": {
          "ast": {
            "language": "typescript",
            "utils": {
              "function": {
                "any": [
                  { "kind": "function_declaration" },
                  { "kind": "function_expression" },
                  { "kind": "arrow_function" },
                  { "kind": "method_definition" }
                ]
              }
            },
            "rule": {
              "pattern": "usePanel($$$ARGS)",
              "inside": {
                "matches": "function",
                "has": {
                  "any": [{ "kind": "jsx_opening_element" }, { "kind": "jsx_self_closing_element" }],
                  "has": { "field": "name", "regex": "^PanelFrame$" },
                  "stopBy": "end"
                },
                "stopBy": { "matches": "function" }
              }
            }
          }
        },
        "violation": {
          "code": "USE_PANEL_BESIDE_FRAME",
          "anchor": "witness.span",
          "message": "usePanel() here runs above the PanelFrame this component renders, so it gets no panel: call it in a child component rendered inside the frame."
        }
      },
      {
        "id": "outside-panels",
        "forbid": {
          "ast": {
            "language": "typescript",
            "file": { "path": { "notRegex": "^src/panels/" } },
            "rule": { "pattern": "usePanel($$$ARGS)" }
          }
        },
        "violation": {
          "code": "USE_PANEL_OUTSIDE_PANELS",
          "anchor": "witness.span",
          "message": "Only components under src/panels render inside a PanelFrame: move this component there, or pass it what it needs from usePanel() as props."
        }
      }
    ]
  },
  "cases": {
    "caught": [
      { "path": "src/panels/OrdersPanel.tsx", "content": "import { PanelFrame, usePanel } from '@/ui/panel';\n\nexport function OrdersPanel() {\n  const panel = usePanel();\n  return (\n    <PanelFrame title=\"Orders\">\n      <button onClick={panel.close}>Close</button>\n    </PanelFrame>\n  );\n}\n" },
      { "path": "src/panels/OrdersPanel.tsx", "content": "import { PanelFrame, usePanel } from '@/ui/panel';\n\nexport const OrdersPanel = () => {\n  const { close } = usePanel();\n  return <PanelFrame title=\"Orders\" onEscape={close} />;\n};\n" },
      { "path": "src/components/CloseButton.tsx", "content": "import { usePanel } from '@/ui/panel';\n\nexport function CloseButton() {\n  const panel = usePanel();\n  return <button onClick={panel.close}>Close</button>;\n}\n" }
    ],
    "passed": [
      { "path": "src/panels/usePanelTitle.ts", "content": "import { usePanel } from '@/ui/panel';\n\nexport function usePanelTitle(): string {\n  return usePanel().title;\n}\n" },
      { "path": "src/panels/OrdersPanel.tsx", "content": "import { PanelFrame, usePanel } from '@/ui/panel';\n\nfunction CloseButton() {\n  const panel = usePanel();\n  return <button onClick={panel.close}>Close</button>;\n}\n\nexport function OrdersPanel() {\n  return (\n    <PanelFrame title=\"Orders\">\n      <CloseButton />\n    </PanelFrame>\n  );\n}\n" },
      { "path": "src/components/CloseButton.tsx", "content": "export function CloseButton({ onClose }: { onClose: () => void }) {\n  return <button onClick={onClose}>Close</button>;\n}\n" }
    ]
  }
}
```

</details>

## What a rule can and can't check

Each rule on this page reads one file at a time: its syntax tree, path and text.

<details>
<summary>What a rule can ask within one file</summary>

| Question | How | Example |
|---|---|---|
| Is this node inside that one, at any depth? | `inside` / `has` with `"stopBy": "end"` | Sentry: a control-silo query inside a `@cell_silo_endpoint` class; a `PanelFrame` inside a `PanelFrame` |
| Is it in the nearest enclosing function, not just some outer one? | `inside` with `"stopBy"` set to a rule | `usePanel()` in the component that renders the frame |
| Where was this name imported from, under any alias? | `resolves`: Python, PHP and Rust through the file's imports; TypeScript through the import that binds the name | Sentry's `User as SentryUser`, Immich's `spawn as run`, Nextcloud's `use OC\Files\Filesystem`, rust-analyzer's `use toolchain::command as tool_command` |
| Does a name here match one elsewhere in the file? | the same capture, such as `$TABLE`, in two places | Synapse: an index on a table the migration does not create |
| Which file is this? | `scope`, and a clause's own `file.path` condition | n8n: `.vue` and `.ts` files under a frontend module |

</details>

A rule cannot see into other files. With no type checker, call graph or render tree, it cannot tell which component
renders inside which, whether a called helper does IO, or a value's type. Hence each example's `misses` line.

A design rule becomes checkable when the code records the decision where one file shows it:

1. **In the same file:** the call sits lexically inside the class, function, block or element it must not be in.
2. **In the names:** the decision follows a module, package or crate (`OC\` versus `OCP\`, `std::fs` versus Zed's
   `fs`), which name resolution reads under any alias.
3. **In the layout:** a folder, file name or import path says which layer the file belongs to (`zerver/views/`,
   `crates/ide/`, `src/panels/`).

When none holds, keep the check where it is today (review, a runtime assertion) and write the rule for the part a file
shows. [The DSL reference](code-rule-dsl-reference.html) documents every construct these examples use.
