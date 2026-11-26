; Installer of ADB Scan for Windows 10 and 11 (x64), built with NSIS 3.
;
;   makensis -INPUTCHARSET UTF8 -DVERSION=1.2.0 -DSOURCE=<folder> -DOUTFILE=<setup.exe> packaging/windows/adb-scan.nsi
;
; The file is UTF-8 with a BOM: without it, makensis on Windows reads it in
; the system's code page and stops at the first “ or ” (“Bad text encoding”).
;
; SOURCE is the assembled folder (adb-scan.exe, its DLLs, lib/, share/,
; platform-tools/ and scrcpy/), as the Windows workflow builds it. Paths in
; this script are relative to its folder (makensis works from there), so give
; SOURCE and OUTFILE as absolute paths.
;
; It installs for the current user only, in %LOCALAPPDATA%\Programs\ADB Scan:
; no administrator rights and no UAC prompt. It adds a Start menu shortcut
; (and, if chosen, one on the desktop) and an entry in Settings → Apps to
; uninstall it. Installing a newer version over an older one replaces it; the
; settings and history (in %APPDATA% and %LOCALAPPDATA%) are kept, also when
; uninstalling.

Unicode true
ManifestDPIAware true
SetCompressor /SOLID lzma
RequestExecutionLevel user

!ifndef VERSION
  !error "Pass -DVERSION=X.Y.Z"
!endif
!ifndef SOURCE
  !error "Pass -DSOURCE=<folder with adb-scan.exe>"
!endif
!ifndef OUTFILE
  !define OUTFILE "ADB-Scan-${VERSION}-setup-x64.exe"
!endif
; Windows wants four numbers in the version information: 1.2.3-rc.1 → 1.2.3.0.
!searchparse /noerrors "${VERSION}" "" VER_MAJOR "." VER_MINOR "." VER_PATCH
!searchparse /noerrors "${VER_PATCH}" "" VER_PATCH_NUM "-"
!ifdef VER_PATCH_NUM
  !define VER_NUM "${VER_MAJOR}.${VER_MINOR}.${VER_PATCH_NUM}.0"
!else
  !define VER_NUM "${VER_MAJOR}.${VER_MINOR}.${VER_PATCH}.0"
!endif

!define APP "ADB Scan"
!define EXE "adb-scan.exe"
!define PUBLISHER "LubrieDev"
!define HOMEPAGE "https://github.com/LubrieDev/ADB-Scan"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP}"
; The same value ADB Scan writes for “Start with the system” (crates/app/src/win.rs).
!define RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"

!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"
!include "x64.nsh"
!include "WinVer.nsh"

Name "${APP}"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\${APP}"
InstallDirRegKey HKCU "${UNINSTALL_KEY}" "InstallLocation"
BrandingText "${APP} ${VERSION}"
ShowInstDetails nevershow
ShowUninstDetails nevershow

VIProductVersion "${VER_NUM}"
VIFileVersion "${VER_NUM}"
VIAddVersionKey /LANG=0 "ProductName" "${APP}"
VIAddVersionKey /LANG=0 "ProductVersion" "${VERSION}"
VIAddVersionKey /LANG=0 "FileVersion" "${VERSION}"
VIAddVersionKey /LANG=0 "FileDescription" "${APP} installer"
VIAddVersionKey /LANG=0 "CompanyName" "${PUBLISHER}"
VIAddVersionKey /LANG=0 "LegalCopyright" "2026, ${PUBLISHER} · GPL-3.0-or-later"

; -- pages -----------------------------------------------------------------------

!define MUI_ICON "../../assets/windows/adb-scan.ico"
!define MUI_UNICON "../../assets/windows/adb-scan.ico"
!define MUI_ABORTWARNING

!insertmacro MUI_PAGE_WELCOME
; The GPL does not have to be accepted to use the program: it is shown for
; information, with “Next” instead of “I Agree”.
!define MUI_LICENSEPAGE_TEXT_BOTTOM "$(LicenseNote)"
!define MUI_LICENSEPAGE_BUTTON "$(^NextBtn)"
!insertmacro MUI_PAGE_LICENSE "../../LICENSE"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\${EXE}"
!define MUI_FINISHPAGE_RUN_TEXT "$(RunApp)"
; The second check box of the last page: a shortcut on the desktop.
!define MUI_FINISHPAGE_SHOWREADME ""
!define MUI_FINISHPAGE_SHOWREADME_TEXT "$(DesktopShortcut)"
!define MUI_FINISHPAGE_SHOWREADME_NOTCHECKED
!define MUI_FINISHPAGE_SHOWREADME_FUNCTION CreateDesktopShortcut
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

