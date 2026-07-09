!macro NSIS_HOOK_PREINSTALL
  DetailPrint "Preparing AspectIDE installation"
!macroend

!macro NSIS_HOOK_POSTINSTALL
  WriteRegStr HKCU "Software\AspectIDE" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "Software\Classes\aspect" "URL Protocol" ""
  WriteRegStr HKCU "Software\Classes\aspect\shell\open\command" "" '"$INSTDIR\AspectIDE.exe" "%1"'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  MessageBox MB_YESNO|MB_ICONQUESTION "Also remove AspectIDE caches, settings, logs, downloaded extensions, indexes, and recent workspaces? User project files will not be touched." IDNO skipFullClean
    RMDir /r "$LOCALAPPDATA\com.aspect.ide"
    RMDir /r "$APPDATA\com.aspect.ide"
    RMDir /r "$LOCALAPPDATA\AspectIDE"
    RMDir /r "$APPDATA\AspectIDE"
  skipFullClean:
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  DeleteRegKey HKCU "Software\AspectIDE"
  DeleteRegKey HKCU "Software\Classes\aspect"
!macroend
