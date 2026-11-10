use super::*;
use serde_json::Map;
use toml_edit::{DocumentMut, Item};

pub(super) fn normalize(mut spec: Value) -> Result<Value> {
    let object = spec.as_object_mut().context("MCP 配置必须是 JSON 对象")?;
    let transport = object
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or(if object.contains_key("command") {
            "stdio"
        } else {
            "http"
        })
        .to_owned();
    let transport = if transport == "streamable-http" {
        "http"
    } else {
        &transport
    };
    let allowed: &[&str] = match transport {
        "stdio" => &["type", "command", "args", "env", "cwd"],
        "http" | "sse" => &["type", "url", "headers"],
        _ => bail!("支持 stdio、HTTP 和 SSE 传输"),
    };
    if object.keys().any(|k| !allowed.contains(&k.as_str())) {
        bail!(
            "配置含暂不支持的字段，请保留为工具原配置；本库支持 command/args/env/cwd 或 url/headers"
        );
    }
    object.insert("type".into(), json!(transport));
    if transport == "stdio" {
        let command = object
            .get("command")
            .and_then(Value::as_str)
            .context("stdio 需要 command")?;
        if command.trim().is_empty()
            || command.len() > 4096
            || command.chars().any(char::is_control)
        {
            bail!("command 无效");
        }
        if let Some(args) = object.get("args") {
            if !args.as_array().is_some_and(|a| {
                a.len() <= 128
                    && a.iter()
                        .all(|s| s.as_str().is_some_and(|s| s.len() <= 8192))
            }) {
                bail!("args 必须是字符串数组");
            }
        }
        if let Some(cwd) = object.get("cwd") {
            if !cwd.as_str().is_some_and(|s| Path::new(s).is_absolute()) {
                bail!("cwd 需要绝对路径");
            }
        }
    } else {
        let url = object
            .get("url")
            .and_then(Value::as_str)
            .context("远程 MCP 需要 url")?;
        let url = reqwest::Url::parse(url).context("MCP 地址无效")?;
        if !["http", "https"].contains(&url.scheme())
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            bail!("MCP 需要不含内嵌账号密码的 HTTP(S) 地址");
        }
    }
    for field in ["env", "headers"] {
        if let Some(values) = object.get(field) {
            let map = values
                .as_object()
                .context("env 和 headers 需要字符串映射")?;
            if map.len() > 64
                || map.iter().any(|(k, v)| {
                    k.is_empty()
                        || k.len() > 128
                        || k.chars().any(char::is_control)
                        || !v.as_str().is_some_and(|s| {
                            s.len() <= 16384 && (field != "headers" || !s.contains(['\r', '\n']))
                        })
                })
            {
                bail!("env 或 headers 字段无效");
            }
        }
    }
    if serde_json::to_vec(&spec)?.len() > 65536 {
        bail!("单个 MCP 配置超过 64 KiB");
    }
    Ok(spec)
}
pub(super) fn project(spec: &Value, tool: &str) -> Result<Value> {
    let mut map = spec.as_object().context("MCP 配置无效")?.clone();
    let transport = map
        .remove("type")
        .and_then(|v| v.as_str().map(str::to_owned))
        .context("MCP 缺少传输类型")?;
    match tool {
        "claude" => {
            map.insert("type".into(), json!(transport));
        }
        // DSH 的转换放在 merge 里（需要条目名），这里保留 type 供它判断传输。
        "deepseek" => {
            map.insert("type".into(), json!(transport));
        }
        // Cursor 的 mcpServers 只认 command/args/env 或 url/headers，不写 type。
        "cursor" => {}
        // Kimi Code 用 transport 字段（stdio / http）；stdio 条目可以不带这个字段。
        "kimi" => {
            if transport == "sse" {
                bail!("Kimi Code 不支持 SSE，请使用 HTTP MCP 地址");
            }
            if transport == "http" {
                map.insert("transport".into(), json!("http"));
            }
        }
        // OpenCode 用 local / remote 两种条目：本地是命令数组 + environment。
        "opencode" => {
            if transport == "stdio" {
                let command = map.remove("command").context("缺少 command")?;
                let mut argv = vec![command];
                if let Some(list) = map.remove("args").and_then(|v| v.as_array().cloned()) {
                    argv.extend(list);
                }
                map.remove("cwd");
                if let Some(env) = map.remove("env") {
                    map.insert("environment".into(), env);
                }
                map.insert("command".into(), Value::Array(argv));
            }
            map.insert(
                "type".into(),
                json!(if transport == "stdio" {
                    "local"
                } else {
                    "remote"
                }),
            );
        }
        // Antigravity 的远程条目用 serverUrl，本地仍是 command/args/env。
        "antigravity" => {
            if transport != "stdio" {
                if let Some(url) = map.remove("url") {
                    map.insert("serverUrl".into(), url);
                }
            }
        }
        // Grok 的 settings.json 里 mcpServers 用标准字段，不写 type。
        "grok" => {}
        // ZCode 的 mcp.servers 用标准 {type, command/args} 或 {type, url}。
        "glm" => {
            map.insert("type".into(), json!(transport));
        }
        "codex" => {
            if transport == "sse" {
                bail!("Codex CLI 不支持此 SSE 配置，请使用 HTTP MCP 地址");
            }
            if let Some(headers) = map.remove("headers") {
                map.insert("http_headers".into(), headers);
            }
        }
        _ => bail!("未知工具"),
    }
    Ok(Value::Object(map))
}
pub(super) fn unproject(mut spec: Value, tool: &str) -> Result<Value> {
    let map = spec.as_object_mut().context("MCP 配置无效")?;
    if tool == "codex" {
        if let Some(headers) = map.remove("http_headers") {
            map.insert("headers".into(), headers);
        }
    }
    if tool == "kimi" {
        map.remove("transport");
    }
    if tool == "opencode" {
        let remote = map.get("type").and_then(Value::as_str) == Some("remote");
        if remote {
            map.insert("type".into(), json!("http"));
        } else {
            if let Some(Value::Array(argv)) = map.remove("command") {
                let mut iter = argv.into_iter();
                if let Some(first) = iter.next() {
                    map.insert("command".into(), first);
                }
                let rest: Vec<Value> = iter.collect();
                if !rest.is_empty() {
                    map.insert("args".into(), Value::Array(rest));
                }
            }
            if let Some(env) = map.remove("environment") {
                map.insert("env".into(), env);
            }
            map.insert("type".into(), json!("stdio"));
        }
        map.remove("enabled");
        map.remove("timeout");
    }
    if tool == "antigravity" {
        if let Some(url) = map.remove("serverUrl") {
            map.insert("url".into(), url);
            map.insert("type".into(), json!("http"));
        }
    }
    normalize(spec)
}
fn json_document(bytes: Option<&[u8]>) -> Result<Value> {
    let value = match bytes {
        Some(bytes) => {
            serde_json::from_slice(bytes.strip_prefix(&[239, 187, 191]).unwrap_or(bytes))
                .map_err(|_| anyhow::anyhow!("原 JSON 配置无法解析，原文件保留"))?
        }
        None => json!({}),
    };
    if !value.is_object() {
        bail!("原 JSON 配置不是对象，原文件保留");
    }
    Ok(value)
}
fn toml_document(bytes: Option<&[u8]>) -> Result<DocumentMut> {
    Ok(std::str::from_utf8(bytes.unwrap_or_default())
        .context("原配置不是 UTF-8")?
        .trim_start_matches('\u{feff}')
        .parse()
        .map_err(|_| anyhow::anyhow!("原 TOML 配置无法解析，原文件保留"))?)
}
fn toml_json(item: &Item) -> Result<Value> {
    if let Some(table) = item.as_table_like() {
        return Ok(Value::Object(
            table
                .iter()
                .map(|(k, v)| Ok((k.to_owned(), toml_json(v)?)))
                .collect::<Result<Map<_, _>>>()?,
        ));
    }
    let v = item.as_value().context("MCP TOML 包含不支持的结构")?;
    if let Some(s) = v.as_str() {
        Ok(json!(s))
    } else if let Some(b) = v.as_bool() {
        Ok(json!(b))
    } else if let Some(n) = v.as_integer() {
        Ok(json!(n))
    } else if let Some(n) = v.as_float() {
        Ok(json!(n))
    } else if let Some(a) = v.as_array() {
        Ok(Value::Array(
            a.iter()
                .map(|v| toml_json(&Item::Value(v.clone())))
                .collect::<Result<_>>()?,
        ))
    } else {
        bail!("MCP TOML 包含不支持的字段类型");
    }
}
fn json_toml(v: &Value) -> Result<toml_edit::Value> {
    Ok(match v {
        Value::String(s) => toml_edit::Value::from(s.as_str()),
        Value::Bool(b) => toml_edit::Value::from(*b),
        Value::Number(n) => {
            if let Some(n) = n.as_i64() {
                toml_edit::Value::from(n)
            } else {
                toml_edit::Value::from(n.as_f64().context("数字无效")?)
            }
        }
        Value::Array(values) => {
            let mut a = toml_edit::Array::new();
            for v in values {
                a.push(json_toml(v)?);
            }
            toml_edit::Value::Array(a)
        }
        Value::Object(values) => {
            let mut t = toml_edit::InlineTable::new();
            for (k, v) in values {
                t.insert(k, json_toml(v)?);
            }
            toml_edit::Value::InlineTable(t)
        }
        _ => bail!("MCP 字段不支持 null"),
    })
}
/// 各工具在配置文件里存放 MCP 服务的键路径（支持嵌套，例如 ZCode 的 mcp.servers）。
fn servers_path(tool: &str) -> &'static [&'static str] {
    match tool {
        "opencode" => &["mcp"],
        // ZCode 的 CLI 配置把服务放在 mcp.servers 下
        "glm" => &["mcp", "servers"],
        _ => &["mcpServers"],
    }
}
fn read_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(value, |node, key| node.get(*key))
}
fn ensure_path<'a>(value: &'a mut Value, path: &[&str]) -> Result<&'a mut Map<String, Value>> {
    if path.is_empty() {
        return value.as_object_mut().context("配置不是对象");
    }
    let object = value.as_object_mut().context("配置不是对象")?;
    let child = object
        .entry(path[0].to_string())
        .or_insert_with(|| json!({}));
    ensure_path(child, &path[1..])
}
/// OpenCode 的配置允许单行注释与尾逗号（JSONC），读之前先清掉。
fn strip_jsonc(bytes: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(bytes);
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' {
            out.push(c);
            i += 1;
            while i < chars.len() {
                let s = chars[i];
                out.push(s);
                if s == '\\' && i + 1 < chars.len() {
                    i += 1;
                    out.push(chars[i]);
                } else if s == '"' {
                    break;
                }
                i += 1;
            }
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        } else {
            out.push(c);
        }
        i += 1;
    }
    let cleaned: Vec<char> = out.chars().collect();
    let mut result = String::with_capacity(out.len());
    for (index, c) in cleaned.iter().enumerate() {
        let trailing = cleaned[index + 1..].iter().find(|ch| !ch.is_whitespace());
        if *c == ',' && matches!(trailing, Some('}') | Some(']')) {
            continue;
        }
        result.push(*c);
    }
    result.into_bytes()
}
fn json_document_for(bytes: Option<&[u8]>, tool: &str) -> Result<Value> {
    if tool == "opencode" {
        json_document(bytes.map(strip_jsonc).as_deref())
    } else {
        json_document(bytes)
    }
}
/// DSH 的 MCP 不是独立 JSON，而是 profile 补丁里的插件条目：
/// `- { name: "@deepseek-ai/dsh-mcp-client", config: { serverName, transport, ... } }`。
const DSH_PLUGIN: &str = "@deepseek-ai/dsh-mcp-client";
fn dsh_text(bytes: Option<&[u8]>) -> String {
    match bytes {
        Some(bytes) => {
            String::from_utf8_lossy(bytes.strip_prefix(&[239, 187, 191]).unwrap_or(bytes))
                .into_owned()
        }
        None => String::new(),
    }
}
/// 顶层列表项的行区间：补丁文件是 YAML 数组，子项都带缩进，按列 0 的 `- ` 切块。
fn dsh_blocks(text: &str) -> Vec<(usize, usize)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut starts: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.starts_with("- ") || **line == "-")
        .map(|(index, _)| index)
        .collect();
    if starts.is_empty() {
        return vec![];
    }
    starts.push(lines.len());
    starts.windows(2).map(|pair| (pair[0], pair[1])).collect()
}
fn dsh_entry(text: &str) -> Result<Option<Value>> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    // 切出来的块本身是「一个列表项」，所以解析出来是单元素序列。
    let yaml: serde_yaml::Value = serde_yaml::from_str(text)
        .map_err(|_| anyhow::anyhow!("DeepSeek Harness 补丁无法解析，原文件保留"))?;
    let item = match yaml {
        serde_yaml::Value::Sequence(mut items) if items.len() == 1 => items.pop(),
        other if other.is_mapping() => Some(other),
        _ => None,
    };
    Ok(item.and_then(|value| serde_json::to_value(value).ok()))
}
/// 把规范化 MCP 配置转成 DSH 插件条目；已有的扩展字段（超时等）保留。
fn dsh_entry_value(
    name: &str,
    spec: &Value,
    existing: Option<&Value>,
) -> Result<serde_yaml::Value> {
    let transport = spec
        .get("type")
        .and_then(Value::as_str)
        .context("MCP 缺少传输类型")?;
    if transport == "sse" {
        bail!("DeepSeek Harness 只支持 stdio 与 Streamable HTTP，请改用 HTTP 地址");
    }
    if name.is_empty()
        || name.len() > 32
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        bail!("DeepSeek Harness 的 MCP 名称只能是 1–32 位字母、数字、- 或 _");
    }
    let value_of = |value: Value| -> Result<serde_yaml::Value> { Ok(serde_yaml::to_value(value)?) };
    let mut config = serde_yaml::Mapping::new();
    config.insert(value_of(json!("serverName"))?, value_of(json!(name))?);
    config.insert(
        value_of(json!("transport"))?,
        value_of(json!(if transport == "stdio" {
            "stdio"
        } else {
            "streamable-http"
        }))?,
    );
    if let Some(existing) = existing
        .and_then(|entry| entry.get("config"))
        .and_then(Value::as_object)
    {
        for (key, value) in existing {
            if ![
                "serverName",
                "transport",
                "command",
                "args",
                "env",
                "url",
                "headers",
            ]
            .contains(&key.as_str())
            {
                config.insert(value_of(json!(key))?, value_of(value.clone())?);
            }
        }
    }
    for key in ["command", "args", "env", "url", "headers"] {
        if let Some(value) = spec.get(key) {
            config.insert(value_of(json!(key))?, value_of(value.clone())?);
        }
    }
    let mut entry = serde_yaml::Mapping::new();
    entry.insert(
        value_of(json!("id"))?,
        value_of(json!(format!("mcp-{name}")))?,
    );
    entry.insert(value_of(json!("name"))?, value_of(json!(DSH_PLUGIN))?);
    entry.insert(
        value_of(json!("config"))?,
        serde_yaml::Value::Mapping(config),
    );
    Ok(serde_yaml::Value::Mapping(entry))
}
/// 渲染成补丁里的列表项文本（首行带 `- `，其余缩进两格）。
fn dsh_render(entry: &serde_yaml::Value) -> Result<String> {
    let yaml = serde_yaml::to_string(entry)?;
    let mut lines = yaml.lines();
    let first = lines.next().unwrap_or_default();
    let mut out = format!("- {first}\n");
    for line in lines {
        if line.is_empty() {
            out.push('\n');
        } else {
            out.push_str("  ");
            out.push_str(line);
            out.push('\n');
        }
    }
    Ok(out)
}
fn dsh_servers(text: &str) -> Result<Map<String, Value>> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Map::new();
    for (start, end) in dsh_blocks(text) {
        let block = lines[start..end].join("\n");
        let Some(entry) = dsh_entry(&block)? else {
            continue;
        };
        if entry.get("name").and_then(Value::as_str) != Some(DSH_PLUGIN) {
            continue;
        }
        let Some(config) = entry.get("config").and_then(Value::as_object) else {
            continue;
        };
        let Some(server) = config.get("serverName").and_then(Value::as_str) else {
            continue;
        };
        let transport = config
            .get("transport")
            .and_then(Value::as_str)
            .unwrap_or("stdio");
        let kind = match transport {
            "stdio" => "stdio",
            "streamable-http" | "http" => "http",
            _ => continue,
        };
        let mut spec = Map::new();
        spec.insert("type".into(), json!(kind));
        for key in ["command", "args", "env", "url", "headers"] {
            if let Some(value) = config.get(key) {
                spec.insert(key.into(), value.clone());
            }
        }
        out.insert(server.to_string(), Value::Object(spec));
    }
    Ok(out)
}
fn dsh_merge(text: &str, name: &str, value: Option<Value>) -> Result<String> {
    let lines: Vec<&str> = text.lines().collect();
    let mut target_range: Option<(usize, usize)> = None;
    let mut existing: Option<Value> = None;
    for (start, end) in dsh_blocks(text) {
        let block = lines[start..end].join("\n");
        let Some(entry) = dsh_entry(&block)? else {
            continue;
        };
        let same_name = entry.get("name").and_then(Value::as_str) == Some(DSH_PLUGIN);
        let same_server = entry
            .get("config")
            .and_then(|config| config.get("serverName"))
            .and_then(Value::as_str)
            == Some(name);
        if same_name && same_server {
            target_range = Some((start, end));
            existing = Some(entry);
            break;
        }
    }
    let mut out: Vec<String> = lines.iter().map(|line| line.to_string()).collect();
    match (value, target_range) {
        (Some(spec), Some((start, end))) => {
            let entry = dsh_entry_value(name, &spec, existing.as_ref())?;
            let block: Vec<String> = dsh_render(&entry)?.lines().map(str::to_string).collect();
            out.splice(start..end, block);
        }
        (Some(spec), None) => {
            let entry = dsh_entry_value(name, &spec, None)?;
            let rendered = dsh_render(&entry)?;
            if !out.is_empty() && out.last().is_some_and(|line| !line.is_empty()) {
                out.push(String::new());
            }
            out.extend(rendered.lines().map(str::to_string));
        }
        (None, Some((start, end))) => {
            out.drain(start..end);
        }
        (None, None) => {}
    }
    let mut merged = out.join("\n");
    if !merged.is_empty() && !merged.ends_with('\n') {
        merged.push('\n');
    }
    Ok(merged)
}
fn servers_from(bytes: Option<&[u8]>, tool: &str) -> Result<Map<String, Value>> {
    if tool == "deepseek" {
        return dsh_servers(&dsh_text(bytes));
    }
    let value = if tool == "codex" {
        let doc = toml_document(bytes)?;
        match doc.get("mcp_servers") {
            Some(item) => toml_json(item)?,
            None => json!({}),
        }
    } else {
        let doc = json_document_for(bytes, tool)?;
        read_path(&doc, servers_path(tool))
            .cloned()
            .unwrap_or_else(|| json!({}))
    };
    value
        .as_object()
        .cloned()
        .context("原 MCP 配置不是对象，原文件保留")
}
pub(super) fn read_servers(target: &Target) -> Result<Map<String, Value>> {
    servers_from(read_bytes(&target.config)?.as_deref(), &target.tool)
}
fn merge(bytes: Option<&[u8]>, tool: &str, name: &str, value: Option<Value>) -> Result<Vec<u8>> {
    if tool == "deepseek" {
        return Ok(dsh_merge(&dsh_text(bytes), name, value)?.into_bytes());
    }
    if tool == "codex" {
        let mut doc = toml_document(bytes)?;
        if doc.get("mcp_servers").is_none() {
            doc["mcp_servers"] = Item::Table(toml_edit::Table::new());
        }
        let servers = doc["mcp_servers"]
            .as_table_like_mut()
            .context("mcp_servers 不是表")?;
        if let Some(value) = value {
            let mut table = toml_edit::Table::new();
            for (k, v) in value.as_object().context("MCP 配置无效")? {
                table.insert(k, Item::Value(json_toml(v)?));
            }
            servers.insert(name, Item::Table(table));
        } else {
            servers.remove(name);
        }
        Ok(doc.to_string().into_bytes())
    } else {
        let mut doc = json_document_for(bytes, tool)?;
        let servers = ensure_path(&mut doc, servers_path(tool))?;
        if let Some(value) = value {
            servers.insert(name.into(), value);
        } else {
            servers.remove(name);
        }
        Ok(serde_json::to_vec_pretty(&doc)?)
    }
}
pub(super) fn sync(root: &Path, entry: &Entry, target: &Target, force: bool) -> Result<()> {
    let before = read_bytes(&target.config)?;
    // 每次在同一份最新文件上验证所有权并合并，避免覆盖其他 MCP、账号或模型字段。
    let current = servers_from(before.as_deref(), &target.tool)?
        .get(&entry.name)
        .map(|v| hash(&serde_json::to_vec(v).unwrap_or_default()));
    let desired = desired_hash(entry, target)?;
    if current != desired
        && current.is_some()
        && !force
        && !entry
            .applied
            .get(&target.id)
            .is_some_and(|r| Some(r.hash.as_str()) == current.as_deref())
    {
        return Err(anyhow::Error::new(Conflict(
            "同名 MCP 已被外部修改，未覆盖".into(),
        )));
    }
    let value = if entry.enabled.get(&target.id) == Some(&true) {
        Some(project(&entry.spec, &target.tool)?)
    } else {
        None
    };
    let after = merge(before.as_deref(), &target.tool, &entry.name, value)?;
    // 首次配置副本保留在 DPAPI 备份中；正常撤销只删除受管理条目。
    let backup = backup_path(root, target);
    if !backup.exists() {
        write_secret(
            &backup,
            &json!({"path":target.config,"original":before.as_ref().map(|b|STANDARD.encode(b))}),
        )?;
    }
    if read_bytes(&target.config)? != before {
        bail!("配置在同步期间变化，请重试");
    }
    crate::platform::secure_storage::write_auth(&target.config, &after)
}
