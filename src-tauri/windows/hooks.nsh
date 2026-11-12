; DistroClaw's additions to the Windows installer and uninstaller (Tauri NSIS hooks).
;
; Before files are replaced or removed, an AI engine that an earlier run of DistroClaw left
; running is stopped, so its files are not in use. Only processes started from this
; installation's folder are touched, never another copy of Ollama. While DistroClaw itself runs,
; nothing is done here: the installer then asks to close it, and the engine ends with the app.
;
; The PowerShell below, readable (the folder comes in through DC_INSTDIR, so no quoting can
; change the command; CIM, because a 32-bit installer cannot read 64-bit process paths otherwise):
;
;   $d = [IO.Path]::GetFullPath($env:DC_INSTDIR).TrimEnd('\') + '\'
;   $p = Get-CimInstance Win32_Process | Where-Object {
;          $_.ExecutablePath -and $_.ExecutablePath.StartsWith($d, 'OrdinalIgnoreCase') }
;   if ($p | Where-Object { $_.Name -notin 'ollama.exe', 'llama-server.exe' }) { exit 0 }
;   $p | ForEach-Object {
;          Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue
;          Wait-Process -Id $_.ProcessId -Timeout 10 -ErrorAction SilentlyContinue }

!macro DISTROCLAW_STOP_LEFTOVER_ENGINE
  System::Call 'kernel32::SetEnvironmentVariable(t "DC_INSTDIR", t "$INSTDIR")'
  nsExec::Exec `"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "$$d = [IO.Path]::GetFullPath($$env:DC_INSTDIR).TrimEnd('\') + '\'; $$p = Get-CimInstance Win32_Process | Where-Object { $$_.ExecutablePath -and $$_.ExecutablePath.StartsWith($$d, 'OrdinalIgnoreCase') }; if ($$p | Where-Object { $$_.Name -notin 'ollama.exe', 'llama-server.exe' }) { exit 0 }; $$p | ForEach-Object { Stop-Process -Id $$_.ProcessId -Force -ErrorAction SilentlyContinue; Wait-Process -Id $$_.ProcessId -Timeout 10 -ErrorAction SilentlyContinue }"`
  Pop $0
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro DISTROCLAW_STOP_LEFTOVER_ENGINE
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro DISTROCLAW_STOP_LEFTOVER_ENGINE
!macroend
