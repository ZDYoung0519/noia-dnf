# -*- coding: utf-8 -*-
# Build via build-installer.ps1. Only the release executable is shipped.
Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"
!include "FileFunc.nsh"

!define APP_NAME "Noia DNF"
!define ARP_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Noia.DNF"
!define UNINSTALL_EXE "Uninstall Noia DNF.exe"
Var PreviousInstallDir
Var UpdatePID
Name "${APP_NAME}"
OutFile "${OUTPUT_FILE}"
InstallDir "$PROGRAMFILES64\${APP_NAME}"
RequestExecutionLevel admin
SetCompressor /SOLID lzma
AllowSkipFiles off
ManifestDPIAware true

!getdllversion "${APP_EXE}" BinVersion
VIProductVersion "${BinVersion1}.${BinVersion2}.${BinVersion3}.${BinVersion4}"
VIAddVersionKey "ProductName" "${APP_NAME}"
VIAddVersionKey "FileDescription" "${APP_NAME} Setup"
VIAddVersionKey "FileVersion" "${APP_VERSION}"
VIAddVersionKey "ProductVersion" "${APP_VERSION}"
VIAddVersionKey "LegalCopyright" "Noia DNF"

!define MUI_ICON "..\assets\app.ico"
!define MUI_UNICON "..\assets\app.ico"
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipUpdatePage
!insertmacro MUI_PAGE_WELCOME
!define MUI_PAGE_CUSTOMFUNCTION_PRE ChooseInstallDirectory
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN
!define MUI_FINISHPAGE_RUN_TEXT "$(DesktopShortcut)"
!define MUI_FINISHPAGE_RUN_FUNCTION CreateDesktopShortcut
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipUpdatePage
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "SimpChinese"

LangString NeedX64 ${LANG_ENGLISH} "Noia DNF requires x64 Windows."
LangString NeedX64 ${LANG_SIMPCHINESE} "Noia DNF 需要 x64 版 Windows。"
LangString DesktopShortcut ${LANG_ENGLISH} "Create a desktop shortcut"
LangString DesktopShortcut ${LANG_SIMPCHINESE} "创建桌面快捷方式"
LangString FileInUse ${LANG_ENGLISH} "Could not update the application file. Close Noia DNF and retry, or cancel setup."
LangString FileInUse ${LANG_SIMPCHINESE} "无法更新程序文件。请退出 Noia DNF 后重试，或取消安装。"
LangString RemoveFailed ${LANG_ENGLISH} "Could not remove the application file. Close Noia DNF and retry, or cancel."
LangString RemoveFailed ${LANG_SIMPCHINESE} "无法移除程序文件。请退出 Noia DNF 后重试，或取消卸载。"
LangString StartupCleanupFailed ${LANG_ENGLISH} "Windows could not remove the logon task. Check the Task Scheduler service and retry."
LangString StartupCleanupFailed ${LANG_SIMPCHINESE} "无法清理登录启动任务。请确认 Windows 任务计划程序服务可用，然后重试。"

Function .onInit
    ${IfNot} ${IsNativeAMD64}
        MessageBox MB_ICONSTOP|MB_OK "$(NeedX64)" /SD IDOK
        SetErrorLevel 1
        Abort
    ${EndIf}
    SetRegView 64
    SetShellVarContext all
    ReadRegStr $PreviousInstallDir HKLM "${ARP_KEY}" "InstallLocation"
    ${GetParameters} $0
    ClearErrors
    ${GetOptions} $0 "/NOIAUPDATE=" $UpdatePID
    ${IfNot} ${Errors}
        ; /D= specifies the running copy, even if another copy is registered.
        ${If} $UpdatePID == ""
        ${OrIfNot} ${FileExists} "$INSTDIR\noia-dnf.exe"
            SetErrorLevel 1
            Abort
        ${EndIf}
        SetAutoClose true
        Return
    ${EndIf}
    StrCpy $UpdatePID ""
    ${If} $PreviousInstallDir != ""
    ${AndIf} ${FileExists} "$PreviousInstallDir\noia-dnf.exe"
        StrCpy $INSTDIR $PreviousInstallDir
    ${Else}
        StrCpy $PreviousInstallDir ""
    ${EndIf}
FunctionEnd

Function SkipUpdatePage
    ${If} $UpdatePID != ""
        Abort
    ${EndIf}
FunctionEnd

Function ChooseInstallDirectory
    ; Upgrade the registered copy in place, alongside its existing settings.
    ${If} $PreviousInstallDir != ""
    ${OrIf} $UpdatePID != ""
        Abort
    ${EndIf}
FunctionEnd

Function CreateDesktopShortcut
    ClearErrors
    CreateShortcut "$DESKTOP\${APP_NAME}.lnk" "$INSTDIR\noia-dnf.exe"
    ${IfNot} ${Errors}
        WriteRegDWORD HKLM "${ARP_KEY}" "DesktopShortcut" 1
    ${EndIf}
FunctionEnd

