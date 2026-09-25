!macro NSIS_HOOK_POSTINSTALL
  nsExec::ExecToLog '"$INSTDIR\helper\iran-split-helper.exe" --install --mihomo "$INSTDIR\dependencies\mihomo.exe" --staging-dir "$PROGRAMDATA\iran-split\staging" --tun-name clash-iran'
  Pop $R0
  StrCmp $R0 "0" helper_install_completed
  DetailPrint "BiFlow Helper installation failed (exit: $R0)."
  IfSilent +2
    MessageBox MB_OK|MB_ICONSTOP "BiFlow Helper installation failed (exit: $R0). The application is not ready to connect."
  SetErrorLevel 1
  Abort "BiFlow Helper installation failed"
  helper_install_completed:
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  nsExec::ExecToLog '"$INSTDIR\helper\iran-split-helper.exe" --uninstall'
  Pop $R0
  StrCmp $R0 "0" helper_uninstall_completed
  DetailPrint "BiFlow Helper removal failed (exit: $R0)."
  IfSilent +2
    MessageBox MB_OK|MB_ICONSTOP "BiFlow Helper removal failed (exit: $R0). The application was not fully removed."
  SetErrorLevel 1
  Abort "BiFlow Helper removal failed"
  helper_uninstall_completed:
!macroend