; The installer speaks the language of Windows: Spanish or, otherwise, English.
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "Spanish"

LangString LicenseNote ${LANG_ENGLISH} "ADB Scan is free software under the GPL 3.0: you do not have to accept anything to use it."
LangString LicenseNote ${LANG_SPANISH} "ADB Scan es software libre con licencia GPL 3.0: no tienes que aceptar nada para usarlo."
LangString RunApp ${LANG_ENGLISH} "Open ADB Scan"
LangString RunApp ${LANG_SPANISH} "Abrir ADB Scan"
LangString DesktopShortcut ${LANG_ENGLISH} "Create a shortcut on the desktop"
LangString DesktopShortcut ${LANG_SPANISH} "Crear un acceso directo en el escritorio"
LangString NeedsWindows10 ${LANG_ENGLISH} "ADB Scan needs Windows 10 or 11, 64-bit."
LangString NeedsWindows10 ${LANG_SPANISH} "ADB Scan necesita Windows 10 u 11 de 64 bits."
LangString AppRunning ${LANG_ENGLISH} "ADB Scan is open. Close it (and the screen mirroring window, if there is one) and press Retry."
LangString FolderInUse ${LANG_ENGLISH} "That folder already has other files. Choose an empty folder or the one where ADB Scan is installed."
LangString FolderInUse ${LANG_SPANISH} "Esa carpeta ya tiene otros archivos. Elige una carpeta vacía o la carpeta donde está instalado ADB Scan."
LangString AppRunning ${LANG_SPANISH} "ADB Scan está abierto. Ciérralo (y la ventana de duplicar pantalla, si hay una) y pulsa Reintentar."

; -- checks shared by the installer and the uninstaller --------------------------

; A running program cannot be opened for writing: that tells whether
; ADB Scan or scrcpy is still open from this folder. adb's server (started by
; ADB Scan, it keeps running after ADB Scan closes) is stopped instead,
; because the user cannot see it. A silent install (ADB Scan's “Update now”
; starts it and then quits) first waits up to 20 s for ADB Scan to close.
!macro CLOSE_RUNNING UN
Function ${UN}CloseRunning
  StrCpy $1 0
  retry:
  ${If} ${FileExists} "$INSTDIR\${EXE}"
    ClearErrors
    FileOpen $0 "$INSTDIR\${EXE}" a
    ; Testing the error flag clears it: one test, everything inside.
    ${If} ${Errors}
      ${If} ${Silent}
      ${AndIf} $1 < 40
        IntOp $1 $1 + 1
        Sleep 500
        Goto retry
      ${EndIf}
      MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "$(AppRunning)" /SD IDCANCEL IDRETRY retry
      Abort
    ${EndIf}
    FileClose $0
  ${EndIf}
  ${If} ${FileExists} "$INSTDIR\scrcpy\scrcpy.exe"
    ClearErrors
    FileOpen $0 "$INSTDIR\scrcpy\scrcpy.exe" a
    ${If} ${Errors}
      MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "$(AppRunning)" /SD IDCANCEL IDRETRY retry
      Abort
    ${EndIf}
    FileClose $0
  ${EndIf}
  ${If} ${FileExists} "$INSTDIR\platform-tools\adb.exe"
    ClearErrors
    FileOpen $0 "$INSTDIR\platform-tools\adb.exe" a
    ${If} ${Errors}
      nsExec::Exec '"$INSTDIR\platform-tools\adb.exe" kill-server'
      Pop $0
      Sleep 1500
    ${Else}
      FileClose $0
    ${EndIf}
  ${EndIf}
FunctionEnd
!macroend
!insertmacro CLOSE_RUNNING ""
!insertmacro CLOSE_RUNNING "un."

; Removes what the installer puts in $INSTDIR, by name: never the whole folder,
; in case it was chosen by hand and holds other files.
!macro REMOVE_FILES
  Delete "$INSTDIR\${EXE}"
  Delete "$INSTDIR\*.dll"
  Delete "$INSTDIR\adb-scan.ico"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\LICENSE-lucide"
  RMDir /r "$INSTDIR\lib"
  RMDir /r "$INSTDIR\share"
  RMDir /r "$INSTDIR\platform-tools"
  RMDir /r "$INSTDIR\scrcpy"
