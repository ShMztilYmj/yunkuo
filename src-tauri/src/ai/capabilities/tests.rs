use super::*;
fn fixture() -> (crate::test_support::FixtureDir, PathBuf, Vec<Target>) {
    let f = crate::test_support::FixtureDir::new();
    let root = f.path().join("app/capability-library");
    let targets = targets(&f.path().join("user"), false, &[]).unwrap();
    (f, root, targets)
}
fn add(root: &Path, library: &mut Library, targets: &[Target], name: &str, spec: Value) -> String {
    action(
        root,
        library,
        targets,
        Action::SaveMcp {
            id: None,
            name: name.into(),
            spec,
        },
    )
    .unwrap();
    library.entries.last().unwrap().id.clone()
}
fn toggle(
    root: &Path,
    library: &mut Library,
    targets: &[Target],
    id: &str,
    target: &str,
    enabled: bool,
) -> Value {
    action(
        root,
        library,
        targets,
        Action::Toggle {
            id: id.into(),
            target: target.into(),
            enabled,
        },
    )
    .unwrap()
}
#[test]
fn mcp_roundtrip_preserves_other_fields_and_removes_only_owned_entry() {
    let (_f, root, targets) = fixture();
    let mut library = Library::default();
    let t = &targets[0];
    let original = json!({"oauthAccount":{"email":"fixture.invalid"},"projects":{"x":{"other":true}},"mcpServers":{"manual":{"command":"keep"}}});
    crate::platform::secure_storage::write_auth(&t.config, &serde_json::to_vec(&original).unwrap())
        .unwrap();
    let id = add(
        &root,
        &mut library,
        &targets,
        "browser",
        json!({"command":"node","args":["browser.js"],"env":{"FIXTURE_TOKEN":"private-fixture"}}),
    );
    let r = toggle(&root, &mut library, &targets, &id, "claude", true);
    assert_eq!(r["results"][0]["ok"], true);
    let doc: Value = serde_json::from_slice(&fs::read(&t.config).unwrap()).unwrap();
    assert_eq!(doc["projects"], original["projects"]);
    assert_eq!(doc["oauthAccount"], original["oauthAccount"]);
    assert_eq!(
        doc["mcpServers"]["manual"],
        original["mcpServers"]["manual"]
    );
    toggle(&root, &mut library, &targets, &id, "claude", false);
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(&t.config).unwrap()).unwrap(),
        original
    );
    assert!(
        !String::from_utf8_lossy(&fs::read(root.join("library.coc")).unwrap())
            .contains("private-fixture")
    );
    assert_eq!(load(&root).unwrap().entries.len(), 1);
}
#[test]
fn display_info_updates_without_touching_spec_or_sync_key() {
    let (_f, root, targets) = fixture();
    let mut library = Library::default();
    let id = add(
        &root,
        &mut library,
        &targets,
        "browser",
        json!({"command":"node","args":["browser.js"]}),
    );
    let original_name = library.entries[0].name.clone();
    let original_spec = library.entries[0].spec.clone();
    action(
        &root,
        &mut library,
        &targets,
        Action::UpdateInfo {
            id: id.clone(),
            title: "浏览器工具".into(),
            summary: "浏览器自动化与页面读取".into(),
            detail: "支持打开页面、截图和读取结构化内容。".into(),
            tags: vec!["浏览器".into(), " 浏览器 ".into(), "自动化".into()],
            repository: "https://example.invalid/browser".into(),
        },
    )
    .unwrap();
    let entry = &library.entries[0];
    assert_eq!(entry.name, original_name);
    assert_eq!(entry.spec, original_spec);
    assert_eq!(entry.title, "浏览器工具");
    assert_eq!(entry.summary, "浏览器自动化与页面读取");
    assert_eq!(entry.tags, vec!["浏览器".to_string(), "自动化".to_string()]);
    let row = snapshot(&root, &library, &targets, &[]);
    assert_eq!(row["entries"][0]["title"], "浏览器工具");
    assert_eq!(
        row["entries"][0]["repository"],
        "https://example.invalid/browser"
    );
}

