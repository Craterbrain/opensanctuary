# Automates the Windows half of docs/TUNNELS.md: installs Caddy (with the
# Cloudflare DNS-01 plugin) as a persistent background service (via NSSM,
# since Windows Home has no built-in way to run a standalone .exe as a
# service) reverse-proxying to OS-Next's loopback-only plane, so a church
# gets a real, browser-trusted HTTPS domain instead of self-signed-cert
# warnings. Idempotent -- re-running it updates an existing install's
# Caddyfile/token and restarts the service rather than failing.
#
# Usage (Administrator PowerShell):
#   .\packaging\setup-tunnel-windows.ps1 -Domain connect.yourchurch.org [-Port 8080] [-DryRun]
#   (The Cloudflare API token is prompted for securely, never passed as a
#    parameter -- a plain parameter would land in PowerShell history.)
#
# What this does NOT automate (see docs/TUNNELS.md for why, and the manual
# steps if you'd rather do any of this by hand):
#   - The local DNS override that makes the domain resolve on-site even when
#     the building's internet is down -- that's done in the church's router
#     or an internal DNS server (Pi-hole/AdGuard), which this script has no
#     access to.
#   - Setting the "Public HTTPS URL" setting in OS-Next itself once this is
#     up -- that's a one-time step in Settings > Network, verified live
#     there via the reachability badge.

[CmdletBinding()]
param(
    [string]$Domain = "",
    [string]$Port = "8080",
    [switch]$DryRun
)

$ErrorActionPreference = "Stop"

function Invoke-Step {
    # Prints every command this script would run even in -DryRun, so an
    # admin can review exactly what's about to happen to their machine
    # before it does -- and actually runs it otherwise.
    param([string]$Description, [scriptblock]$Action)
    Write-Host "+ $Description"
    if (-not $DryRun) {
        & $Action
    }
}

$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $DryRun -and -not $isAdmin) {
    Write-Error "This script needs an Administrator PowerShell (installs a service, opens firewall ports, binds 80/443). Re-run as Administrator."
    exit 1
}

if ([string]::IsNullOrWhiteSpace($Domain)) {
    $Domain = Read-Host "Public domain (e.g. connect.yourchurch.org)"
}
if ([string]::IsNullOrWhiteSpace($Domain)) {
    Write-Error "A domain is required."
    exit 1
}

# Never echoed, never passed as a parameter -- only needs the Cloudflare
# Zone.DNS:Edit permission scoped to this one zone (docs/TUNNELS.md), not
# full account access.
$cfTokenSecure = Read-Host "Cloudflare API token (Zone.DNS:Edit, scoped to this zone)" -AsSecureString
$cfToken = [Runtime.InteropServices.Marshal]::PtrToStringAuto([Runtime.InteropServices.Marshal]::SecureStringToBSTR($cfTokenSecure))
if ([string]::IsNullOrWhiteSpace($cfToken)) {
    Write-Error "A Cloudflare API token is required."
    exit 1
}

Write-Host "============================================================"
Write-Host " OS-Next Tunnel Setup (Windows) -- docs/TUNNELS.md"
Write-Host "============================================================"
Write-Host " Domain          : $Domain"
Write-Host " OS-Next port    : $Port"
Write-Host " Dry run         : $($DryRun.IsPresent)"
Write-Host "============================================================"

# --- 1. Port collision check --------------------------------------------
# The single most common reason this kind of setup fails is something else
# already bound to 80/443 (IIS, Skype-for-Business-style tools, a stray
# previous Caddy instance) -- fail fast with a clear answer instead of a
# cryptic bind error deep inside Caddy's own startup log.
Write-Host "==> Checking 80/443 for existing listeners..."
$portConflict = $false
foreach ($p in @(80, 443)) {
    $conns = Get-NetTCPConnection -LocalPort $p -State Listen -ErrorAction SilentlyContinue
    if ($conns) {
        foreach ($c in $conns) {
            $procName = (Get-Process -Id $c.OwningProcess -ErrorAction SilentlyContinue).ProcessName
            Write-Host "!! Port $p is already in use by $procName (PID $($c.OwningProcess))"
        }
        $portConflict = $true
    }
}
if ($portConflict) {
    Write-Host "   Stop or reconfigure whatever that is before continuing, or run this on a different machine."
    if ($DryRun) {
        Write-Host "    (dry run -- would abort here for real)"
    } else {
        exit 1
    }
}

# --- 2. Fetch Caddy with the Cloudflare DNS plugin ----------------------
# Caddy's official download API builds a custom binary with exactly the
# plugins requested -- the same official source docs/TUNNELS.md already
# points admins to for the manual download, just automated.
$caddyDir = "C:\Caddy"
$caddyExe = Join-Path $caddyDir "caddy.exe"
$hasCloudflarePlugin = $false
if (Test-Path $caddyExe) {
    $modules = & $caddyExe list-modules 2>$null
    $hasCloudflarePlugin = $modules -match "dns\.providers\.cloudflare"
}
if ($hasCloudflarePlugin) {
    Write-Host "==> Caddy with the Cloudflare plugin already installed at $caddyExe -- skipping download."
} else {
    Write-Host "==> Downloading Caddy (with github.com/caddy-dns/cloudflare) from caddyserver.com..."
    $downloadUrl = "https://caddyserver.com/api/download?os=windows&arch=amd64&p=github.com/caddy-dns/cloudflare"
    Invoke-Step "New-Item -ItemType Directory -Force $caddyDir" { New-Item -ItemType Directory -Force -Path $caddyDir | Out-Null }
    Invoke-Step "Download caddy.exe from $downloadUrl" { Invoke-WebRequest -Uri $downloadUrl -OutFile $caddyExe }
}

