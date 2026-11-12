# Installs DistroClaw-Setup.exe, tests the installed app offline, uninstalls it and checks that
# nothing is left behind and no personal file was touched. Used by .github/workflows/windows.yml.
#
# It changes the machine it runs on (installs and removes the app, turns the firewall on, adds
# firewall rules), so run it only on a throwaway Windows machine such as a CI runner:
#
#   pwsh scripts/windows-smoke-test.ps1 -Installer <DistroClaw-Setup.exe> -Report <folder>
#
# Every check is written to <folder>/smoke-test.md; the script fails on the first one that fails.

param(
  [Parameter(Mandatory)] [string] $Installer,
  [Parameter(Mandatory)] [string] $Report
)

$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $Report | Out-Null
$log = Join-Path $Report 'smoke-test.md'
Set-Content -Path $log -Value "# Windows smoke test`n" -Encoding utf8

function Note([string] $Text) {
  Add-Content -Path $log -Value $Text -Encoding utf8
  Write-Host $Text
}

function Check([string] $What, [bool] $Ok, [string] $Detail = '') {
  $line = if ($Ok) { "- passed: $What" } else { "- FAILED: $What" }
  if ($Detail) { $line += " ($Detail)" }
  Note $line
  if (-not $Ok) { throw "smoke test failed: $What $Detail" }
}

$uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\DistroClaw'
$dataDir = Join-Path $env:LOCALAPPDATA 'com.distroclaw.app'
$roamingDir = Join-Path $env:APPDATA 'com.distroclaw.app'
$startMenu = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'

# The process with this id and everything it started.
function Get-Tree([int] $Root) {
  $all = @(Get-CimInstance Win32_Process)
  $ids = [System.Collections.Generic.HashSet[int]]::new()
  [void] $ids.Add($Root)
  do {
    $added = $false
    foreach ($p in $all) {
      if ($ids.Contains([int] $p.ParentProcessId) -and $ids.Add([int] $p.ProcessId)) { $added = $true }
    }
  } while ($added)
  $all | Where-Object { $ids.Contains([int] $_.ProcessId) }
}