#[test]
fn each_tool_uses_correct_transport_and_toml_survives_model_changes() {
    let (_f, root, targets) = fixture();
    let mut library = Library::default();
    let t = &targets[1];
    crate::platform::secure_storage::write_auth(&t.config,b"# preserve comment\nmodel = \"fixture-model\"\n[features]\nkeep = true\n[mcp_servers.manual]\ncommand = \"keep\"\n").unwrap();
    let id = add(
        &root,
        &mut library,
        &targets,
        "remote",
        json!({"type":"http","url":"https://example.invalid/mcp","headers":{"Authorization":"Bearer fixture"}}),
    );
    for target in ["claude", "codex", "cursor"] {
        let r = toggle(&root, &mut library, &targets, &id, target, true);
        assert_eq!(r["results"][0]["ok"], true, "{r}");
    }
    assert_eq!(
        adapters::read_servers(&targets[0]).unwrap()["remote"]["url"],
        "https://example.invalid/mcp"
    );
    assert_eq!(
        adapters::read_servers(t).unwrap()["remote"]["http_headers"]["Authorization"],
        "Bearer fixture"
    );
    let text = fs::read_to_string(&t.config).unwrap();
    assert!(text.contains("# preserve comment"));
    assert!(text.contains("fixture-model"));
    assert!(text.contains("[features]"));
    assert!(text.contains("[mcp_servers.manual]"));
    toggle(&root, &mut library, &targets, &id, "codex", false);
    assert!(adapters::read_servers(t).unwrap().contains_key("manual"));
    assert!(
        snapshot(&root, &library, &targets, &[])
            .to_string()
            .find("Bearer fixture")
            .is_none()
    );
}
#[test]
fn conflicts_are_per_target_and_explicit_import_can_adopt_existing_config() {
    let (_f, root, targets) = fixture();
    let mut library = Library::default();
    crate::platform::secure_storage::write_auth(
        &targets[0].config,
        br#"{"mcpServers":{"browser":{"command":"external"}}}"#,
    )
    .unwrap();
    let id = add(
        &root,
        &mut library,
        &targets,
        "browser",
        json!({"command":"managed"}),
    );
    let r = toggle(&root, &mut library, &targets, &id, "claude", true);
    assert_eq!(r["results"][0]["ok"], false);
    assert_eq!(
        adapters::read_servers(&targets[0]).unwrap()["browser"]["command"],
        "external"
    );
    assert_eq!(
        toggle(&root, &mut library, &targets, &id, "codex", true)["results"][0]["ok"],
        true
    );
    action(&root, &mut library, &targets, Action::Delete { id }).unwrap();
    action(
        &root,
        &mut library,
        &targets,
        Action::ImportMcp {
            target: "claude".into(),
            name: "browser".into(),
            as_name: "browser".into(),
        },
    )
    .unwrap();
    let id = library.entries[0].id.clone();
    assert_eq!(
        toggle(&root, &mut library, &targets, &id, "claude", true)["results"][0]["ok"],
        true
    );
    crate::platform::secure_storage::write_auth(
        &targets[0].config,
        br#"{"mcpServers":{"browser":{"command":"edited-by-user"}}}"#,
    )
    .unwrap();
    let r = toggle(&root, &mut library, &targets, &id, "claude", false);
    assert_eq!(r["results"][0]["ok"], false);
    assert_eq!(
        adapters::read_servers(&targets[0]).unwrap()["browser"]["command"],
        "edited-by-user"
    );
}
#[test]
fn invalid_config_is_preserved_and_sse_is_not_silently_converted_for_codex() {
    let (_f, root, targets) = fixture();
    let mut library = Library::default();
    crate::platform::secure_storage::write_auth(&targets[2].config, b"{broken").unwrap();
    let id = add(
        &root,
        &mut library,
        &targets,
        "sse",
        json!({"type":"sse","url":"http://localhost:1234/sse"}),
    );
    assert_eq!(
        toggle(&root, &mut library, &targets, &id, "cursor", true)["results"][0]["ok"],
        false
    );
    assert!(
        action(
            &root,
            &mut library,
            &targets,
            Action::Toggle {
                id,
                target: "codex".into(),
                enabled: true
            }
        )
        .is_err()
    );
    assert_eq!(fs::read(&targets[2].config).unwrap(), b"{broken");
    for name in ["..", "../escape", "CON", "LPT1", "x:y"] {
        assert!(valid_name(name).is_err());
    }
    assert!(adapters::normalize(json!({"command":"node","unknown":true})).is_err());
}
#[test]
fn skill_copy_update_disable_and_conflict_do_not_touch_other_directories() {
    let (f, root, targets) = fixture();
    let source = f.path().join("source");
    fs::create_dir_all(source.join("scripts")).unwrap();
    fs::write(source.join("SKILL.md"), "fixture instructions").unwrap();
    fs::write(source.join("scripts/task.py"), "print('fixture')").unwrap();
    let mut library = Library::default();
    action(
        &root,
        &mut library,
        &targets,
        Action::ImportSkill {
            path: source.to_string_lossy().into(),
            name: "fixture-skill".into(),
            id: None,
        },
    )
    .unwrap();
    let id = library.entries[0].id.clone();
    let t = &targets[1];
    let installed = t.skills.join("fixture-skill");
    assert_eq!(
        toggle(&root, &mut library, &targets, &id, "codex", true)["results"][0]["ok"],
        true
    );
    assert_eq!(
        fs::read(installed.join("scripts/task.py")).unwrap(),
        fs::read(source.join("scripts/task.py")).unwrap()
    );
    fs::write(source.join("SKILL.md"), "new fixture").unwrap();
    action(
        &root,
        &mut library,
        &targets,
        Action::ImportSkill {
            path: source.to_string_lossy().into(),
            name: "fixture-skill".into(),
            id: Some(id.clone()),
        },
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(installed.join("SKILL.md")).unwrap(),
        "new fixture"
    );
    fs::write(installed.join("SKILL.md"), "user editing").unwrap();
    assert_eq!(
        toggle(&root, &mut library, &targets, &id, "codex", false)["results"][0]["ok"],
        false
    );
    assert_eq!(
        fs::read_to_string(installed.join("SKILL.md")).unwrap(),
        "user editing"
    );
    assert!(source.join("SKILL.md").exists());
    assert!(!root.join("skill-pending.coc").exists());
}
#[test]
fn skill_interrupted_publish_restores_original_and_cleans_partial_stage() {
    skills::test_recovery();
}
#[test]
fn skill_disable_removes_only_the_unchanged_managed_folder() {
    let (f, root, targets) = fixture();
    let source = f.path().join("source");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("SKILL.md"), "fixture").unwrap();
    let mut library = Library::default();
    action(
        &root,
        &mut library,
        &targets,
        Action::ImportSkill {
            path: source.to_string_lossy().into(),
            name: "managed".into(),
            id: None,
        },
    )
    .unwrap();
    let id = library.entries[0].id.clone();
    toggle(&root, &mut library, &targets, &id, "claude", true);
    let manual = targets[0].skills.join("manual");
    fs::create_dir_all(&manual).unwrap();
    fs::write(manual.join("SKILL.md"), "keep").unwrap();
    assert_eq!(
        toggle(&root, &mut library, &targets, &id, "claude", false)["results"][0]["ok"],
        true
    );
    assert!(!targets[0].skills.join("managed").exists());
    assert!(manual.join("SKILL.md").exists());
    assert!(source.join("SKILL.md").exists());
}
#[test]
fn update_preview_reports_source_changes_before_apply() {
    let (f, root, targets) = fixture();
    let source = f.path().join("source");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("SKILL.md"), "first").unwrap();
    let mut library = Library::default();
    action(
        &root,
        &mut library,
        &targets,
        Action::ImportSkill {
            path: source.to_string_lossy().into(),
            name: "tracked".into(),
            id: None,
        },
    )
    .unwrap();
    let id = library.entries[0].id.clone();
    assert_eq!(library.entries[0].source.as_ref().unwrap().kind, "folder");
    toggle(&root, &mut library, &targets, &id, "claude", true);
    let (recorded, up) = upstream(&library.entries[0], &targets).unwrap();
    let (files, fields, changed) = upstream_changes(&library.entries[0], &recorded, &up);
    assert!(!changed && files.is_empty() && fields.is_empty());
    fs::write(source.join("SKILL.md"), "second").unwrap();
    fs::write(source.join("extra.txt"), "extra").unwrap();
    let (latest, up) = upstream(&library.entries[0], &targets).unwrap();
    let (files, _, changed) = upstream_changes(&library.entries[0], &latest, &up);
    assert!(changed);
    assert!(
        files
            .iter()
            .any(|f| f["path"] == "SKILL.md" && f["status"] == "modified")
    );
    assert!(
        files
            .iter()
            .any(|f| f["path"] == "extra.txt" && f["status"] == "added")
    );
    assert!(
        action(
            &root,
            &mut library,
            &targets,
            Action::ApplyUpdate {
                id: id.clone(),
                expected: "stale".into(),
            },
        )
        .is_err()
    );
    let expected = up.hash.clone();
    action(
        &root,
        &mut library,
        &targets,
        Action::ApplyUpdate {
            id: id.clone(),
            expected,
        },
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(targets[0].skills.join("tracked/SKILL.md")).unwrap(),
        "second"
    );
    assert_eq!(library.entries[0].source.as_ref().unwrap().hash, up.hash);
    assert_eq!(state(&library.entries[0], &targets[0]), "synced");
}
#[test]
fn conflicts_offer_adopt_overwrite_and_duplicate() {
    let (_f, root, targets) = fixture();
    let mut library = Library::default();
    crate::platform::secure_storage::write_auth(
        &targets[0].config,
        br#"{"mcpServers":{"browser":{"command":"external"}}}"#,
    )
    .unwrap();
    let id = add(
        &root,
        &mut library,
        &targets,
        "browser",
        json!({"command":"managed"}),
    );
    let r = toggle(&root, &mut library, &targets, &id, "claude", true);
    assert_eq!(r["results"][0]["ok"], false);
    assert_eq!(r["results"][0]["conflict"], true);
    action(
        &root,
        &mut library,
        &targets,
        Action::ResolveConflict {
            id: id.clone(),
            target: "claude".into(),
            choice: "adopt".into(),
            name: None,
        },
    )
    .unwrap();
    assert_eq!(library.entries[0].spec["command"], "external");
    assert_eq!(state(&library.entries[0], &targets[0]), "synced");
    crate::platform::secure_storage::write_auth(
        &targets[0].config,
        br#"{"mcpServers":{"browser":{"command":"drifted"}}}"#,
    )
    .unwrap();
    assert_eq!(
        toggle(&root, &mut library, &targets, &id, "claude", false)["results"][0]["conflict"],
        true
    );
    action(
        &root,
        &mut library,
        &targets,
        Action::ResolveConflict {
            id: id.clone(),
            target: "claude".into(),
            choice: "overwrite".into(),
            name: None,
        },
    )
    .unwrap();
    assert_eq!(
        adapters::read_servers(&targets[0]).unwrap()["browser"]["command"],
        "external"
    );
    crate::platform::secure_storage::write_auth(
        &targets[0].config,
        br#"{"mcpServers":{"browser":{"command":"external-2"}}}"#,
    )
    .unwrap();
    action(
        &root,
        &mut library,
        &targets,
        Action::ResolveConflict {
            id: id.clone(),
            target: "claude".into(),
            choice: "duplicate".into(),
            name: Some("browser-local".into()),
        },
    )
    .unwrap();
    assert_eq!(library.entries.len(), 2);
    assert_eq!(library.entries[1].name, "browser-local");
    assert_eq!(library.entries[1].spec["command"], "external-2");
    assert_eq!(
        adapters::read_servers(&targets[0]).unwrap()["browser"]["command"],
        "external-2"
    );
}
#[test]
fn first_sync_backup_is_restorable_and_reported() {
    let (_f, root, targets) = fixture();
    let mut library = Library::default();
    let original = br#"{"mcpServers":{"manual":{"command":"keep"}}}"#;
    crate::platform::secure_storage::write_auth(&targets[0].config, original).unwrap();
    let id = add(
        &root,
        &mut library,
        &targets,
        "browser",
        json!({"command":"managed"}),
    );
    assert_eq!(
        toggle(&root, &mut library, &targets, &id, "claude", true)["results"][0]["ok"],
        true
    );
    assert_eq!(
        snapshot(&root, &library, &targets, &[])["targets"][0]["hasBackup"],
        true
    );
    action(
        &root,
        &mut library,
        &targets,
        Action::RestoreBackup {
            target: "claude".into(),
        },
    )
    .unwrap();
    assert_eq!(fs::read(&targets[0].config).unwrap(), original.to_vec());
    assert_eq!(
        snapshot(&root, &library, &targets, &[])["targets"][0]["hasBackup"],
        false
    );
    assert!(library.entries[0].applied.is_empty());
    assert_eq!(library.entries[0].enabled.get("claude"), Some(&false));
}

