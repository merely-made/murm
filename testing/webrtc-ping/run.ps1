# Driver for the C1 headed receipt.
#
#   1. build the page half for wasm32 and generate its bindings into ./pkg
#   2. build the native answerer
#   3. start the answerer (its stdout, including the WEBRTC_PING_RECEIPT line,
#      lands in this console)
#   4. serve this directory so index.html and pkg/ are same-origin
#
# wasm-pack and http-server are deliberately not used: neither is installed on
# this machine, and neither is needed. `wasm-bindgen --target web` is the whole
# of step 1, and Python's `http.server` already maps .wasm to application/wasm.
#
# The wasm-bindgen CLI and the wasm-bindgen crate must agree exactly; the crate
# is pinned to =0.2.126 in Cargo.toml to match the installed CLI, and this
# script refuses to go on if that stops being true.
#
# Ctrl-C stops the web server; the answerer is stopped in `finally`.
param(
    [int]$Port = 8080,
    [int]$SignalPort = 8787,
    # 0 = ephemeral, which is what lets each session rebind while the previous
    # socket is still winding down.
    [int]$UdpPort = 0,
    [string]$WasmBindgen = 'wasm-bindgen',
    [string]$Python = 'python',
    [switch]$SkipBuild,
    [switch]$NoServe
)

$ErrorActionPreference = 'Stop'
Push-Location $PSScriptRoot
$answerer = $null
try {
    if (-not $SkipBuild) {
        # The pin, checked rather than assumed. A schema mismatch here shows up
        # as an opaque binding-generation failure three steps later.
        $cliVersion = (& $WasmBindgen --version) -replace '^wasm-bindgen\s+', ''
        $cratePin = (Select-String -Path 'Cargo.toml' -Pattern 'wasm-bindgen\s*=\s*\{\s*version\s*=\s*"=([0-9.]+)"').Matches[0].Groups[1].Value
        if ($cliVersion.Trim() -ne $cratePin) {
            throw "wasm-bindgen CLI is $cliVersion but Cargo.toml pins =$cratePin. They must match exactly."
        }

        Write-Host "== building the page half (wasm32-unknown-unknown, --features browser)" -ForegroundColor Cyan
        cargo build --target wasm32-unknown-unknown --features browser
        if ($LASTEXITCODE -ne 0) { throw "wasm build failed" }

        Write-Host "== generating bindings into ./pkg" -ForegroundColor Cyan
        & $WasmBindgen --target web --out-dir pkg target/wasm32-unknown-unknown/debug/webrtc_ping.wasm
        if ($LASTEXITCODE -ne 0) { throw "wasm-bindgen failed" }

        Write-Host "== building the native answerer (--features native)" -ForegroundColor Cyan
        cargo build --bin answerer --features native
        if ($LASTEXITCODE -ne 0) { throw "answerer build failed" }
    }

    $exe = Join-Path $PSScriptRoot 'target\debug\answerer.exe'
    if (-not (Test-Path $exe)) { throw "no answerer at $exe — run without -SkipBuild" }

    Write-Host "== starting the answerer" -ForegroundColor Cyan
    # Base direct-mode args, unchanged. Optional TURN flags are appended only
    # when their env vars are present, so the direct receipt still runs with
    # nothing set. The long-term secret is passed straight through to the
    # answerer, which mints from it; it never reaches the browser.
    $answererArgs = @('--signal-port', "$SignalPort", '--udp-port', "$UdpPort")
    if ($env:WEBRTC_PING_TURN_SECRET) {
        $answererArgs += @('--turn-secret', $env:WEBRTC_PING_TURN_SECRET)
    }
    if ($env:WEBRTC_PING_TURN_REALM) {
        $answererArgs += @('--turn-realm', $env:WEBRTC_PING_TURN_REALM)
    }
    if ($env:WEBRTC_PING_TURN_TTL) {
        $answererArgs += @('--turn-ttl', $env:WEBRTC_PING_TURN_TTL)
    }
    if ($env:WEBRTC_PING_TURN_URL) {
        foreach ($u in ($env:WEBRTC_PING_TURN_URL -split '[;,]')) {
            if ($u.Trim()) { $answererArgs += @('--turn-url', $u.Trim()) }
        }
    }
    if ($answererArgs.Count -gt 4) {
        Write-Host "   passing TURN flags through from the environment" -ForegroundColor Yellow
    }
    $answerer = Start-Process -FilePath $exe `
        -ArgumentList $answererArgs `
        -NoNewWindow -PassThru

    if ($NoServe) {
        Write-Host "answerer running (pid $($answerer.Id)); -NoServe, so no web server. Ctrl-C to stop." -ForegroundColor Yellow
        Wait-Process -Id $answerer.Id
        return
    }

    Write-Host ""
    Write-Host "  OPEN  http://127.0.0.1:$Port/index.html" -ForegroundColor Green
    Write-Host "  the page's signalling field should read  http://127.0.0.1:$SignalPort/offer"
    Write-Host ""
    Write-Host "== serving $PSScriptRoot on 127.0.0.1:$Port (Ctrl-C to stop)" -ForegroundColor Cyan
    & $Python -m http.server $Port --bind 127.0.0.1
} finally {
    if ($answerer -and -not $answerer.HasExited) {
        Write-Host "== stopping the answerer" -ForegroundColor Cyan
        Stop-Process -Id $answerer.Id -Force -ErrorAction SilentlyContinue
    }
    Pop-Location
}
