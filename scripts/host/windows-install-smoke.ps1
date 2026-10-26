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
# An accented profile folder, as a person's name gives it: cmd.exe reads the batch launcher in the console's code
# page, so a path written out in UTF-8 would come back garbled.
$sandbox = Join-Path $base ("Jos" + [char]0x00E9)
$project = Join-Path $sandbox "proj"
$rerailHome = Join-Path $sandbox ".rerail"
$launcher = Join-Path $rerailHome "bin\rerail.cmd"

function Step($message) { Write-Host "==> $message" }

New-Item -ItemType Directory -Force $project, "$sandbox\.claude", "$sandbox\.codex", "$sandbox\.cursor", "$base\npm-cache", "$base\git-bash-home" | Out-Null
$registry = $null
try {
  Step "Starting a local registry for the @neralabs scope"
  $registry = Start-RerailRegistry -Root "$base\registry" -Tarballs @($Tarball)

  Step "Running npx @neralabs/rerail setup in a sandbox profile, answering its questions (stdout captured)"
  # os.homedir() reads USERPROFILE on Windows; the rest pins npm and Rerail state, including the one Rerail
  # database under RERAIL_HOME, inside the sandbox. The user config maps only the @neralabs scope to the
  # local registry, so Rerail's own npm calls (install, update check) resolve it the same way. BROWSER=none keeps
  # the dashboard link from opening a browser on the runner. HOME names another folder, as Git Bash's may: coding
  # agents read the profile folder, so set-up must connect them there.
  $env:USERPROFILE = $sandbox; $env:HOME = "$base\git-bash-home"; $env:RERAIL_HOME = $rerailHome
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

  Step "Running rerail from Git Bash"
  # Git Bash runs a command without an extension or with `.exe`, never `rerail.cmd`; a new terminal gets the user Path
  # set-up wrote, which this one predates.
  $windowsPath = $env:PATH
  $env:PATH = "$rerailHome\bin;$windowsPath"
  $gitBash = (& (Join-Path $env:ProgramFiles "Git\bin\bash.exe") -c 'command -v rerail && rerail version') -join "`n"
  $gitBashExit = $LASTEXITCODE
  $env:PATH = $windowsPath
  Write-Host $gitBash
  if ($gitBashExit -ne 0) { throw "bare rerail did not run in Git Bash (exit $gitBashExit)" }

  Step "Running rerail.cmd in a console on code page 437"
  # The runner's console is on UTF-8 (65001); a person's is on their OEM code page, 437 or 850 in the West. A console of
  # its own (CreateNoWindow) keeps the chcp from reaching this one.
  $env:SMOKE_LAUNCHER = $launcher
  $legacy = New-Object Diagnostics.ProcessStartInfo("cmd.exe")
  $legacy.Arguments = '/d /s /c "chcp 437 >nul & call "%SMOKE_LAUNCHER%" version"'
  $legacy.UseShellExecute = $false; $legacy.CreateNoWindow = $true
  $legacy.RedirectStandardOutput = $true; $legacy.RedirectStandardError = $true
  $legacyRun = [Diagnostics.Process]::Start($legacy)
  $legacyOut = $legacyRun.StandardOutput.ReadToEndAsync(); $legacyErr = $legacyRun.StandardError.ReadToEndAsync()
  $legacyRun.WaitForExit()
  Write-Host ($legacyOut.Result + $legacyErr.Result)
  if ($legacyRun.ExitCode -ne 0) { throw "rerail.cmd failed on code page 437 (exit $($legacyRun.ExitCode))" }

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