!macroend

; -- installer -------------------------------------------------------------------

; The installer only writes to (and cleans) an empty or missing folder, or one
; that already holds ADB Scan: never a folder of other files, where cleaning
; an older version could take its DLLs or its lib\ and share\ folders.
; $R9 = 1 if $INSTDIR can be used.
Function CheckFolder
  StrCpy $R9 1
  ${If} ${FileExists} "$INSTDIR\${EXE}"
  ${OrIfNot} ${FileExists} "$INSTDIR\*.*"
    Return
  ${EndIf}
  FindFirst $R7 $R8 "$INSTDIR\*.*"
  ${DoWhile} $R8 != ""
    ${If} $R8 != "."
    ${AndIf} $R8 != ".."
      StrCpy $R9 0
      ${Break}
    ${EndIf}
    FindNext $R7 $R8
  ${Loop}
  FindClose $R7
FunctionEnd

; Keeps “Next” disabled on the folder page while the folder cannot be used.
Function .onVerifyInstDir
  Call CheckFolder
  ${If} $R9 != 1
    Abort
  ${EndIf}
FunctionEnd

Function .onInit
  ${IfNot} ${RunningX64}
  ${OrIfNot} ${AtLeastWin10}
    MessageBox MB_OK|MB_ICONSTOP "$(NeedsWindows10)" /SD IDOK
    Abort
  ${EndIf}
  SetRegView 64
FunctionEnd

Section "ADB Scan"
  SetShellVarContext current
  ; Also for a silent install with /D=, which skips the folder page.
  Call CheckFolder
  ${If} $R9 != 1
    MessageBox MB_OK|MB_ICONSTOP "$(FolderInUse)" /SD IDOK
    Abort
  ${EndIf}
  Call CloseRunning
  ; An older version's DLLs and tools go first: the new ones may be other
  ; versions or have other names.
  ${If} ${FileExists} "$INSTDIR\${EXE}"
    !insertmacro REMOVE_FILES
  ${EndIf}

  SetOutPath "$INSTDIR"
  File /r "${SOURCE}/*"
  File "/oname=adb-scan.ico" "../../assets/windows/adb-scan.ico"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  CreateShortcut "$SMPROGRAMS\${APP}.lnk" "$INSTDIR\${EXE}" "" "$INSTDIR\adb-scan.ico" 0

  ; Settings → Apps.
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "${APP}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "${PUBLISHER}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\adb-scan.ico"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNINSTALL_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegStr HKCU "${UNINSTALL_KEY}" "URLInfoAbout" "${HOMEPAGE}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "HelpLink" "${HOMEPAGE}/issues"
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "EstimatedSize" "$0"
SectionEnd

; /RUN (from “Update now”): open ADB Scan again once installed.
Function .onInstSuccess
  ${GetParameters} $0
  ClearErrors
  ${GetOptions} $0 "/RUN" $1
  ${IfNot} ${Errors}
    Exec '"$INSTDIR\${EXE}"'
  ${EndIf}
FunctionEnd

Function CreateDesktopShortcut
  SetShellVarContext current
  CreateShortcut "$DESKTOP\${APP}.lnk" "$INSTDIR\${EXE}" "" "$INSTDIR\adb-scan.ico" 0
FunctionEnd

; -- uninstaller -----------------------------------------------------------------

Function un.onInit
  SetRegView 64
FunctionEnd

Section "Uninstall"
  SetShellVarContext current
  Call un.CloseRunning

  Delete "$SMPROGRAMS\${APP}.lnk"
  Delete "$DESKTOP\${APP}.lnk"
  ; “Start with the system”, only if it points to this copy.
  ReadRegStr $0 HKCU "${RUN_KEY}" "${APP}"
  ${If} $0 == '"$INSTDIR\${EXE}"'
    DeleteRegValue HKCU "${RUN_KEY}" "${APP}"
  ${EndIf}
  DeleteRegKey HKCU "${UNINSTALL_KEY}"

  ${If} ${FileExists} "$INSTDIR\${EXE}"
    !insertmacro REMOVE_FILES
  ${EndIf}
  Delete "$INSTDIR\uninstall.exe"
  ; Only if nothing else is left in it.
  RMDir "$INSTDIR"
SectionEnd