#[test]
fn skill_detail_exposes_manifest_and_files() {
    let (f, root, targets) = fixture();
    let source = f.path().join("source");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("SKILL.md"), "---\nname: demo\n---\nbody").unwrap();
    fs::write(source.join("notes.md"), "note").unwrap();
    let mut library = Library::default();
    action(
        &root,
        &mut library,
        &targets,
        Action::ImportSkill {
            path: source.to_string_lossy().into(),
            name: "demo".into(),
            id: None,
        },
    )
    .unwrap();
    let detail = skill_detail(&library, &library.entries[0].id).unwrap();
    assert_eq!(detail["preview"], "---\nname: demo\n---\nbody");
    assert_eq!(detail["fileCount"], 2);
    assert_eq!(detail["files"][0]["path"], "SKILL.md");
    assert_eq!(detail["files"][1]["path"], "notes.md");
    assert!(library.entries.len() == 1);
}

#[test]
fn sync_targets_follow_added_agents() {
    let home = std::path::PathBuf::from("C:/Users/fixture");
    let all = targets(&home, false, &[]).unwrap();
    assert_eq!(all.len(), 9, "空名单表示不过滤：九款工具都接入了同步");

    let defaults = targets(&home, false, &["claude".into(), "chatgpt".into()]).unwrap();
    assert_eq!(
        defaults.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        ["claude", "codex"],
        "默认只添加了 Claude 与 Codex，同步位置就只有这两条"
    );

    let cursor_only = targets(&home, false, &["cursor".into()]).unwrap();
    assert_eq!(cursor_only.len(), 1, "Cursor 已接入 MCP 同步");
    assert_eq!(cursor_only[0].tool, "cursor");
    assert!(
        !cursor_only[0].skills_supported,
        "Cursor 只有 MCP，不该显示 Skills 目录"
    );
    assert!(
        cursor_only[0]
            .config
            .to_string_lossy()
            .replace('\\', "/")
            .ends_with(".cursor/mcp.json")
    );

    let kimi = targets(&home, false, &["kimi".into()]).unwrap();
    assert_eq!(kimi.len(), 1, "Kimi Code 已接入 MCP + Skills");
    assert!(kimi[0].skills_supported);
    assert!(
        kimi[0]
            .config
            .to_string_lossy()
            .replace('\\', "/")
            .ends_with(".kimi/mcp.json")
    );

    let opencode = targets(&home, false, &["opencode".into()]).unwrap();
    assert_eq!(opencode.len(), 1, "OpenCode 已接入 MCP 同步");
    assert!(!opencode[0].skills_supported);
    assert!(
        opencode[0]
            .config
            .to_string_lossy()
            .replace('\\', "/")
            .ends_with(".config/opencode/opencode.json")
    );

    let antigravity = targets(&home, false, &["antigravity".into()]).unwrap();
    assert_eq!(antigravity.len(), 1, "Antigravity 已接入 MCP 同步");
    assert!(
        antigravity[0]
            .config
            .to_string_lossy()
            .replace('\\', "/")
            .ends_with(".gemini/config/mcp_config.json")
    );

    let grok = targets(&home, false, &["grok".into()]).unwrap();
    assert_eq!(grok.len(), 1, "Grok 已接入 MCP 同步");
    assert!(
        grok[0]
            .config
            .to_string_lossy()
            .replace('\\', "/")
            .ends_with(".grok/settings.json")
    );

    let zcode = targets(&home, false, &["glm".into()]).unwrap();
    assert_eq!(zcode.len(), 1, "ZCode 已接入 MCP + Skills");
    assert!(zcode[0].skills_supported);
    assert!(
        zcode[0]
            .config
            .to_string_lossy()
            .replace('\\', "/")
            .ends_with(".zcode/cli/config.json")
    );

    let dsh = targets(&home, false, &["deepseek".into()]).unwrap();
    assert_eq!(dsh.len(), 1, "DeepSeek Harness 已接入 MCP + Skills");
    assert!(dsh[0].skills_supported);
    assert!(
        dsh[0]
            .config
            .to_string_lossy()
            .replace('\\', "/")
            .ends_with(".dsh/profiles/web/cordis.patch.yml")
    );
}

