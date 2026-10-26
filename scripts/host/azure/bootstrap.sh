#!/usr/bin/env bash
#
# One-time Azure setup for scripts/host/azure/matrix.mjs, safe to run again: it creates only what is missing.
#
#   bash scripts/host/azure/bootstrap.sh [--github] [--branch <name>] [--remove-branch <name>]
#
# Needs `az login` as someone who may create resource groups, app registrations and role assignments in the
# subscription. Creates, in RERAIL_AZURE_RESOURCE_GROUP (default rerail-envtest-rg, RERAIL_AZURE_LOCATION default
# canadacentral):
#   - a storage account whose blobs carry the package in and results out (deleted after 7 days);
#   - a virtual network whose subnet's security group admits nothing from outside (only Azure's default rules);
#   - for GitHub Actions, the app registration rerail-env-matrix-github, trusted through OIDC for pushes to main of
#     RERAIL_GITHUB_REPOSITORY (default neralabs/rerail), with Contributor on this resource group only: CI can make
#     and delete VMs here and nowhere else.
# --github writes the repository variables the workflow reads (AZURE_CLIENT_ID, AZURE_TENANT_ID,
# AZURE_SUBSCRIPTION_ID, RERAIL_AZURE_RESOURCE_GROUP) with `gh`. --branch also trusts pushes to one more branch, to try
# the workflow before it merges; --remove-branch takes that trust away again.
set -euo pipefail

RG="${RERAIL_AZURE_RESOURCE_GROUP:-rerail-envtest-rg}"
LOCATION="${RERAIL_AZURE_LOCATION:-canadacentral}"
REPO="${RERAIL_GITHUB_REPOSITORY:-neralabs/rerail}"
APP_NAME="rerail-env-matrix-github"
PURPOSE="rerail-env-matrix"
VNET="rerail-env-vnet"
NSG="rerail-env-nsg"
github=0
branch=""
remove_branch=""
while [ $# -gt 0 ]; do
  case "$1" in
    --github) github=1 ;;
    --branch) branch="$2"; shift ;;
    --remove-branch) remove_branch="$2"; shift ;;
    *) echo "unknown option $1" >&2; exit 2 ;;
  esac
  shift
done

info() { printf '\033[34m==>\033[0m %s\n' "$*"; }

subscription="$(az account show --query id -o tsv)"
tenant="$(az account show --query tenantId -o tsv)"
info "Subscription $subscription, resource group $RG in $LOCATION"
az group create -n "$RG" -l "$LOCATION" --tags "purpose=$PURPOSE" -o none

account="$(az storage account list -g "$RG" --query "[?tags.purpose=='$PURPOSE'].name | [0]" -o tsv)"
if [ -z "$account" ]; then
  account="rerailenv$(openssl rand -hex 3)"
  info "Creating storage account $account"
  az storage account create -n "$account" -g "$RG" -l "$LOCATION" --sku Standard_LRS --kind StorageV2 \
    --min-tls-version TLS1_2 --allow-blob-public-access false --tags "purpose=$PURPOSE" -o none
fi
az storage account management-policy create --account-name "$account" -g "$RG" -o none --policy \
  '{"rules":[{"enabled":true,"name":"expire-runs","type":"Lifecycle","definition":{"actions":{"baseBlob":{"delete":{"daysAfterModificationGreaterThan":7}}},"filters":{"blobTypes":["blockBlob"]}}}]}'
info "Storage account $account (blobs expire after 7 days)"

if ! az network nsg show -g "$RG" -n "$NSG" -o none 2>/dev/null; then
  az network nsg create -g "$RG" -n "$NSG" -l "$LOCATION" --tags "purpose=$PURPOSE" -o none
fi
if ! az network vnet show -g "$RG" -n "$VNET" -o none 2>/dev/null; then
  az network vnet create -g "$RG" -n "$VNET" -l "$LOCATION" --address-prefixes 10.42.0.0/16 \
    --subnet-name vms --subnet-prefixes 10.42.0.0/24 --network-security-group "$NSG" --tags "purpose=$PURPOSE" -o none
fi
info "Network $VNET/vms behind $NSG (no inbound rules)"

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
# `repo:<owner>@<owner id>/<repo>@<repo id>`; the repository reports which.
subject_prefix="$(gh api "repos/$REPO/actions/oidc/customization/sub" --jq 'if .use_default then (.sub_claim_prefix // empty) else "custom" end' 2>/dev/null || true)"
if [ "$subject_prefix" = custom ]; then
  echo "$REPO uses a custom OIDC subject template; trust its subject by hand" >&2
  exit 1
fi
subject_prefix="${subject_prefix:-repo:$REPO}"

credential() { # <name> <subject>
  local current
  current="$(az ad app federated-credential list --id "$app_id" --query "[?name=='$1'].subject | [0]" -o tsv)"
  local parameters="{\"name\":\"$1\",\"issuer\":\"https://token.actions.githubusercontent.com\",\"subject\":\"$2\",\"audiences\":[\"api://AzureADTokenExchange\"]}"
  if [ -z "$current" ]; then
    az ad app federated-credential create --id "$app_id" -o none --parameters "$parameters"
  elif [ "$current" != "$2" ]; then
    az ad app federated-credential update --id "$app_id" --federated-credential-id "$1" -o none --parameters "$parameters"
  fi
  info "OIDC trust $1: $2"
}
credential "main" "$subject_prefix:ref:refs/heads/main"
if [ -n "$branch" ]; then credential "branch-$(printf '%s' "$branch" | tr -c 'A-Za-z0-9-' '-')" "$subject_prefix:ref:refs/heads/$branch"; fi
if [ -n "$remove_branch" ]; then
  name="branch-$(printf '%s' "$remove_branch" | tr -c 'A-Za-z0-9-' '-')"
  az ad app federated-credential delete --id "$app_id" --federated-credential-id "$name" -o none 2>/dev/null || true
  info "Removed OIDC trust $name"
fi

scope="/subscriptions/$subscription/resourceGroups/$RG"
if [ -z "$(az role assignment list --assignee "$principal" --scope "$scope" --role Contributor --query '[0].id' -o tsv)" ]; then
  # A new service principal can take a few seconds to reach every replica of the directory.
  for attempt in 1 2 3 4 5 6; do
    az role assignment create --assignee-object-id "$principal" --assignee-principal-type ServicePrincipal \
      --role Contributor --scope "$scope" -o none && break
    [ "$attempt" = 6 ] && exit 1
    sleep 10
  done
fi
info "$APP_NAME ($app_id) is Contributor on $RG only"

if [ "$github" = 1 ]; then
  gh variable set AZURE_CLIENT_ID -R "$REPO" -b "$app_id"
  gh variable set AZURE_TENANT_ID -R "$REPO" -b "$tenant"
  gh variable set AZURE_SUBSCRIPTION_ID -R "$REPO" -b "$subscription"
  gh variable set RERAIL_AZURE_RESOURCE_GROUP -R "$REPO" -b "$RG"
  info "Set the workflow's repository variables on $REPO"
else
  echo "Repository variables for .github/workflows/env-matrix.yml (or rerun with --github):"
  echo "  AZURE_CLIENT_ID=$app_id AZURE_TENANT_ID=$tenant AZURE_SUBSCRIPTION_ID=$subscription RERAIL_AZURE_RESOURCE_GROUP=$RG"
fi
