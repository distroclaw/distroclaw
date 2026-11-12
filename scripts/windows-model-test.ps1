# Downloads models from the DistroClaw model library with the AI engine the Windows installer
# ships, chats with each once and deletes it again, making the same engine calls as the app
# (src-tauri/src/models.rs pull, src-tauri/src/service/setup.rs remove_model). Used by
# .github/workflows/windows.yml. The CI runner has no graphics card, so it also shows the engine
# running on the processor. Models go to a temporary folder that is deleted at the end.
#
#   pwsh scripts/windows-model-test.ps1 -Engine <folder with ollama.exe> -Models qwen3.5:0.8b,qwen3.5:4b -Report <folder>
#
# Results are written to <folder>/model-test.md; the script fails on the first check that fails.

param(
  [Parameter(Mandatory)] [string] $Engine,
  [Parameter(Mandatory)] [string[]] $Models,
  [Parameter(Mandatory)] [string] $Report,
  # The Windows app asks the library for the GGUF build: <tag>-gguf (src-tauri/src/models.rs).
  [string] $Library = 'models.distroclaw.com'
)

$ErrorActionPreference = 'Stop'
# Also accepts "a,b" as one argument, as `pwsh -File` passes a comma list.
$Models = @($Models | ForEach-Object { $_ -split ',' } | ForEach-Object Trim | Where-Object { $_ })
New-Item -ItemType Directory -Force -Path $Report | Out-Null
$log = Join-Path $Report 'model-test.md'
Set-Content -Path $log -Value "# Windows model test`n" -Encoding utf8

function Note([string] $Text) {
  Add-Content -Path $log -Value $Text -Encoding utf8
  Write-Host $Text
}

function Check([string] $What, [bool] $Ok, [string] $Detail = '') {
  $line = if ($Ok) { "- passed: $What" } else { "- FAILED: $What" }
  if ($Detail) { $line += " ($Detail)" }
  Note $line
  if (-not $Ok) { throw "model test failed: $What $Detail" }
}

# A free port on the loopback address, as the app picks one.
$listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, 0)
$listener.Start()
$port = $listener.LocalEndpoint.Port
$listener.Stop()
$base = "http://127.0.0.1:$port"
$modelsDir = Join-Path ([IO.Path]::GetTempPath()) "distroclaw-model-test-$([guid]::NewGuid())"
New-Item -ItemType Directory -Force -Path $modelsDir | Out-Null

# The settings the app starts the engine with (src-tauri/src/engine.rs engine_command).
$env:OLLAMA_HOST = "127.0.0.1:$port"
$env:OLLAMA_MODELS = $modelsDir
$env:OLLAMA_KEEP_ALIVE = '30m'
$env:OLLAMA_NOPRUNE = '1'
$env:OLLAMA_NO_CLOUD = '1'
$env:OLLAMA_NUM_PARALLEL = '1'
$env:OLLAMA_MAX_LOADED_MODELS = '2'
$env:OLLAMA_DEBUG = '0'
$engineLog = Join-Path $Report 'engine.log'
$server = Start-Process -FilePath (Join-Path $Engine 'ollama.exe') -ArgumentList 'serve' -WorkingDirectory $Engine `
  -RedirectStandardError $engineLog -RedirectStandardOutput (Join-Path $Report 'engine.out.log') -PassThru

function Api([string] $Method, [string] $Path, $Body, [int] $TimeoutSec = 60) {
  $json = if ($null -ne $Body) { $Body | ConvertTo-Json -Depth 8 -Compress } else { $null }
  Invoke-RestMethod -Method $Method -Uri "$base$Path" -Body $json -ContentType 'application/json' -TimeoutSec $TimeoutSec
}

# Files of the model store larger than 1 MB: what a deleted model must not leave behind.
function Get-BigBlobs {
  @(Get-ChildItem -Path (Join-Path $modelsDir 'blobs') -File -ErrorAction SilentlyContinue | Where-Object Length -gt 1MB)
}

try {
  $deadline = (Get-Date).AddSeconds(60)
  $version = $null
  while (-not $version -and (Get-Date) -lt $deadline) {
    try { $version = (Api GET '/api/version' $null 5).version } catch { Start-Sleep -Milliseconds 500 }
  }
  Check 'engine starts' ([bool] $version) "version $version on port $port"
  $compute = Select-String -Path $engineLog -Pattern 'inference compute' -SimpleMatch | Select-Object -Last 1
  Note "- note: $(if ($compute) { $compute.Line.Trim() } else { 'no inference compute line in the engine log' })"

  foreach ($model in $Models) {
    Note "`n## $model"
    $source = "$Library/distroclaw/$model-gguf"

    $started = Get-Date
    $pull = Api POST '/api/pull' @{ model = $source; stream = $false; insecure = $false } 3600
    $seconds = [math]::Round(((Get-Date) - $started).TotalSeconds)
    Check 'download from the model library' ($pull.status -eq 'success') "$source in $seconds s"
    Api POST '/api/copy' @{ source = $source; destination = $model } | Out-Null
    Api DELETE '/api/delete' @{ model = $source } | Out-Null
    $names = @((Api GET '/api/tags' $null).models | ForEach-Object name)
    Check 'installed under its catalogue name only' (($names -contains $model) -and ($names -notcontains $source)) ($names -join ', ')
    $size = (Get-BigBlobs | Measure-Object Length -Sum).Sum
    Note "- note: on disk: $([math]::Round($size / 1GB, 2)) GB"

    $chat = Api POST '/api/chat' @{
      model = $model
      messages = @(@{ role = 'user'; content = 'What is 2 + 3? Answer with one short sentence.' })
      stream = $false
      think = $false
      options = @{ num_ctx = 4096 }
    } 900
    $answer = "$($chat.message.content)".Trim()
    Check 'chat answers' ($answer.Length -gt 0) ($answer.Substring(0, [math]::Min(80, $answer.Length)))
    $prefill = if ($chat.prompt_eval_duration) { [math]::Round($chat.prompt_eval_count / ($chat.prompt_eval_duration / 1e9), 1) } else { 0 }
    $generate = if ($chat.eval_duration) { [math]::Round($chat.eval_count / ($chat.eval_duration / 1e9), 1) } else { 0 }
    Note "- note: speed on this runner's processor: prompt $prefill tokens/s, answer $generate tokens/s ($($chat.eval_count) tokens)"

    # Deleted as Settings > Model does it: right after use, while the model is still loaded.
    Api DELETE '/api/delete' @{ model = $model } | Out-Null
    $names = @((Api GET '/api/tags' $null).models | ForEach-Object name)
    Check 'deleted from the model list' ($names -notcontains $model) ($names -join ', ')
    Start-Sleep -Seconds 2
    $left = Get-BigBlobs
    Check 'its files are deleted from disk' ($left.Count -eq 0) (($left | ForEach-Object { "$($_.Name) $([math]::Round($_.Length / 1MB)) MB" }) -join ', ')
  }
  Note "`nAll model checks passed."
}
finally {
  & "$env:SystemRoot\System32\taskkill.exe" /T /F /PID $server.Id 2>&1 | Out-Null
  Start-Sleep -Seconds 2
  Remove-Item -Recurse -Force $modelsDir -ErrorAction SilentlyContinue
}
