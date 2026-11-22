; Additions to the Tauri installer (bundle > windows > nsis > installerHooks).
; The file is UTF-8 with BOM: without it NSIS would read the Russian and Ukrainian text in the system encoding.

; Take PrtSc screenshots through the island instead of Windows Snipping Tool?
; Asked once: on first install, not in silent mode, and only if PrtSc
; currently belongs to Snipping Tool. "Yes" turns off the Windows setting "Use the Print
; screen button to open screen capture"; the island by default
; takes screenshots on PrtSc as soon as it is free.
!macro NSIS_HOOK_POSTINSTALL
  ; An update keeps the language the island already has: a silent installer knows only the system's one.
  StrCmp $UpdateMode 1 bi_language_done
  ; The island starts in the language picked for the installer (1049 = Russian, 1058 = Ukrainian, otherwise English).
  StrCpy $1 "en"
  StrCmp $LANGUAGE 1049 0 +2
  StrCpy $1 "ru"
  StrCmp $LANGUAGE 1058 0 +2
  StrCpy $1 "uk"
  WriteRegStr HKCU "Software\Great Island" "Language" $1
  bi_language_done:

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
  StrCmp $LANGUAGE 1049 0 bi_prtsc_not_ru
  StrCpy $1 "Делать скриншоты клавишей PrtSc через Great Island?$\n$\nСейчас по PrtSc открываются «Ножницы» Windows. Они останутся на Win+Shift+S. Клавишу можно сменить в настройках острова, а вернуть PrtSc «Ножницам» — в параметрах Windows: Специальные возможности → Клавиатура."
  bi_prtsc_not_ru:
  StrCmp $LANGUAGE 1058 0 bi_prtsc_text_done
  StrCpy $1 "Робити скриншоти клавішею PrtSc через Great Island?$\n$\nЗараз за PrtSc відкриваються «Ножиці» Windows. Вони залишаться на Win+Shift+S. Клавішу можна змінити в налаштуваннях острова, а повернути PrtSc «Ножицям» — у параметрах Windows: Спеціальні можливості → Клавіатура."
  bi_prtsc_text_done:
  MessageBox MB_YESNO|MB_ICONQUESTION "$1" /SD IDNO IDNO bi_prtsc_done
  WriteRegDWORD HKCU "Control Panel\Keyboard" "PrintScreenKeyForSnippingEnabled" 0
  ; Tell the windows the keyboard settings changed (HWND_BROADCAST, WM_SETTINGCHANGE).
  SendMessage 0xFFFF 0x001A 0 "STR:Control Panel\Keyboard" /TIMEOUT=1000
  bi_prtsc_done:

  ; The island in "Open with" for photos and videos, written by the exe itself (it knows the
  ; types); every update writes it again for the new exe.
  ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --register'
  ; Windows lets only the user make an app the default: asked once, on the first install, and
  ; "Yes" opens the island's page among Windows' Default apps.
  IfSilent bi_viewer_done
  ReadRegDWORD $0 HKCU "Software\Great Island" "ViewerAsked"
  StrCmp $0 "1" bi_viewer_done
  WriteRegDWORD HKCU "Software\Great Island" "ViewerAsked" 1
  StrCpy $1 "Open photos and videos in Great Island?$\n$\nThe island is now in the $\"Open with$\" menu for photos and videos. $\"Yes$\" opens Windows settings, where it can be made the default app for each file type."
  StrCmp $LANGUAGE 1049 0 bi_viewer_not_ru
  StrCpy $1 "Открывать фото и видео в Great Island?$\n$\nОстров добавлен в меню «Открыть с помощью» для фото и видео. «Да» откроет параметры Windows, где его можно назначить приложением по умолчанию для каждого типа файлов."
  bi_viewer_not_ru:
  StrCmp $LANGUAGE 1058 0 bi_viewer_text_done
  StrCpy $1 "Відкривати фото та відео в Great Island?$\n$\nОстрів додано до меню «Відкрити за допомогою» для фото та відео. «Так» відкриє параметри Windows, де його можна призначити застосунком за замовчуванням для кожного типу файлів."
  bi_viewer_text_done:
  MessageBox MB_YESNO|MB_ICONQUESTION "$1" /SD IDNO IDNO bi_viewer_done
  ExecShell "open" "ms-settings:defaultapps?registeredAppUser=Great%20Island"
  bi_viewer_done:
!macroend

; Out of "Open with" and Default apps, while the exe is still there to say what it wrote. Not
; on an update: the new version registers right after, and the user's default stays.
!macro NSIS_HOOK_PREUNINSTALL
  StrCmp $UpdateMode 1 bi_unregister_done
  ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --unregister'
  bi_unregister_done:
!macroend
