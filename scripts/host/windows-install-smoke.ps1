# Release gate for native Windows: install a packed @neralabs/rerail tarball the way a user does, with
# `npx @neralabs/rerail setup`, from a throwaway local registry standing in for npmjs, in a sandbox profile, answering
# set-up's questions through its machine-readable lane. Then exercise the managed runtime: launcher, `current`
# junction, daemon and native engine, Claude Code's exec-form hook, the Codex hook as Codex runs it (PowerShell),
# update check, and coding-agent registration removal. Uses `node` from PATH, so the runner's architecture decides
# which native engine is exercised. The environment matrix (scripts/host/azure/guest.ps1) runs the same steps from
# windows-host.ps1 on Windows 11 VMs.
#
# Only the @neralabs scope resolves through the local registry (Verdaccio); every dependency still comes from
# npmjs, exactly as for a user who installs the published package.
param([Parameter(Mandatory)][string]$Tarball)

$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "windows-host.ps1")
$Tarball = (Resolve-Path $Tarball).Path
$base = Join-Path ([IO.Path]::GetTempPath()) ("rerail-windows-smoke-" + [guid]::NewGuid())
$sandbox = Join-Path $base "home"
$project = Join-Path $sandbox "proj"
$rerailHome = Join-Path $sandbox ".rerail"
$launcher = Join-Path $rerailHome "bin\rerail.cmd"

function Step($message) { Write-Host "==> $message" }

New-Item -ItemType Directory -Force $project, "$sandbox\.claude", "$sandbox\.codex", "$sandbox\.cursor", "$base\npm-cache" | Out-Null
$registry = $null
try {
  Step "Starting a local registry for the @neralabs scope"
  $registry = Start-RerailRegistry -Root "$base\registry" -Tarballs @($Tarball)

  Step "Running npx @neralabs/rerail setup in a sandbox profile, answering its questions (stdout captured)"
  # os.homedir() reads USERPROFILE on Windows; the rest pins npm and Rerail state, including the one Rerail
  # database under RERAIL_HOME, inside the sandbox. The user config maps only the @neralabs scope to the
  # local registry, so Rerail's own npm calls (install, update check) resolve it the same way. BROWSER=none keeps
  # the dashboard link from opening a browser on the runner.
  $env:USERPROFILE = $sandbox; $env:HOME = $sandbox; $env:RERAIL_HOME = $rerailHome
  $env:NPM_CONFIG_USERCONFIG = "$sandbox\.npmrc"; $env:NPM_CONFIG_CACHE = "$base\npm-cache"
  $env:RERAIL_STORAGE_ROOT = "$project\.rerail\sessions"
  $env:BROWSER = "none"
  Write-RerailUtf8 "$sandbox\.npmrc" "@neralabs:registry=$($registry.Url)/`n"
  $daemonPort = Get-RerailFreePort
  Invoke-RerailSetup -Spec "latest" -Project $project -Launcher $launcher -Options @("--port", "$daemonPort") | ForEach-Object { Write-Host $_ }

  Step "Checking the managed installation"
  Assert-RerailManagedInstall -RerailHome $rerailHome | ForEach-Object { Write-Host $_ }
  $status = (& $launcher status) -join "`n"
  Write-Host $status
  if ($status -notmatch [regex]::Escape("running at http://127.0.0.1:$daemonPort")) { throw "rerail status does not show the setup-started daemon" }
  if ($status -notmatch "Engine: ok") { throw "the daemon has no working native engine" }

  Step "Starting Rerail in the project"
  # Set-up's "start" answer turned Rerail on in the project; starting again says it is already on.
  Write-Host ((& $launcher start --repo $project) -join "`n")
  if ($LASTEXITCODE -ne 0) { throw "rerail start exited $LASTEXITCODE" }

  Step "Running the harness hooks as each harness spawns them"
  Invoke-RerailSessionStartHooks -UserHome $sandbox -Project $project -WorkDir $base -Prefix "windows-smoke" | ForEach-Object { Write-Host $_ }
  Assert-RerailSessionsRecorded -Port $daemonPort -SessionIds @("windows-smoke-claude", "windows-smoke-codex") | ForEach-Object { Write-Host $_ }

  Step "rerail update check"
  $update = (& $launcher update check) -join "`n"
  Write-Host $update
  if ($update -notmatch "Update available: no") { throw "update check did not report 'Update available: no'" }

  Step "rerail install --remove"
  & $launcher install --remove
  if ($LASTEXITCODE -ne 0) { throw "install --remove exited $LASTEXITCODE" }
  $leftovers = @(Get-RerailHookLeftovers -UserHome $sandbox)
  if ($leftovers.Count -gt 0) { throw "install --remove left Rerail hooks in $($leftovers -join ', ')" }
  Step "windows-install-smoke: PASS"
} finally {
  if (Test-Path $launcher) { & $launcher daemon stop | Out-Null }
  Stop-RerailRegistry $registry
  Remove-Item -Recurse -Force $base -ErrorAction SilentlyContinue
}
