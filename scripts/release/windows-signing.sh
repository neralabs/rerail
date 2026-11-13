#!/usr/bin/env bash
#
# One-time Azure setup for signing the Windows engines in .github/workflows/package.yml, safe to run again: it creates
# only what is missing.
#
#   bash scripts/release/windows-signing.sh [--identity-validation-id <id>] [--github]
#
# Needs `az login` as someone who may create resource groups, app registrations and role assignments in the
# subscription, and `gh` signed in as an admin of RERAIL_GITHUB_REPOSITORY (default neralabs/rerail) for --github.
# Creates, in RERAIL_SIGNING_RESOURCE_GROUP (default rerail-signing-rg, RERAIL_SIGNING_LOCATION default eastus, a region
# Artifact Signing serves):
#   - the Artifact Signing account RERAIL_SIGNING_ACCOUNT (default rerailsigning, Basic tier);
#   - with --identity-validation-id, its Public Trust certificate profile RERAIL_SIGNING_PROFILE (default
#     rerail-public). The identity validation is made by a person in the Azure portal (Artifact Signing account >
#     Identity validations > Organization > Public), never here; the id is that validation's, once it is Completed;
#   - for GitHub Actions, the app registration rerail-release-signing-github, trusted through OIDC only for jobs of
#     the repository's `windows-signing` environment, with Artifact Signing Certificate Profile Signer on this
#     account only: it can sign with the account's profiles and do nothing else in Azure.
# --github creates the `windows-signing` environment, deployable from main and `v*` tags only, and writes the
# repository variables package.yml reads (ARTIFACT_SIGNING_CLIENT_ID, ARTIFACT_SIGNING_ENDPOINT,
# ARTIFACT_SIGNING_ACCOUNT, and once the profile exists ARTIFACT_SIGNING_PROFILE, which turns signing on). It
# reuses AZURE_TENANT_ID, which scripts/host/azure/bootstrap.sh writes for the same tenant.
set -euo pipefail

RG="${RERAIL_SIGNING_RESOURCE_GROUP:-rerail-signing-rg}"
LOCATION="${RERAIL_SIGNING_LOCATION:-eastus}"
ACCOUNT="${RERAIL_SIGNING_ACCOUNT:-rerailsigning}"
PROFILE="${RERAIL_SIGNING_PROFILE:-rerail-public}"
REPO="${RERAIL_GITHUB_REPOSITORY:-neralabs/rerail}"
APP_NAME="rerail-release-signing-github"
ENVIRONMENT="windows-signing"
PURPOSE="rerail-release-signing"
ROLE="Artifact Signing Certificate Profile Signer"
github=0
validation=""
while [ $# -gt 0 ]; do
  case "$1" in
    --github) github=1 ;;
    --identity-validation-id) validation="$2"; shift ;;
    *) echo "unknown option $1" >&2; exit 2 ;;
  esac
  shift
done

info() { printf '\033[34m==>\033[0m %s\n' "$*"; }

az extension add --name artifact-signing --allow-preview true --upgrade -y -o none --only-show-errors
subscription="$(az account show --query id -o tsv)"
tenant="$(az account show --query tenantId -o tsv)"
info "Subscription $subscription, resource group $RG in $LOCATION"

if [ "$(az provider show --namespace Microsoft.CodeSigning --query registrationState -o tsv)" != Registered ]; then
  info "Registering the Microsoft.CodeSigning resource provider"
  az provider register --namespace Microsoft.CodeSigning --wait -o none
fi
az group create -n "$RG" -l "$LOCATION" --tags "purpose=$PURPOSE" -o none

# The artifact-signing commands are a preview extension, which warns on every call; --only-show-errors keeps errors.
signing() { az artifact-signing "$@" --only-show-errors; }
if ! signing show -g "$RG" -n "$ACCOUNT" -o none 2>/dev/null; then
  info "Creating Artifact Signing account $ACCOUNT"
  signing create -g "$RG" -n "$ACCOUNT" -l "$LOCATION" --sku Basic --tags "purpose=$PURPOSE" -o none
fi
account_id="$(signing show -g "$RG" -n "$ACCOUNT" --query id -o tsv)"
endpoint="$(signing show -g "$RG" -n "$ACCOUNT" --query accountUri -o tsv)"
if [ -z "$endpoint" ]; then
  echo "Artifact Signing account $ACCOUNT reports no endpoint" >&2
  exit 1
fi
info "Artifact Signing account $ACCOUNT at $endpoint"

profile_ready=0
if signing certificate-profile show -g "$RG" --account-name "$ACCOUNT" -n "$PROFILE" -o none 2>/dev/null; then
  profile_ready=1
elif [ -n "$validation" ]; then
  info "Creating Public Trust certificate profile $PROFILE"
  signing certificate-profile create -g "$RG" --account-name "$ACCOUNT" -n "$PROFILE" \
    --profile-type PublicTrust --identity-validation-id "$validation" -o none
  profile_ready=1
fi
if [ "$profile_ready" = 1 ]; then
  info "Certificate profile $PROFILE: $(signing certificate-profile show -g "$RG" --account-name "$ACCOUNT" -n "$PROFILE" --query 'status || properties.status' -o tsv)"
