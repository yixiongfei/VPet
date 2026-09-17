param([switch]$Rebuild)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$runtimeDir = Join-Path $projectRoot '.runtime'
$ollamaExe = Join-Path $runtimeDir 'ollama\ollama.exe'
# Release build with the frontend embedded. target\debug\vpet.exe is the `cargo run` / `tauri dev`
# binary: it expects the Vite dev server on :1420 and must not be used here.
$petExe = Join-Path $projectRoot 'apps\desktop\src-tauri\target\release\vpet.exe'
# Chat model + embedding model (semantic memory retrieval). Both are pulled once if missing.
$chatModel = 'qwen3.5:9b'
$embedModel = 'qwen3-embedding:0.6b'
New-Item -ItemType Directory -Force -Path $runtimeDir | Out-Null

# Process-local settings only; keep model weights in this project on D:.
$env:OLLAMA_HOST = '127.0.0.1:11434'
$env:OLLAMA_MODELS = Join-Path $runtimeDir 'models'
# Run inference on the GPU. Ollama drops integrated GPUs by default; the Intel Arc iGPU
# via Vulkan cuts the 9B model's first-token latency from ~4.8 s to ~1.9 s and keeps the CPU free.
$env:OLLAMA_VULKAN = '1'
$env:OLLAMA_IGPU_ENABLE = '1'
$env:OLLAMA_KEEP_ALIVE = '30m'

$ollamaReady = $false
try { $null = Invoke-RestMethod 'http://127.0.0.1:11434/api/version' -TimeoutSec 2; $ollamaReady = $true } catch {}
if (-not $ollamaReady -and (Test-Path -LiteralPath $ollamaExe)) {
    Start-Process -FilePath $ollamaExe -ArgumentList 'serve' -WorkingDirectory $runtimeDir -WindowStyle Hidden `
        -RedirectStandardOutput (Join-Path $runtimeDir 'ollama-server.log') `
        -RedirectStandardError (Join-Path $runtimeDir 'ollama-server-error.log') | Out-Null
    foreach ($i in 1..20) {
        Start-Sleep -Milliseconds 500
        try { $null = Invoke-RestMethod 'http://127.0.0.1:11434/api/version' -TimeoutSec 2; $ollamaReady = $true; break } catch {}
    }
}

# Pull anything missing. This is the one place downloads happen (6.6 GB + 0.6 GB on first run).
if ($ollamaReady -and (Test-Path -LiteralPath $ollamaExe)) {
    try {
        $installed = (Invoke-RestMethod 'http://127.0.0.1:11434/api/tags' -TimeoutSec 5).models | ForEach-Object { $_.name }
        foreach ($model in @($chatModel, $embedModel)) {
            if ($installed -notcontains $model -and $installed -notcontains "${model}:latest") {
                Write-Host "Downloading $model (first run only)..."
                & $ollamaExe pull $model
            }
        }
    } catch { Write-Warning "Could not check installed models: $_" }
}

$runningPet = Get-Process vpet -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $petExe }
if ($runningPet) {
    Write-Host 'VPet is already running. Click the pet or press Alt+V to chat.'
    return
}
if ($Rebuild -or -not (Test-Path -LiteralPath $petExe)) {
    Push-Location $projectRoot
    try {
        & pnpm build:local
        if ($LASTEXITCODE -ne 0) { throw 'VPet build failed.' }
    } finally { Pop-Location }
}
# build:local (tauri build --no-bundle) embeds the frontend: no terminal/dev server needs to stay open.
Start-Process -FilePath $petExe -WorkingDirectory $projectRoot -WindowStyle Hidden `
    -RedirectStandardOutput (Join-Path $runtimeDir 'vpet.log') `
    -RedirectStandardError (Join-Path $runtimeDir 'vpet-error.log') | Out-Null
Write-Host 'VPet started. Click the pet to chat; right-click for settings.'