Section "Noia DNF"
    ${If} $UpdatePID != ""
        StrCpy $0 $UpdatePID
        System::Call 'kernel32::OpenProcess(i 0x100000, i 0, i r0) p.r1'
        ${If} $1 != 0
            System::Call 'kernel32::WaitForSingleObject(p r1, i 30000) i.r0'
            System::Call 'kernel32::CloseHandle(p r1)'
            ${If} $0 != 0
                MessageBox MB_ICONSTOP|MB_OK "$(FileInUse)" /SD IDOK
                SetErrorLevel 1
                Abort
            ${EndIf}
        ${EndIf}
    ${EndIf}
    SetOutPath "$INSTDIR"
    ; Stage on the same volume so an interrupted upgrade keeps a complete exe.
    GetTempFileName $0 "$INSTDIR"
    File "/oname=$0" "${APP_EXE}"
    retry_replace:
    System::Call 'kernel32::MoveFileExW(w r0, w "$INSTDIR\noia-dnf.exe", i 9) i.r1'
    ${If} $1 == 0
        MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "$(FileInUse)" /SD IDCANCEL IDRETRY retry_replace
        Delete "$0"
        SetErrorLevel 1
        Abort
    ${EndIf}
    ${If} $UpdatePID != ""
    ${AndIf} $INSTDIR != $PreviousInstallDir
        ; A portable copy owns neither an uninstaller nor shell registrations.
        Goto installed
    ${EndIf}
    WriteUninstaller "$INSTDIR\${UNINSTALL_EXE}"
    CreateShortcut "$SMPROGRAMS\${APP_NAME}.lnk" "$INSTDIR\noia-dnf.exe"
    WriteRegStr HKLM "${ARP_KEY}" "DisplayName" "${APP_NAME}"
    WriteRegStr HKLM "${ARP_KEY}" "DisplayVersion" "${APP_VERSION}"
    WriteRegStr HKLM "${ARP_KEY}" "DisplayIcon" '"$INSTDIR\noia-dnf.exe",0'
    WriteRegStr HKLM "${ARP_KEY}" "InstallLocation" "$INSTDIR"
    WriteRegStr HKLM "${ARP_KEY}" "UninstallString" '"$INSTDIR\${UNINSTALL_EXE}"'
    WriteRegStr HKLM "${ARP_KEY}" "QuietUninstallString" '"$INSTDIR\${UNINSTALL_EXE}" /S'
    WriteRegDWORD HKLM "${ARP_KEY}" "NoModify" 1
    WriteRegDWORD HKLM "${ARP_KEY}" "NoRepair" 1
    installed:
SectionEnd

Function .onInstSuccess
    ${If} $UpdatePID != ""
        Exec '"$INSTDIR\noia-dnf.exe"'
    ${EndIf}
FunctionEnd

Section "Uninstall"
    SetRegView 64
    SetShellVarContext all
    ReadRegStr $0 HKLM "${ARP_KEY}" "InstallLocation"
    ${If} $0 != $INSTDIR
        SetErrorLevel 1
        Abort
    ${EndIf}
    retry_delete:
    ; Remove logon tasks owned by this installation, including other users' tasks.
    ; Pass the path as data so quotes and PowerShell syntax cannot change the command.
    System::Call 'kernel32::SetEnvironmentVariableW(w "NOIA_UNINSTALL_EXE", w "$INSTDIR\noia-dnf.exe") i.r0'
    ${If} $0 != 0
        StrCpy $2 "$$ErrorActionPreference='Stop'; $$env:PSModulePath=[IO.Path]::Combine($$PSHOME,'Modules');"
        StrCpy $2 "$2 try { $$s=New-Object -ComObject Schedule.Service; $$s.Connect(); $$f=$$s.GetFolder('\');"
        StrCpy $2 "$2 foreach ($$t in @($$f.GetTasks(1))) {"
        StrCpy $2 "$2 if ($$t.Name -notmatch '^Noia\.DNF\.Logon\.S-1-[0-9-]+$$' -or $$t.Definition.Actions.Count -ne 1) { continue };"
        StrCpy $2 "$2 $$a=$$t.Definition.Actions.Item(1);"
        StrCpy $2 "$2 if ($$a.Path -eq $$env:NOIA_UNINSTALL_EXE -and $$a.Arguments -eq '--autostart') { $$f.DeleteTask($$t.Name,0) }"
        StrCpy $2 "$2 } } catch { exit 1 }"
        nsExec::ExecToStack `"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -NonInteractive -Command "$2"`
        Pop $0
        Pop $1
    ${Else}
        StrCpy $0 1
    ${EndIf}
    ${If} $0 != 0
        MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "$(StartupCleanupFailed)" /SD IDCANCEL IDRETRY retry_delete
        SetErrorLevel 1
        Abort
    ${EndIf}
    ClearErrors
    Delete "$INSTDIR\noia-dnf.exe"
    ${If} ${Errors}
        MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "$(RemoveFailed)" /SD IDCANCEL IDRETRY retry_delete
        SetErrorLevel 1
        Abort
    ${EndIf}
    Delete "$SMPROGRAMS\${APP_NAME}.lnk"
    ReadRegDWORD $0 HKLM "${ARP_KEY}" "DesktopShortcut"
    ${If} $0 == 1
        Delete "$DESKTOP\${APP_NAME}.lnk"
    ${EndIf}
    DeleteRegKey HKLM "${ARP_KEY}"
    Delete "$INSTDIR\${UNINSTALL_EXE}"
    ; Only remove an empty folder. User settings live beside the executable.
    RMDir "$INSTDIR"
SectionEnd