#[test]
fn zcode_nested_mcp_servers_keep_other_keys_and_restore_cleanly() {
    let (_f, root, targets) = fixture();
    let zcode = targets.iter().find(|t| t.id == "glm").unwrap();
    let original = json!({
        "mcp": { "servers": { "existing": { "type": "http", "url": "https://existing.example/mcp" } } },
        "plugins": { "enabledPlugins": { "demo@market": true } }
    });
    crate::platform::secure_storage::write_auth(
        &zcode.config,
        &serde_json::to_vec(&original).unwrap(),
    )
    .unwrap();
    let mut library = Library::default();
    let id = add(
        &root,
        &mut library,
        &targets,
        "z-asset",
        json!({"type":"http","url":"https://mcp.example/mcp"}),
    );
    let r = toggle(&root, &mut library, &targets, &id, "glm", true);
    assert_eq!(r["results"][0]["ok"], true);
    let doc: Value = serde_json::from_slice(&fs::read(&zcode.config).unwrap()).unwrap();
    assert_eq!(
        doc["mcp"]["servers"]["z-asset"]["url"],
        "https://mcp.example/mcp"
    );
    assert_eq!(
        doc["mcp"]["servers"]["existing"]["url"],
        "https://existing.example/mcp"
    );
    assert_eq!(doc["plugins"], original["plugins"]);
    toggle(&root, &mut library, &targets, &id, "glm", false);
    let after: Value = serde_json::from_slice(&fs::read(&zcode.config).unwrap()).unwrap();
    assert_eq!(after, original);
}
#[test]
fn dsh_yaml_patch_adds_removes_and_keeps_comments_and_other_entries() {
    let (_f, root, targets) = fixture();
    let dsh = targets.iter().find(|t| t.id == "deepseek").unwrap();
    let original = "# patch layer for this profile\n- id: agent-default-model\n  name: \"@deepseek-ai/dsh-agent-default-model\"\n  config:\n    provider: demo\n- id: mcp-old\n  name: \"@deepseek-ai/dsh-mcp-client\"\n  config:\n    serverName: old\n    transport: stdio\n    command: node\n";
    crate::platform::secure_storage::write_auth(&dsh.config, original.as_bytes()).unwrap();
    let mut library = Library::default();
    let id = add(
        &root,
        &mut library,
        &targets,
        "dsh-asset",
        json!({"type":"stdio","command":"npx","args":["-y","demo-mcp"]}),
    );
    let r = toggle(&root, &mut library, &targets, &id, "deepseek", true);
    assert_eq!(r["results"][0]["ok"], true, "{}", r);
    let text = fs::read_to_string(&dsh.config).unwrap();
    assert!(text.starts_with("# patch layer for this profile"));
    assert!(text.contains("dsh-agent-default-model"));
    assert!(text.contains("serverName: dsh-asset"));
    assert!(text.contains("transport: stdio"));
    let parsed: serde_yaml::Value = serde_yaml::from_str(&text).unwrap();
    let entries = parsed.as_sequence().unwrap();
    let ours = entries
        .iter()
        .find(|entry| entry["config"]["serverName"].as_str() == Some("dsh-asset"))
        .unwrap();
    assert_eq!(ours["name"].as_str(), Some("@deepseek-ai/dsh-mcp-client"));
    assert_eq!(ours["id"].as_str(), Some("mcp-dsh-asset"));
    toggle(&root, &mut library, &targets, &id, "deepseek", false);
    let after = fs::read_to_string(&dsh.config).unwrap();
    assert!(after.starts_with("# patch layer for this profile"));
    assert!(!after.contains("dsh-asset"), "文件内容：\n{after}");
    assert!(after.contains("serverName: old"));
}

#[test]
fn dsh_rejects_sse_and_bad_names() {
    let (_f, root, targets) = fixture();
    let _dsh = targets.iter().find(|t| t.id == "deepseek").unwrap();
    let mut library = Library::default();
    let id = add(
        &root,
        &mut library,
        &targets,
        "dsh-sse",
        json!({"type":"sse","url":"https://example/mcp"}),
    );
    let r = toggle(&root, &mut library, &targets, &id, "deepseek", true);
    assert_eq!(r["results"][0]["ok"], false);
    assert!(
        r["results"][0]["error"]
            .as_str()
            .unwrap_or_default()
            .contains("Streamable HTTP")
    );
}
