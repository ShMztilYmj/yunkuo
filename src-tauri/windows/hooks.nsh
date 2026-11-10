; 云阔安装器钩子（tauri.conf.json > bundle.windows.nsis.installerHooks）。
; 本文件在模板定义 PRODUCTNAME 等常量之前被包含，宏内引用的常量和变量在插入时才展开，顶层不要使用它们。
; 须保存为带 BOM 的 UTF-8。

!macro NSIS_HOOK_POSTINSTALL
  ; 主程序无法打开时的离线恢复入口。
  ${If} $NoShortcutMode <> 1
    CreateShortcut "$SMPROGRAMS\${PRODUCTNAME} 恢复联网.lnk" "$INSTDIR\resources\recovery\恢复联网.cmd" "" "$INSTDIR\${MAINBINARYNAME}.exe" 0
  ${EndIf}
  ; 上次卸载清理失败时保留的恢复工具副本，由新安装的版本接替（旧品牌目录一并清掉）。
  RMDir /r "$LOCALAPPDATA\Yunkuo\recovery"
  RMDir /r "$LOCALAPPDATA\ClaudeOneClick\recovery"
!macroend

; 卸载前清理：在删除任何程序文件之前解除网络保护、移除安全服务、还原系统代理与默认浏览器。
; 带 /UPDATE 的覆盖升级不清理；升级时选择“先卸载旧版本”与正常卸载一样完整清理，之后需在设置中重新安装安全服务。
!macro NSIS_HOOK_PREUNINSTALL
  ${If} $UpdateMode <> 1
    ; 新旧两个服务名都算「装了安全服务」：改名前的版本可能还留着旧注册项。
    StrCpy $R8 0
    ClearErrors
    ReadRegStr $R9 HKLM "SYSTEM\CurrentControlSet\Services\YunkuoService" "ImagePath"
    ${IfNot} ${Errors}
      StrCpy $R8 1
    ${EndIf}
    ClearErrors
    ReadRegStr $R9 HKLM "SYSTEM\CurrentControlSet\Services\ClaudeOneClickService" "ImagePath"
    ${IfNot} ${Errors}
      StrCpy $R8 1
    ${EndIf}
    ${If} $R8 = 1
      ${IfNot} ${Cmd} `MessageBox MB_OKCANCEL|MB_ICONINFORMATION "卸载会关闭云阔、解除网络保护并移除安全服务，随后需要管理员授权。$\r$\n$\r$\n请先关闭 Claude、Codex 等 AI 客户端，避免它们在保护解除后直连网络。" /SD IDOK IDOK`
        Abort "已取消卸载，网络保护和安全服务保持不变。"
      ${EndIf}
    ${EndIf}

    InitPluginsDir
    Delete "$PLUGINSDIR\cleanup-result.txt"
    DetailPrint "正在解除网络保护并还原系统代理与默认浏览器……"
    ClearErrors
    ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --uninstall-cleanup "$PLUGINSDIR\cleanup-result.txt"' $R8
    ${If} ${Errors}
      StrCpy $R8 -1
    ${EndIf}

    ${If} $R8 = 0
      RMDir /r "$LOCALAPPDATA\Yunkuo\recovery"
      RMDir /r "$LOCALAPPDATA\ClaudeOneClick\recovery"
    ${Else}
      StrCpy $R7 "清理程序无法运行，主程序文件可能已损坏。"
      ClearErrors
      FileOpen $R6 "$PLUGINSDIR\cleanup-result.txt" r
      ${IfNot} ${Errors}
        FileReadUTF16LE $R6 $R5 1000
        FileClose $R6
        ${If} $R5 != ""
          StrCpy $R7 $R5
        ${EndIf}
      ${EndIf}
      DetailPrint "卸载前清理未完成：$R7"
      ${IfNot} ${Cmd} `MessageBox MB_YESNO|MB_ICONEXCLAMATION|MB_DEFBUTTON2 "卸载前清理未完成：$R7$\r$\n$\r$\n选择“否”取消卸载（推荐），处理后重新卸载。$\r$\n选择“是”仍然卸载：恢复工具会保留在 $LOCALAPPDATA\Yunkuo\recovery，断网时运行其中的“恢复联网.cmd”。" /SD IDNO IDYES`
        Abort "已取消卸载。"
      ${EndIf}
      CreateDirectory "$LOCALAPPDATA\Yunkuo\recovery"
      CopyFiles /SILENT "$INSTDIR\resources\recovery\*.*" "$LOCALAPPDATA\Yunkuo\recovery"
    ${EndIf}
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
    Delete "$SMPROGRAMS\${PRODUCTNAME} 恢复联网.lnk"
    ; 清理未完成而仍然卸载时，入口改指向保留的恢复工具副本。
    ${If} ${FileExists} "$LOCALAPPDATA\Yunkuo\recovery\恢复联网.cmd"
      CreateShortcut "$SMPROGRAMS\${PRODUCTNAME} 恢复联网.lnk" "$LOCALAPPDATA\Yunkuo\recovery\恢复联网.cmd"
    ${ElseIf} ${FileExists} "$LOCALAPPDATA\ClaudeOneClick\recovery\恢复联网.cmd"
      CreateShortcut "$SMPROGRAMS\${PRODUCTNAME} 恢复联网.lnk" "$LOCALAPPDATA\ClaudeOneClick\recovery\恢复联网.cmd"
    ${EndIf}
  ${EndIf}
!macroend
