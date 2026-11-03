; Windows Network Manager installer hooks.
; Monitor history is written to "monitor-data" next to the program. Upgrades
; keep it; uninstalling removes it only when "Delete the application data" is
; checked, the same choice that removes the configuration folder.
!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    RMDir /r "$INSTDIR\monitor-data"
    RMDir "$INSTDIR"
  ${EndIf}
!macroend
