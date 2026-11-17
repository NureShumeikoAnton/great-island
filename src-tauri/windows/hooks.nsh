; Additions to the Tauri installer (bundle > windows > nsis > installerHooks).
; The file is UTF-8 with BOM: without it NSIS would read the Russian text in the system encoding.

; Take PrtSc screenshots through the island instead of Windows Snipping Tool?
; Asked once: on first install, not in silent mode, and only if PrtSc
; currently belongs to Snipping Tool. "Yes" turns off the Windows setting "Use the Print
; screen button to open screen capture"; the island by default
; takes screenshots on PrtSc as soon as it is free.
!macro NSIS_HOOK_POSTINSTALL
  ; The island starts in the language picked for the installer (1049 = Russian, otherwise English).
  StrCpy $1 "en"
  StrCmp $LANGUAGE 1049 0 +2
  StrCpy $1 "ru"
  WriteRegStr HKCU "Software\Great Island" "Language" $1

  IfSilent bi_prtsc_done
  ReadRegDWORD $0 HKCU "Software\Great Island" "PrintScreenAsked"
  StrCmp $0 "1" bi_prtsc_done
  WriteRegDWORD HKCU "Software\Great Island" "PrintScreenAsked" 1
  ; 0 = PrtSc is already free. No entry = the default value (in Windows 11 it belongs to Snipping Tool).
  ClearErrors
  ReadRegDWORD $0 HKCU "Control Panel\Keyboard" "PrintScreenKeyForSnippingEnabled"
  IfErrors bi_prtsc_ask
  StrCmp $0 "0" bi_prtsc_done
  bi_prtsc_ask:
  ; $LANGUAGE, not LangString: this file is included before the languages are inserted.
  StrCpy $1 "Take screenshots with the PrtSc key through Great Island?$\n$\nRight now PrtSc opens Windows Snipping Tool. It stays on Win+Shift+S. You can change the key in the island's settings, and give PrtSc back to Snipping Tool in Windows settings: Accessibility → Keyboard."
  StrCmp $LANGUAGE 1049 0 bi_prtsc_text_done
  StrCpy $1 "Делать скриншоты клавишей PrtSc через Great Island?$\n$\nСейчас по PrtSc открываются «Ножницы» Windows. Они останутся на Win+Shift+S. Клавишу можно сменить в настройках острова, а вернуть PrtSc «Ножницам» — в параметрах Windows: Специальные возможности → Клавиатура."
  bi_prtsc_text_done:
  MessageBox MB_YESNO|MB_ICONQUESTION "$1" /SD IDNO IDNO bi_prtsc_done
  WriteRegDWORD HKCU "Control Panel\Keyboard" "PrintScreenKeyForSnippingEnabled" 0
  ; Tell the windows the keyboard settings changed (HWND_BROADCAST, WM_SETTINGCHANGE).
  SendMessage 0xFFFF 0x001A 0 "STR:Control Panel\Keyboard" /TIMEOUT=1000
  bi_prtsc_done:
!macroend