# --- 3. Caddyfile -------------------------------------------------------
Write-Host "==> Writing $caddyDir\Caddyfile..."
$caddyfileContent = @"
$Domain {
    tls {
        dns cloudflare {env.CLOUDFLARE_API_TOKEN}
    }

    reverse_proxy localhost:$Port
}
"@
if ($DryRun) {
    Write-Host "+ write $caddyDir\Caddyfile:"
    $caddyfileContent -split "`n" | ForEach-Object { Write-Host "    $_" }
} else {
    New-Item -ItemType Directory -Force -Path $caddyDir | Out-Null
    Set-Content -Path (Join-Path $caddyDir "Caddyfile") -Value $caddyfileContent
}

# --- 4. NSSM service -----------------------------------------------------
Write-Host "==> Registering Caddy as a background service via NSSM..."
if (-not (Get-Command nssm -ErrorAction SilentlyContinue)) {
    Invoke-Step "winget install -e --id NSSM.NSSM" {
        winget install -e --id NSSM.NSSM --accept-source-agreements --accept-package-agreements
    }
}
$serviceExists = Get-Service -Name Caddy -ErrorAction SilentlyContinue
if (-not $serviceExists) {
    Invoke-Step "nssm install Caddy $caddyExe `"run --config $caddyDir\Caddyfile`"" {
        nssm install Caddy $caddyExe "run --config $caddyDir\Caddyfile"
    }
}
Invoke-Step "nssm set Caddy AppEnvironmentExtra CLOUDFLARE_API_TOKEN=<redacted>" {
    nssm set Caddy AppEnvironmentExtra "CLOUDFLARE_API_TOKEN=$cfToken"
}
Invoke-Step "nssm set Caddy Start SERVICE_AUTO_START" { nssm set Caddy Start SERVICE_AUTO_START }
if ($serviceExists -and $serviceExists.Status -eq "Running") {
    Write-Host "==> Caddy already running -- restarting to pick up the updated config/token."
    Invoke-Step "nssm restart Caddy" { nssm restart Caddy }
} else {
    Invoke-Step "nssm start Caddy" { nssm start Caddy }
}

# --- 5. Firewall ---------------------------------------------------------
Write-Host "==> Opening the firewall for 80/443..."
function Show-FirewallManualFallback {
    Write-Host "   Could not open the firewall automatically. Open it manually (Administrator PowerShell):"
    Write-Host "     New-NetFirewallRule -DisplayName 'Caddy-HTTP' -Direction Inbound -LocalPort 80 -Protocol TCP -Action Allow"
    Write-Host "     New-NetFirewallRule -DisplayName 'Caddy-HTTPS' -Direction Inbound -LocalPort 443 -Protocol TCP -Action Allow"
    Write-Host "   See docs/TUNNELS.md 'Setup for Windows' step 2 for the full reference."
}
try {
    Invoke-Step "New-NetFirewallRule -DisplayName 'Caddy-HTTP' -LocalPort 80" {
        if (-not (Get-NetFirewallRule -DisplayName "Caddy-HTTP" -ErrorAction SilentlyContinue)) {
            New-NetFirewallRule -DisplayName "Caddy-HTTP" -Direction Inbound -LocalPort 80 -Protocol TCP -Action Allow | Out-Null
        }
    }
    Invoke-Step "New-NetFirewallRule -DisplayName 'Caddy-HTTPS' -LocalPort 443" {
        if (-not (Get-NetFirewallRule -DisplayName "Caddy-HTTPS" -ErrorAction SilentlyContinue)) {
            New-NetFirewallRule -DisplayName "Caddy-HTTPS" -Direction Inbound -LocalPort 443 -Protocol TCP -Action Allow | Out-Null
        }
    }
} catch {
    Write-Host "   $_"
    Show-FirewallManualFallback
}

Write-Host "============================================================"
Write-Host " Still to do manually (docs/TUNNELS.md has the full detail):"
Write-Host "============================================================"
Write-Host " 1. Local DNS override: in the church's router (or Pi-hole/AdGuard),"
Write-Host "    add an A record pointing $Domain -> this machine's LAN IP, so"
Write-Host "    on-site devices can resolve it even when the internet is down."
Write-Host "    If the router has DNS rebinding protection, allow-list the domain."
Write-Host " 2. In OS-Next: Settings > Network > Public HTTPS URL, set it to"
Write-Host "    https://$Domain -- the badge there live-verifies it reaches this"
Write-Host "    server once DNS/Caddy are both up."
Write-Host "============================================================"

if (-not $DryRun) {
    Write-Host "==> Waiting a moment for Caddy to obtain the certificate, then verifying..."
    Start-Sleep -Seconds 5
    try {
        $resp = Invoke-WebRequest -Uri "https://$Domain" -UseBasicParsing -TimeoutSec 10
        Write-Host "    https://$Domain is responding (HTTP $($resp.StatusCode)). Check 'Get-Service Caddy' / the NSSM-managed service log if anything looks off."
    } catch {
        Write-Host "    https://$Domain isn't responding yet -- this is normal if DNS hasn't propagated or the local override"
        Write-Host "    above isn't in place yet. Check the NSSM-managed service log for the actual certificate-issuance status."
    }
}
