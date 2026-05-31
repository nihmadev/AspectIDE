!macro NSIS_HOOK_PREINSTALL
  DetailPrint "Preparing Lux IDE installation"
!macroend

!macro NSIS_HOOK_POSTINSTALL
  WriteRegStr HKCU "Software\Lux IDE" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "Software\Classes\lux" "URL Protocol" ""
  WriteRegStr HKCU "Software\Classes\lux\shell\open\command" "" '"$INSTDIR\Lux IDE.exe" "%1"'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  MessageBox MB_YESNO|MB_ICONQUESTION "Also remove Lux IDE caches, settings, logs, downloaded extensions, indexes, and recent workspaces? User project files will not be touched." IDNO skipFullClean
    RMDir /r "$LOCALAPPDATA\dev.lux.ide"
    RMDir /r "$APPDATA\dev.lux.ide"
    RMDir /r "$LOCALAPPDATA\Lux IDE"
    RMDir /r "$APPDATA\Lux IDE"
  skipFullClean:
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  DeleteRegKey HKCU "Software\Lux IDE"
  DeleteRegKey HKCU "Software\Classes\lux"
!macroend