# Processes running from the install folder (the app, the engine and its runners).
function Get-Installed([string] $Dir) {
  $prefix = $Dir.TrimEnd('\') + '\'
  @(Get-CimInstance Win32_Process | Where-Object {
      $_.ExecutablePath -and $_.ExecutablePath.StartsWith($prefix, 'OrdinalIgnoreCase') })
}

# Watches the given processes for $Seconds and returns every TCP connection to a remote address,
# with the process (and WebView2 process type) that opened it and the server's host name.
function Watch-Connections([scriptblock] $Pids, [int] $Seconds, [System.Diagnostics.Process] $Until) {
  $seen = @{}
  $end = (Get-Date).AddSeconds($Seconds)
  while ((Get-Date) -lt $end) {
    $ids = @(& $Pids)
    foreach ($c in @(Get-NetTCPConnection -ErrorAction SilentlyContinue)) {
      if ($ids -contains $c.OwningProcess -and $c.State -ne 'Listen' -and
          $c.RemoteAddress -notin '127.0.0.1', '::1', '0.0.0.0', '::') {
        $proc = Get-CimInstance Win32_Process -Filter "ProcessId = $($c.OwningProcess)" -ErrorAction SilentlyContinue
        $kind = if ("$($proc.CommandLine)" -match '--utility-sub-type=(\S+)') { " $($Matches[1])" }
          elseif ("$($proc.CommandLine)" -match '--type=(\S+)') { " $($Matches[1])" } else { '' }
        $seen["$($proc.Name)${kind}: $($c.RemoteAddress):$($c.RemotePort)"] = $c.RemoteAddress
      }
    }
    if ($Until -and $Until.HasExited) { break }
    Start-Sleep -Milliseconds 500
  }
  foreach ($key in @($seen.Keys)) {
    $host_ = try { (Resolve-DnsName -Name $seen[$key] -Type PTR -QuickTimeout -ErrorAction Stop | Select-Object -First 1).NameHost } catch { $null }
    if ($host_) { "$key ($host_)" } else { $key }
  }
}

# A personal file: it must be the same after install, use and uninstall.
$documents = [Environment]::GetFolderPath('MyDocuments')
$sentinel = Join-Path $documents 'distroclaw-smoke-sentinel.txt'
Set-Content -Path $sentinel -Value "personal file $(New-Guid)" -Encoding utf8
$sentinelHash = (Get-FileHash $sentinel).Hash

Note "Installer: $((Get-Item $Installer).Length / 1MB -as [int]) MB, SHA-256 $((Get-FileHash $Installer).Hash.ToLower())"
Note "Windows: $((Get-CimInstance Win32_OperatingSystem).Caption) $([Environment]::OSVersion.Version)`n"

# ---------- 1. Silent install ----------
Note '## Install'
$setup = Start-Process -FilePath $Installer -ArgumentList '/S' -Wait -PassThru
Check 'installer exits with 0' ($setup.ExitCode -eq 0) "exit code $($setup.ExitCode)"
Check 'uninstall entry for the current user' (Test-Path $uninstallKey)
$entry = Get-ItemProperty $uninstallKey
$installDir = $entry.InstallLocation.Trim('"')
$exe = Join-Path $installDir $entry.MainBinaryName
Check 'installed for this user, without admin rights' ($installDir.StartsWith($env:LOCALAPPDATA, 'OrdinalIgnoreCase')) $installDir
Check 'app executable' (Test-Path $exe) $exe
Check 'uninstaller' (Test-Path (Join-Path $installDir 'uninstall.exe'))
Check 'AI engine' (Test-Path (Join-Path $installDir 'engine\ollama.exe'))
Check 'Vulkan runtime of the engine' (Test-Path (Join-Path $installDir 'engine\lib\ollama\vulkan\ggml-vulkan.dll'))
$cuda = @(Get-ChildItem -Path (Join-Path $installDir 'engine') -Recurse -Directory -Filter 'cuda_*')
Check 'no CUDA runtime' ($cuda.Count -eq 0) ($cuda.FullName -join ', ')
$shortcut = Get-ChildItem -Path $startMenu -Recurse -Filter 'DistroClaw.lnk' | Select-Object -First 1
Check 'Start Menu shortcut' ($null -ne $shortcut) "$($shortcut.FullName)"
$engineSig = Get-AuthenticodeSignature (Join-Path $installDir 'engine\ollama.exe')
Check 'engine signature valid' ($engineSig.Status -eq 'Valid') "$($engineSig.Status), $($engineSig.SignerCertificate.Subject)"
$appSig = Get-AuthenticodeSignature $exe
Note "- note: app executable signature: $($appSig.Status) (unsigned builds show SmartScreen warnings)"
Note "- note: desktop shortcut after a silent install: $(Test-Path (Join-Path ([Environment]::GetFolderPath('Desktop')) 'DistroClaw.lnk'))"

# ---------- 2. Offline self-test ----------
Note "`n## Offline self-test"
Set-NetFirewallProfile -Profile Domain, Private, Public -Enabled True
$blocked = @($exe, (Join-Path $installDir 'engine\ollama.exe'), (Join-Path $installDir 'engine\lib\ollama\llama-server.exe'))
foreach ($program in $blocked) {
  New-NetFirewallRule -DisplayName "DistroClaw smoke test: block $(Split-Path $program -Leaf)" `
    -Direction Outbound -Program $program -Action Block | Out-Null
}
Note "- note: outbound traffic blocked for: $(($blocked | ForEach-Object { Split-Path $_ -Leaf }) -join ', ')"
$selfTestReport = Join-Path $Report 'self-test.json'
$selfTest = Start-Process -FilePath $exe -ArgumentList '--self-test', "`"$selfTestReport`"" -PassThru
$remote = Watch-Connections -Pids { @($selfTest.Id) + @(Get-Installed $installDir | ForEach-Object ProcessId) } -Seconds 90 -Until $selfTest
$selfTest.WaitForExit()
Check 'self-test exits with 0' ($selfTest.ExitCode -eq 0) "exit code $($selfTest.ExitCode)"
$result = Get-Content $selfTestReport -Raw | ConvertFrom-Json
foreach ($name in 'database', 'documents', 'engine') {
  $c = $result.checks | Where-Object name -eq $name
  Check "self-test: $name" ($c.result -eq 'passed') "$($c.result): $($c.detail)"
}
Check 'no connection to another computer during the self-test' ($remote.Count -eq 0) ($remote -join ', ')

# ---------- 3. Normal start, offline ----------
Note "`n## Normal start"
$app = Start-Process -FilePath $exe -PassThru
$remote = Watch-Connections -Pids { @(Get-Tree $app.Id | ForEach-Object ProcessId) } -Seconds 60
Check 'app keeps running' (-not $app.HasExited)
$tree = @(Get-Tree $app.Id)
Check 'WebView2 started' (@($tree | Where-Object Name -eq 'msedgewebview2.exe').Count -gt 0)
Check 'engine started' (@($tree | Where-Object Name -eq 'ollama.exe').Count -gt 0)
Check 'no connection to another computer while the app runs' ($remote.Count -eq 0) ($remote -join ', ')
Check 'data folder in the local profile' (Test-Path $dataDir) $dataDir
Check 'nothing in the roaming profile' (-not (Test-Path $roamingDir)) $roamingDir

# A picture of the screen with the app open, as evidence that the window renders on Windows.
try {
  Add-Type -AssemblyName System.Windows.Forms, System.Drawing
  $bounds = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
  $bitmap = [System.Drawing.Bitmap]::new($bounds.Width, $bounds.Height)
  $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
  $graphics.CopyFromScreen($bounds.Location, [System.Drawing.Point]::Empty, $bounds.Size)
  $bitmap.Save((Join-Path $Report 'app-window.png'), [System.Drawing.Imaging.ImageFormat]::Png)
  $graphics.Dispose(); $bitmap.Dispose()
  Note '- note: screenshot of the running app: app-window.png'
} catch {
  Note "- note: no screenshot ($($_.Exception.Message))"
}

# Ended like Task Manager does: the engine must end with the app (Windows job).
Stop-Process -Id $app.Id -Force
Start-Sleep -Seconds 5
$left = @(Get-Installed $installDir)
Check 'engine ends when the app is ended' ($left.Count -eq 0) (($left | ForEach-Object Name) -join ', ')

# ---------- 4. Uninstall while the app runs ----------
Note "`n## Uninstall"
$app = Start-Process -FilePath $exe -PassThru
Start-Sleep -Seconds 15
Check 'engine running before the uninstall' (@(Get-Installed $installDir | Where-Object Name -eq 'ollama.exe').Count -gt 0)
# The uninstaller copies itself to the temp folder and continues from there, so wait for its
# entry to disappear rather than for the first process.
Start-Process -FilePath (Join-Path $installDir 'uninstall.exe') -ArgumentList '/S' | Out-Null
$deadline = (Get-Date).AddMinutes(3)
while ((Test-Path $uninstallKey) -and (Get-Date) -lt $deadline) { Start-Sleep -Seconds 2 }
while ((Get-Process -Name 'Au_*' -ErrorAction SilentlyContinue) -and (Get-Date) -lt $deadline) { Start-Sleep -Seconds 1 }
Start-Sleep -Seconds 3
Check 'uninstall entry removed' (-not (Test-Path $uninstallKey))
$leftFiles = @(Get-ChildItem -Path $installDir -Recurse -Force -ErrorAction SilentlyContinue)
Check 'install folder removed' (-not (Test-Path $installDir)) (($leftFiles | Select-Object -First 10 -ExpandProperty FullName) -join ', ')
Check 'Start Menu shortcut removed' (-not (Get-ChildItem -Path $startMenu -Recurse -Filter 'DistroClaw.lnk'))
Check 'desktop shortcut removed' (-not (Test-Path (Join-Path ([Environment]::GetFolderPath('Desktop')) 'DistroClaw.lnk')))
$left = @(Get-Installed $installDir)
Check 'no app or engine process left' ($left.Count -eq 0) (($left | ForEach-Object Name) -join ', ')
Check 'app data kept (the "delete app data" box is off by default)' (Test-Path $dataDir)
Check 'personal file untouched' ((Test-Path $sentinel) -and (Get-FileHash $sentinel).Hash -eq $sentinelHash) $sentinel

Remove-Item $sentinel
Get-NetFirewallRule -DisplayName 'DistroClaw smoke test: *' | Remove-NetFirewallRule
Note "`nAll checks passed."