else
  info "No certificate profile yet: complete the organization's Public identity validation in the portal, then rerun with --identity-validation-id <id>"
fi

app_id="$(az ad app list --display-name "$APP_NAME" --query '[0].appId' -o tsv)"
if [ -z "$app_id" ]; then
  info "Creating app registration $APP_NAME"
  app_id="$(az ad app create --display-name "$APP_NAME" --query appId -o tsv)"
fi
if [ -z "$(az ad sp list --filter "appId eq '$app_id'" --query '[0].id' -o tsv)" ]; then
  az ad sp create --id "$app_id" -o none
fi
principal="$(az ad sp list --filter "appId eq '$app_id'" --query '[0].id' -o tsv)"

# The subject GitHub puts in the repository's OIDC tokens: `repo:<owner>/<repo>`, or with immutable subjects on,
# `repo:<owner>@<owner id>/<repo>@<repo id>`; the repository reports which. A job that names an environment gets
# `<prefix>:environment:<name>`, whatever ref it runs on, so one trust covers main and every release tag, and the
# environment's deployment policy decides which refs may use it.
subject_prefix="$(gh api "repos/$REPO/actions/oidc/customization/sub" --jq 'if .use_default then (.sub_claim_prefix // empty) else "custom" end' 2>/dev/null || true)"
if [ "$subject_prefix" = custom ]; then
  echo "$REPO uses a custom OIDC subject template; trust its subject by hand" >&2
  exit 1
fi
subject="${subject_prefix:-repo:$REPO}:environment:$ENVIRONMENT"
current="$(az ad app federated-credential list --id "$app_id" --query "[?name=='$ENVIRONMENT'].subject | [0]" -o tsv)"
parameters="{\"name\":\"$ENVIRONMENT\",\"issuer\":\"https://token.actions.githubusercontent.com\",\"subject\":\"$subject\",\"audiences\":[\"api://AzureADTokenExchange\"]}"
if [ -z "$current" ]; then
  az ad app federated-credential create --id "$app_id" -o none --parameters "$parameters"
elif [ "$current" != "$subject" ]; then
  az ad app federated-credential update --id "$app_id" --federated-credential-id "$ENVIRONMENT" -o none --parameters "$parameters"
fi
info "OIDC trust $ENVIRONMENT: $subject"

if [ -z "$(az role assignment list --assignee "$principal" --scope "$account_id" --role "$ROLE" --query '[0].id' -o tsv)" ]; then
  # A new service principal can take a few seconds to reach every replica of the directory.
  for attempt in 1 2 3 4 5 6; do
    az role assignment create --assignee-object-id "$principal" --assignee-principal-type ServicePrincipal \
      --role "$ROLE" --scope "$account_id" -o none && break
    [ "$attempt" = 6 ] && exit 1
    sleep 10
  done
fi
info "$APP_NAME ($app_id) is $ROLE on $ACCOUNT only"

if [ "$github" = 1 ]; then
  gh api -X PUT "repos/$REPO/environments/$ENVIRONMENT" --input - >/dev/null <<'EOF'
{"deployment_branch_policy":{"protected_branches":false,"custom_branch_policies":true}}
EOF
  policies="$(gh api "repos/$REPO/environments/$ENVIRONMENT/deployment-branch-policies" --jq '.branch_policies[] | "\(.type):\(.name)"')"
  for policy in branch:main tag:v*; do
    if ! grep -qxF "$policy" <<<"$policies"; then
      gh api -X POST "repos/$REPO/environments/$ENVIRONMENT/deployment-branch-policies" \
        -f "type=${policy%%:*}" -f "name=${policy#*:}" >/dev/null
    fi
  done
  info "Environment $ENVIRONMENT deploys from main and v* tags only"
  if [ -z "$(gh variable get AZURE_TENANT_ID -R "$REPO" 2>/dev/null)" ]; then gh variable set AZURE_TENANT_ID -R "$REPO" -b "$tenant"; fi
  gh variable set ARTIFACT_SIGNING_CLIENT_ID -R "$REPO" -b "$app_id"
  gh variable set ARTIFACT_SIGNING_ENDPOINT -R "$REPO" -b "$endpoint"
  gh variable set ARTIFACT_SIGNING_ACCOUNT -R "$REPO" -b "$ACCOUNT"
  if [ "$profile_ready" = 1 ]; then
    gh variable set ARTIFACT_SIGNING_PROFILE -R "$REPO" -b "$PROFILE"
    info "Set the signing variables on $REPO: main and release tags now sign the Windows engines"
  else
    info "Set the signing variables on $REPO except ARTIFACT_SIGNING_PROFILE, so nothing signs until the profile exists"
  fi
else
  echo "Repository variables for .github/workflows/package.yml (or rerun with --github):"
  echo "  ARTIFACT_SIGNING_CLIENT_ID=$app_id ARTIFACT_SIGNING_ENDPOINT=$endpoint ARTIFACT_SIGNING_ACCOUNT=$ACCOUNT"
  if [ "$profile_ready" = 1 ]; then echo "  ARTIFACT_SIGNING_PROFILE=$PROFILE"; fi
fi
