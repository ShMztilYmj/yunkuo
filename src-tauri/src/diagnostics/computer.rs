use crate::platform::files::{self, environment_root as root, error};
use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use tauri::Manager;

const READ: &str = r#"$ErrorActionPreference='Stop'; [Console]::OutputEncoding=New-Object Text.UTF8Encoding; @{languages=@(Get-WinUserLanguageList | ForEach-Object {$_.LanguageTag}); culture=(Get-Culture).Name; uiLanguage=[string](Get-WinUILanguageOverride); timezone=(Get-TimeZone).Id} | ConvertTo-Json -Compress"#;
const APPLY: &str = r#"
$ErrorActionPreference='Stop'
$env:PSModulePath=(Join-Path $PSHOME 'Modules')+';'+$env:PSModulePath
$before=Get-WinUserLanguageList
if(!(Test-Path -LiteralPath $env:COC_LANGUAGE_BACKUP)) {
    $before | Export-Clixml -LiteralPath $env:COC_LANGUAGE_BACKUP -Encoding UTF8
}
$target=New-WinUserLanguageList $env:COC_LANGUAGE
foreach($language in $before) {
    if($language.LanguageTag -ieq $env:COC_LANGUAGE) {$target.RemoveAt(0);$target.Insert(0,$language)} else {$target.Add($language)}
}
Set-WinUserLanguageList $target -Force
Set-WinUILanguageOverride -Language $env:COC_LANGUAGE
Set-Culture -CultureInfo $env:COC_LANGUAGE
if((Get-WinUserLanguageList)[0].LanguageTag -ine $env:COC_LANGUAGE){throw '首选语言未生效'}
"#;
const RESTORE: &str = r#"
$ErrorActionPreference='Stop'
$env:PSModulePath=(Join-Path $PSHOME 'Modules')+';'+$env:PSModulePath
$saved=Import-Clixml -LiteralPath $env:COC_LANGUAGE_BACKUP
$target=New-WinUserLanguageList $saved[0].LanguageTag
$target.Clear()
foreach($entry in $saved){$item=(New-WinUserLanguageList $entry.LanguageTag)[0];$item.InputMethodTips.Clear();foreach($tip in $entry.InputMethodTips){$item.InputMethodTips.Add([string]$tip)};$item.Spellchecking=$entry.Spellchecking;$item.Handwriting=$entry.Handwriting;$target.Add($item)}
Set-WinUserLanguageList $target -Force
if($env:COC_UI){Set-WinUILanguageOverride -Language $env:COC_UI}else{Set-WinUILanguageOverride}
Set-Culture -CultureInfo $env:COC_CULTURE
"#;
pub fn presets() -> Value {
    json!([
        {"id":"US","name":"美国（纽约）","language":"en-US","timezone":"America/New_York","windowsTimezone":"Eastern Standard Time"},
        {"id":"US-WEST","name":"美国（洛杉矶）","language":"en-US","timezone":"America/Los_Angeles","windowsTimezone":"Pacific Standard Time"},
        {"id":"GB","name":"英国","language":"en-GB","timezone":"Europe/London","windowsTimezone":"GMT Standard Time"},
        {"id":"CA","name":"加拿大（多伦多）","language":"en-CA","timezone":"America/Toronto","windowsTimezone":"Eastern Standard Time"},
        {"id":"SG","name":"新加坡","language":"en-SG","timezone":"Asia/Singapore","windowsTimezone":"Singapore Standard Time"},
        {"id":"AU","name":"澳大利亚（悉尼）","language":"en-AU","timezone":"Australia/Sydney","windowsTimezone":"AUS Eastern Standard Time"},
        {"id":"JP","name":"日本","language":"ja-JP","timezone":"Asia/Tokyo","windowsTimezone":"Tokyo Standard Time"},
        {"id":"KR","name":"韩国","language":"ko-KR","timezone":"Asia/Seoul","windowsTimezone":"Korea Standard Time"},
        {"id":"DE","name":"德国","language":"de-DE","timezone":"Europe/Berlin","windowsTimezone":"W. Europe Standard Time"},
        {"id":"FR","name":"法国","language":"fr-FR","timezone":"Europe/Paris","windowsTimezone":"Romance Standard Time"},
        {"id":"TW","name":"中国台湾","language":"zh-TW","timezone":"Asia/Taipei","windowsTimezone":"Taipei Standard Time"}
    ])
}
#[tauri::command]
pub async fn computer_environment_status(app: tauri::AppHandle) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<Value> {
        let current: Value = serde_json::from_str(&files::shell(READ, &[])?)?;
        let backup: Value = files::read(&root(&app)?.join("computer-backup.json"))?;
        Ok(json!({"current":current,"backup":backup,"presets":presets()}))
    })
    .await
    .map_err(error)?
    .map_err(error)
}
pub(crate) fn apply(app: &tauri::AppHandle, region: &str, restore: bool) -> Result<Value> {
    let dir = root(&app)?;
    files::safe(&dir)?;
    std::fs::create_dir_all(&dir)?;
    let metadata = dir.join("computer-backup.json");
    let language = dir.join("languages-backup.xml");
    files::safe(&language)?;
    if restore {
        let saved: Value = files::read(&metadata)?;
        let culture = saved["culture"]
            .as_str()
            .ok_or_else(|| anyhow!("没有可恢复的系统设置备份"))?;
        files::shell(
            RESTORE,
            &[
                ("COC_LANGUAGE_BACKUP", &language.to_string_lossy()),
                ("COC_CULTURE", culture),
                ("COC_UI", saved["uiLanguage"].as_str().unwrap_or("")),
            ],
        )?;
        super::timezone::set_timezone_by_name(
            saved["timezone"]
                .as_str()
                .ok_or_else(|| anyhow!("时区备份无效"))?,
        )?;
    } else {
        let presets = presets();
        let preset = presets
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == region)
            .ok_or_else(|| anyhow!("请选择支持的国家或地区"))?;
        if !metadata.exists() {
            let saved: Value = serde_json::from_str(&files::shell(READ, &[])?)?;
            files::write(&metadata, &saved)?;
        }
        // 系统时区修改也保留项目原有的恢复记录。
        super::timezone::backup_current(&app.path().app_data_dir()?)?;
        files::shell(
            APPLY,
            &[
                ("COC_LANGUAGE", preset["language"].as_str().unwrap()),
                ("COC_LANGUAGE_BACKUP", &language.to_string_lossy()),
            ],
        )?;
        super::timezone::set_timezone_by_name(preset["windowsTimezone"].as_str().unwrap())?;
    }
    let current: Value = serde_json::from_str(&files::shell(READ, &[])?)?;
    Ok(
        json!({"current":current,"message":"已写入并读回系统设置；Windows 显示语言可能需要语言包和重新登录。若操作中途失败，可使用已保存备份恢复。"}),
    )
}
