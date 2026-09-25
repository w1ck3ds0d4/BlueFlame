param(
    [ValidateSet('before', 'after')]
    [string]$Out = 'before'
)

# pnpm shots / pnpm shots:after
#
# Builds the design preview mode, serves it as static files, and renders
# every fixture screen with headless Edge into docs/screenshots/<Out>/,
# in dark at 1280x800 and 1920x1080. The "after" set (once the WickIT
# design system adoption landed) also renders light at 1280x800, the
# laptop width the design system's pre-ship checklist asks a console
# screen to be checked at in both themes; "before" predates the light
# theme entirely, so it stays dark-only. Run plain `pnpm shots` for the
# before set and `pnpm shots:after` for the after set, so the two can
# sit side by side.
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

# Deliberately not 'Stop': the per-shot retry loop below already treats
# a failed attempt as recoverable, and this host has shown transient
# Win32-level hiccups (a locked file, a slow-to-release Edge profile
# handle) that are not worth failing 45 shots over. Every step that
# truly must not fail silently (missing Edge, the dev server never
# coming up, every retry of one shot failing) already throws or checks
# its own exit code explicitly.
$ErrorActionPreference = 'Continue'

# Edge can still hold a handle open on its own profile directory for a
# moment after the process this script waited on has exited (AV scan,
# a lagging crashpad writer). A plain Remove-Item -ErrorAction
# SilentlyContinue does not always swallow that: it is a low-level
# Win32Exception, not an ordinary PowerShell error record. Retry
# briefly instead of failing the whole run over a directory that will
# be gone a few hundred ms later anyway.
function Remove-ProfileDir($path) {
    for ($i = 0; $i -lt 5; $i++) {
        try {
            Remove-Item -Recurse -Force $path -ErrorAction Stop
            return
        } catch [System.Management.Automation.ItemNotFoundException] {
            return
        } catch {
            Start-Sleep -Milliseconds 200
        }
    }
}

$root = Split-Path -Parent $PSScriptRoot
$distDesign = Join-Path $root 'dist-design'
$outDir = Join-Path $root "docs\screenshots\$Out"
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
Remove-ProfileDir $profileDir

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

# Warm up the shared profile before the real sequence. In testing, the
# first handful of shots against a brand new --user-data-dir landed on
# the fixture's pre-click state far more often than later ones did in
# the exact same run, on the exact same code path; a few throwaway
# navigations here removed that pattern. Output is discarded.
$warmupFile = Join-Path $env:TEMP 'blueflame-shots-warmup.png'
for ($i = 0; $i -lt 3; $i++) {
    $warmupArgs = '--headless=new --disable-gpu --hide-scrollbars --no-first-run --no-default-browser-check ' +
        '--proxy-server=direct:// --proxy-bypass-list=* ' +
        "`"--user-data-dir=$profileDir`" --window-size=1280,800 " +
        "`"--screenshot=$warmupFile`" --virtual-time-budget=20000 `"http://127.0.0.1:$port/design.html?screen=dashboard&hideswitcher=1`""
    Start-Process -FilePath $edge -ArgumentList $warmupArgs -Wait -NoNewWindow | Out-Null
}
Remove-Item -Force $warmupFile -ErrorAction SilentlyContinue
Write-Output 'profile warmed up'

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
# Dark at both sizes, every screen. Light only for the after set, only
# at the laptop width the design system asks a console screen be
# checked against in both themes.
$runs = @()
foreach ($shot in $shots) {
    $runs += @{ shot = $shot; w = 1280; h = 800; theme = 'dark'; themeParam = '' }
    $runs += @{ shot = $shot; w = 1920; h = 1080; theme = 'dark'; themeParam = '' }
    if ($Out -eq 'after') {
        $runs += @{ shot = $shot; w = 1280; h = 800; theme = 'light'; themeParam = '&theme=light' }
    }
}

$missing = @()
try {
    foreach ($run in $runs) {
        $shot = $run.shot
        # A fresh profile per run, not just per script invocation. Two
        # shots of the same screen back to back, one dark one light,
        # once shared a profile long enough for the first one's state
        # to still be on disk when the second launched, and the second
        # rendered the first one's theme instead of its own.
        Remove-ProfileDir $profileDir
        $outFile = Join-Path $outDir "$($shot.id)-$($run.w)x$($run.h)-$($run.theme).png"
        $url = "http://127.0.0.1:$port/design.html?$($shot.query)$($run.themeParam)"

        # The fixture-driven screens reach their real state through a
        # click dispatched from a setTimeout after mount (driveToScreen
        # in design-main.tsx), not on first paint. Chromium's headless
        # --virtual-time-budget occasionally ends and takes the shot
        # before that click has landed, capturing the screen mid-boot
        # instead of settled. That is a timing race, not a per-shot
        # property, so retrying is the fix: take up to 5 shots and keep
        # the largest file, since a settled screen has strictly more on
        # it than the transient boot state does.
        $attemptFiles = @()
        for ($attempt = 1; $attempt -le 5; $attempt++) {
            $attemptFile = Join-Path $outDir "$($shot.id)-$($run.w)x$($run.h)-$($run.theme).attempt$attempt.png"
            $argStr = '--headless=new --disable-gpu --hide-scrollbars --no-first-run --no-default-browser-check ' +
                '--proxy-server=direct:// --proxy-bypass-list=* ' +
                "`"--user-data-dir=$profileDir`" --window-size=$($run.w),$($run.h) " +
                "`"--screenshot=$attemptFile`" --virtual-time-budget=20000 `"$url`""
            Start-Process -FilePath $edge -ArgumentList $argStr -Wait -NoNewWindow | Out-Null
            if (Test-Path $attemptFile) { $attemptFiles += $attemptFile }
        }

        Write-Output "shooting $($shot.id) $($run.w)x$($run.h) $($run.theme)"
        # Re-stat defensively: something on this machine (seen in testing,
        # never explained - possibly AV scanning a freshly written PNG
        # from a headless browser process) occasionally removes an
        # attempt file between the existence check above and now.
        $survivors = $attemptFiles | Where-Object { Test-Path $_ } | Get-Item
        if ($survivors) {
            $best = $survivors | Sort-Object Length -Descending | Select-Object -First 1
            Copy-Item -Force $best.FullName $outFile
            Write-Output "  wrote $($shot.id)-$($run.w)x$($run.h)-$($run.theme).png ($($best.Length) bytes, best of $($survivors.Count))"
            foreach ($f in $attemptFiles) { Remove-Item -Force $f -ErrorAction SilentlyContinue }
        } else {
            Write-Output "  MISSING (all attempts failed or vanished)"
            $missing += "$($shot.id)-$($run.w)x$($run.h)-$($run.theme)"
        }
    }
} finally {
    Stop-Process -Id $serverProc.Id -Force -ErrorAction SilentlyContinue
    Remove-ProfileDir $profileDir
}

$total = $runs.Count
$ok = $total - $missing.Count
Write-Output "done: $ok/$total screenshots in docs\screenshots\$Out"
if ($missing.Count -gt 0) {
    Write-Output "missing: $($missing -join ', ')"
    exit 1
}
