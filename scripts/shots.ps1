# pnpm shots
#
# Builds the design preview mode, serves it as static files, and renders
# every fixture screen with headless Edge into docs/screenshots/before/,
# at 1280x800 and 1920x1080. BlueFlame has no light theme (App.css
# defines one dark palette only), so this only captures dark. Rerun
# after a real design pass to produce the "after" set for comparison.
#
# This is a PowerShell script, not a Node one, on purpose: launching
# msedge.exe from Node's own child_process (spawnSync), even through a
# `Start-Process -Wait` wrapper, was measured on this host to sometimes
# take minutes per shot instead of the ~1-2s a plain `pnpm build:design`
# + PowerShell loop takes. Node calling PowerShell calling msedge.exe
# added a nesting level that Node calling msedge.exe directly did not
# fix either; only removing Node from that specific call chain did.
# Node still does the build and serves the static files (as its own
# sibling process, never the one launching msedge), just not the
# screenshot step itself.

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$distDesign = Join-Path $root 'dist-design'
$outDir = Join-Path $root 'docs\screenshots\before'
$edge = 'C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe'
$port = 4173
$profileDir = Join-Path $env:TEMP 'blueflame-design-shots-profile'

if (-not (Test-Path $edge)) {
    throw "headless Edge not found at $edge"
}

Write-Output 'building design preview...'
Push-Location $root
try {
    & pnpm build:design
    if ($LASTEXITCODE -ne 0) { throw "pnpm build:design exited with $LASTEXITCODE" }
} finally {
    Pop-Location
}

New-Item -ItemType Directory -Force -Path $outDir | Out-Null
Remove-Item -Recurse -Force $profileDir -ErrorAction SilentlyContinue

# A daily-driver Edge (or a Tauri app's WebView2) is usually already
# running on this machine. Clear out anything left over from an
# interrupted previous run of this script before starting: a pile of
# dead headless instances is enough to make every new launch slow.
Get-CimInstance Win32_Process -Filter "Name='msedge.exe'" |
    Where-Object { $_.CommandLine -like '*headless*' -and $_.CommandLine -like '*blueflame-design-shots-profile*' } |
    ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }

$env:SHOTS_PORT = "$port"
$serverScript = Join-Path $root 'scripts\serve-dist-design.mjs'
# The repo lives under a path with spaces ("Development and Research"),
# and Start-Process -ArgumentList does not reliably quote a single
# unquoted string argument before handing it to CreateProcess: the space
# splits it into two arguments and node fails to resolve either one.
$serverProc = Start-Process -FilePath 'node' -ArgumentList "`"$serverScript`"" `
    -WorkingDirectory $root -NoNewWindow -PassThru

$serverUp = $false
for ($i = 0; $i -lt 20; $i++) {
    Start-Sleep -Milliseconds 250
    try {
        Invoke-WebRequest -Uri "http://127.0.0.1:$port/design.html" -UseBasicParsing -TimeoutSec 2 | Out-Null
        $serverUp = $true
        break
    } catch {
        if ($serverProc.HasExited) { break }
    }
}
if (-not $serverUp) {
    Stop-Process -Id $serverProc.Id -Force -ErrorAction SilentlyContinue
    throw "dist-design server on http://127.0.0.1:$port never came up (see node output above)"
}
Write-Output "serving dist-design on http://127.0.0.1:$port (pid $($serverProc.Id))"

# id, query string appended to design.html. hideswitcher=1 keeps the
# dev-only jump panel out of the actual screenshots.
$shots = @(
    @{ id = 'dashboard'; query = 'screen=dashboard&hideswitcher=1' }
    @{ id = 'browsing'; query = 'screen=browsing&hideswitcher=1' }
    @{ id = 'bookmarks'; query = 'screen=bookmarks&hideswitcher=1' }
    @{ id = 'downloads'; query = 'screen=downloads&hideswitcher=1' }
    @{ id = 'metrics'; query = 'screen=metrics&hideswitcher=1' }
    @{ id = 'settings'; query = 'screen=settings&hideswitcher=1' }
    @{ id = 'debug'; query = 'screen=debug&hideswitcher=1' }
    @{ id = 'trust-modal'; query = 'screen=trust-modal&hideswitcher=1' }
    @{ id = 'approval'; query = 'screen=approval&hideswitcher=1' }
    @{ id = 'mobile'; query = 'screen=mobile&hideswitcher=1' }
    @{ id = 'tabswitcher'; query = 'screen=tabswitcher&hideswitcher=1' }
    @{ id = 'menu-hamburger'; query = 'panel=menu&kind=hamburger&view=dashboard' }
    @{ id = 'menu-kebab'; query = 'panel=menu&kind=kebab&view=dashboard&bookmarked=1&browsing=1' }
    @{ id = 'context-menu'; query = 'panel=context' }
    @{ id = 'trust-popup'; query = 'panel=trust&url=https://github.com/w1ck3ds0d4/BlueFlame&tab=overview' }
)
$sizes = @(
    @{ w = 1280; h = 800 }
    @{ w = 1920; h = 1080 }
)

$missing = @()
try {
    foreach ($shot in $shots) {
        foreach ($size in $sizes) {
            $outFile = Join-Path $outDir "$($shot.id)-$($size.w)x$($size.h)-dark.png"
            $url = "http://127.0.0.1:$port/design.html?$($shot.query)"
            $argStr = '--headless=new --disable-gpu --hide-scrollbars --no-first-run --no-default-browser-check ' +
                '--proxy-server=direct:// --proxy-bypass-list=* ' +
                "`"--user-data-dir=$profileDir`" --window-size=$($size.w),$($size.h) " +
                "`"--screenshot=$outFile`" --virtual-time-budget=4000 `"$url`""

            Write-Output "shooting $($shot.id) $($size.w)x$($size.h)"
            $p = Start-Process -FilePath $edge -ArgumentList $argStr -Wait -PassThru -NoNewWindow
            if (Test-Path $outFile) {
                Write-Output "  wrote $($shot.id)-$($size.w)x$($size.h)-dark.png"
            } else {
                Write-Output "  MISSING (exit $($p.ExitCode))"
                $missing += "$($shot.id)-$($size.w)x$($size.h)"
            }
        }
    }
} finally {
    Stop-Process -Id $serverProc.Id -Force -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force $profileDir -ErrorAction SilentlyContinue
}

$total = $shots.Count * $sizes.Count
$ok = $total - $missing.Count
Write-Output "done: $ok/$total screenshots in docs\screenshots\before"
if ($missing.Count -gt 0) {
    Write-Output "missing: $($missing -join ', ')"
    exit 1
}
