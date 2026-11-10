use crate::config::FileStore;
use anyhow::{Context, Result, anyhow};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_yaml::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

const MAX_PROFILE_BYTES: usize = 10 * 1024 * 1024;
/// 订阅地址长度上限：超长 URL 没有实际用途，只会污染日志与界面。
const MAX_SUBSCRIPTION_URL_LEN: usize = 2048;
/// 允许跟随的重定向跳数。
const MAX_SUBSCRIPTION_REDIRECTS: usize = 5;
/// YAML 别名引用上限：Clash 订阅不需要锚点/别名，超过即按别名炸弹拒绝。
const MAX_YAML_ALIASES: usize = 64;

pub(crate) static PROFILE_LOCK: once_cell::sync::Lazy<parking_lot::Mutex<()>> =
    once_cell::sync::Lazy::new(|| parking_lot::Mutex::new(()));

/// 订阅流量信息（来自 subscription-userinfo 响应头）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct TrafficInfo {
    pub upload: u64,
    pub download: u64,
    pub total: u64,
    /// Unix 秒
    pub expire: u64,
}

/// 手动额度：服务商不上报时（本地导入、未返回 userinfo 的订阅）由用户自己填。
///
/// `unlimited = true` 表示套餐不限量，只记到期；`expire = 0` 表示不到期。
/// 「不限量 + 不到期」是合法组合，所以不能再拿 `total == 0` 兼任「没填」——
/// 是否为空由 `is_blank()` 判定，存盘时清掉，卡片才会回到「未上报额度」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ManualQuota {
    /// 不限量套餐：不填总量，只看到期
    pub unlimited: bool,
    /// 总量（字节）；unlimited 时无意义，存盘前清零
    pub total: u64,
    /// Unix 秒，0 = 不到期
    pub expire: u64,
}

impl ManualQuota {
    /// 什么都没填：既没声明不限量，也没有量和到期。
    fn is_blank(&self) -> bool {
        !self.unlimited && self.total == 0 && self.expire == 0
    }

    /// 早期数据只写了 `total = 0` 来表示不限量，读回时补齐标志；顺带清掉多余的总量。
    fn normalized(mut self) -> Self {
        if self.unlimited || self.total == 0 {
            self.unlimited = true;
            self.total = 0;
        }
        self
    }
}

/// 订阅配置项
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ProfileItem {
    pub uid: String,
    /// remote = 订阅链接, local = 本地文件
    #[serde(rename = "type")]
    pub item_type: String,
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub url: String,
    /// 相对 profiles 目录的文件名
    pub file: String,
    pub updated_at: Option<String>,
    /// 自动更新间隔（小时），0 = 不自动更新
    pub update_interval: u64,
    /// 已用/总量/到期（订阅服务器返回时填充）
    pub traffic: Option<TrafficInfo>,
    /// 手动额度；服务商上报的 `traffic` 优先，这里只在缺数据时兜底
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota: Option<ManualQuota>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ProfileStore {
    pub current: Option<String>,
    pub items: Vec<ProfileItem>,
}

pub struct ProfileManager {
    pub store: FileStore,
    pub dir: PathBuf,
}

impl ProfileManager {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            store: FileStore::new(data_dir.join("profiles.yaml")),
            dir: data_dir.join("profiles"),
        }
    }

    pub fn load(&self) -> Result<ProfileStore> {
        self.store.load()
    }

    pub fn save(&self, ps: &ProfileStore) -> Result<()> {
        self.store.save(ps)
    }

    pub fn file_path(&self, file: &str) -> Result<PathBuf> {
        if file.is_empty() || file.contains(['/', '\\', ':']) || file == "." || file == ".." {
            return Err(anyhow!("订阅文件名不合法"));
        }
        Ok(self.dir.join(file))
    }

    /// 从 URL 导入订阅并下载内容
    pub async fn create_remote(&self, url: &str, name: Option<String>) -> Result<ProfileItem> {
        let uid = uuid::Uuid::new_v4().simple().to_string();
        let file = format!("{uid}.yaml");
        // 先校验再下载：订阅地址不得指向环回/内网/保留地址，避免被当作 SSRF 探测入口。
        let target = validated_subscription_url(url)?;
        let (content, traffic) = download_text(&target).await?;
        validate_yaml(&content)?;
        std::fs::create_dir_all(&self.dir)?;
        crate::config::atomic_write(&self.file_path(&file)?, content.as_bytes())?;
        let item = ProfileItem {
            uid,
            item_type: "remote".into(),
            name: name.unwrap_or_else(|| "订阅".into()),
            // 统一保存规范化后的地址，保证更新时的 TOCTOU 比对与本次下载目标一致。
            url: target.as_str().to_string(),
            file,
            updated_at: Some(now_string()),
            update_interval: 0,
            traffic,
            quota: None,
        };
        self.push_item(item.clone())?;
        Ok(item)
    }

    /// 从本地文件导入
    pub async fn create_local(&self, path: &str, name: Option<String>) -> Result<ProfileItem> {
        if path.trim().is_empty() {
            return Err(anyhow!("订阅文件路径不能为空"));
        }
        let uid = uuid::Uuid::new_v4().simple().to_string();
        let file = format!("{uid}.yaml");
        let source = PathBuf::from(path);
        let content =
            tauri::async_runtime::spawn_blocking(move || read_local_subscription(&source))
                .await
                .context("读取本地订阅任务中断")??;
        std::fs::create_dir_all(&self.dir)?;
        crate::config::atomic_write(&self.file_path(&file)?, content.as_bytes())?;
        let item = ProfileItem {
            uid,
            item_type: "local".into(),
            name: name.unwrap_or_else(|| {
                Path::new(path)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "本地配置".into())
            }),
            url: String::new(),
            file,
            updated_at: Some(now_string()),
            update_interval: 0,
            traffic: None,
            quota: None,
        };
        self.push_item(item.clone())?;
        Ok(item)
    }

    /// 节点串导入：把代理商给的接入串（`host:port:token`、
    /// `socks5://user:pass@host:port` 等）转成只有单个节点的 Clash 配置。
    ///
    /// 住宅/静态 IP 供应商通常只给这种串，不给订阅 YAML；这里生成同等结构，
    /// 后续内核、分流、检测都按普通订阅走，不需要另开一套通道。
    pub async fn create_from_node(
        &self,
        spec: &str,
        protocol: Option<&str>,
        name: Option<String>,
    ) -> Result<ProfileItem> {
        let node = parse_node_spec(spec, protocol)?;
        let content = node.to_yaml(name.as_deref())?;
        validate_yaml(&content)?;
        let uid = uuid::Uuid::new_v4().simple().to_string();
        let file = format!("{uid}.yaml");
        std::fs::create_dir_all(&self.dir)?;
        crate::config::atomic_write(&self.file_path(&file)?, content.as_bytes())?;
        let item = ProfileItem {
            uid,
            item_type: "local".into(),
            name: name.unwrap_or_else(|| node.default_name()),
            url: String::new(),
            file,
            updated_at: Some(now_string()),
            update_interval: 0,
            traffic: None,
            quota: None,
        };
        self.push_item(item.clone())?;
        Ok(item)
    }

    fn push_item(&self, item: ProfileItem) -> Result<()> {
        let _guard = PROFILE_LOCK.lock();
        let mut ps = self.load()?;
        if ps.items.is_empty() {
            ps.current = Some(item.uid.clone());
        }
        ps.items.push(item);
        self.save(&ps)
    }

    /// 更新订阅（重新下载）
    pub async fn update(&self, uid: &str) -> Result<()> {
        let original = {
            let _guard = PROFILE_LOCK.lock();
            self.load()?
                .items
                .into_iter()
                .find(|i| i.uid == uid)
                .ok_or_else(|| anyhow!("配置不存在"))?
        };
        if original.item_type != "remote" {
            return Err(anyhow!("本地配置不支持在线更新"));
        }
        // 旧版本可能存下内网地址，更新前同样重新校验，不合格时直接拒绝下载。
        let target = validated_subscription_url(&original.url)?;
        let (content, traffic) = download_text(&target).await?;
        validate_yaml(&content)?;
        let _guard = PROFILE_LOCK.lock();
        let mut ps = self.load()?;
        let idx = ps
            .items
            .iter()
            .position(|i| i.uid == uid)
            .ok_or_else(|| anyhow!("配置不存在"))?;
        let item = &mut ps.items[idx];
        if item.url != original.url
            || item.file != original.file
            || item.item_type != "remote"
            || item.updated_at != original.updated_at
        {
            return Err(anyhow!("订阅在下载期间已改变，请重新更新"));
        }
        std::fs::create_dir_all(&self.dir)?;
        crate::config::atomic_write(&self.file_path(&item.file)?, content.as_bytes())?;
        item.updated_at = Some(now_string());
        if traffic.is_some() {
            item.traffic = traffic;
        }
        self.save(&ps)
    }

    pub fn delete(&self, uid: &str) -> Result<()> {
        let _guard = PROFILE_LOCK.lock();
        let mut ps = self.load()?;
        if let Some(idx) = ps.items.iter().position(|i| i.uid == uid) {
            let item = ps.items.remove(idx);
            if ps.current.as_deref() == Some(uid) {
                ps.current = ps.items.first().map(|i| i.uid.clone());
            }
            self.save(&ps)?;
            let _ = std::fs::remove_file(self.file_path(&item.file)?);
        }
        Ok(())
    }

    pub fn patch(
        &self,
        uid: &str,
        name: Option<String>,
        update_interval: Option<u64>,
    ) -> Result<()> {
        let _guard = PROFILE_LOCK.lock();
        let mut ps = self.load()?;
        let idx = ps
            .items
            .iter()
            .position(|i| i.uid == uid)
            .ok_or_else(|| anyhow!("配置不存在"))?;
        if let Some(name) = name {
            ps.items[idx].name = name;
        }
        if let Some(v) = update_interval {
            ps.items[idx].update_interval = v;
        }
        self.save(&ps)
    }

    /// 写入或清除手动额度。
    ///
    /// 只改本地记录，不重载内核：额度是展示用的元数据，改它不该打断正在跑的连接。
    pub fn set_quota(&self, uid: &str, quota: Option<ManualQuota>) -> Result<()> {
        let _guard = PROFILE_LOCK.lock();
        let mut ps = self.load()?;
        let idx = ps
            .items
            .iter()
            .position(|i| i.uid == uid)
            .ok_or_else(|| anyhow!("配置不存在"))?;
        ps.items[idx].quota = quota.filter(|q| !q.is_blank()).map(ManualQuota::normalized);
        self.save(&ps)
    }

    pub fn select(&self, uid: &str) -> Result<()> {
        let _guard = PROFILE_LOCK.lock();
        let mut ps = self.load()?;
        if !ps.items.iter().any(|i| i.uid == uid) {
            return Err(anyhow!("配置不存在"));
        }
        ps.current = Some(uid.into());
        self.save(&ps)
    }

    /// 读取当前订阅的原始 YAML
    pub fn current_content(&self) -> Result<Option<String>> {
        let ps = self.load()?;
        let Some(uid) = ps.current else {
            return Ok(None);
        };
        let Some(item) = ps.items.iter().find(|i| i.uid == uid) else {
            return Ok(None);
        };
        let path = self.file_path(&item.file)?;
        if !path.exists() {
            return Err(anyhow!("当前订阅文件缺失，请重新导入或切换配置"));
        }
        Ok(Some(std::fs::read_to_string(path)?))
    }

    /// 当前订阅的摘要信息
    pub fn current_summary(&self) -> Result<Option<ProfileItem>> {
        let ps = self.load()?;
        let Some(uid) = ps.current else {
            return Ok(None);
        };
        Ok(ps.items.into_iter().find(|i| i.uid == uid))
    }
}

/// 读取本地订阅文件：先判断路径形态与文件类型，再打开读取。
/// `\\.\pipe\...` 这类设备路径会在 open 处无限期阻塞，而调用方此时可能持有全局锁，
/// 因此必须在 open 之前拒绝。
fn read_local_subscription(source: &Path) -> Result<String> {
    use std::io::Read;

    let raw = source.as_os_str().to_string_lossy();
    let normalized = raw.replace('/', "\\");
    if normalized.starts_with(r"\\.\") || normalized.starts_with(r"\\?\") {
        return Err(anyhow!("不支持设备路径或扩展路径作为订阅文件"));
    }
    let meta = std::fs::symlink_metadata(source)
        .with_context(|| format!("读取文件失败：{}", source.display()))?;
    if !meta.is_file() {
        return Err(anyhow!(
            "本地订阅必须是普通文件，不能是目录、符号链接或设备"
        ));
    }
    if meta.len() > MAX_PROFILE_BYTES as u64 {
        return Err(anyhow!("本地订阅文件超过 10 MiB 或不是普通文件"));
    }
    let file = std::fs::File::open(source)
        .with_context(|| format!("读取文件失败：{}", source.display()))?;
    // 打开后复核句柄本身，避免检查与打开之间路径被替换。
    let opened = file
        .metadata()
        .with_context(|| format!("读取文件失败：{}", source.display()))?;
    if !opened.is_file() || opened.len() > MAX_PROFILE_BYTES as u64 {
        return Err(anyhow!("本地订阅文件超过 10 MiB 或不是普通文件"));
    }
    let mut content = String::new();
    file.take((MAX_PROFILE_BYTES + 1) as u64)
        .read_to_string(&mut content)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::InvalidData {
                anyhow!("本地订阅文件不是 UTF-8 文本：{e}")
            } else {
                anyhow!("读取文件失败：{}：{e}", source.display())
            }
        })?;
    if content.len() > MAX_PROFILE_BYTES {
        return Err(anyhow!("本地订阅文件超过 10 MiB"));
    }
    validate_yaml(&content)?;
    Ok(content)
}

pub(crate) fn validate_yaml(content: &str) -> Result<()> {
    if content.len() > MAX_PROFILE_BYTES {
        return Err(anyhow!("文件过大"));
    }
    // serde_yaml 只限制嵌套深度，不限制别名展开量：先按别名引用次数拦截别名炸弹。
    let aliases = count_yaml_aliases(content);
    if aliases > MAX_YAML_ALIASES {
        return Err(anyhow!(
            "YAML 别名引用过多（{aliases} 处，上限 {MAX_YAML_ALIASES} 处），已拒绝解析"
        ));
    }
    let value: Value =
        serde_yaml::from_str(content).with_context(|| "文件不是有效的 YAML".to_string())?;
    let is_clash = value
        .get("proxies")
        .map(|v| v.is_sequence())
        .unwrap_or(false)
        || value
            .get("proxy-providers")
            .map(|v| v.is_mapping())
            .unwrap_or(false);
    if !is_clash {
        return Err(anyhow!(
            "文件缺少 proxies / proxy-providers，可能不是 Clash 订阅"
        ));
    }
    Ok(())
}

/// 统计 `*alias` 形式的别名引用个数。
/// 只认 YAML 记号起始处的 `*`：正则过滤（`filter: ".*"`）和密码里的星号不算别名，
/// 避免正常订阅被误判成别名炸弹。
fn count_yaml_aliases(content: &str) -> usize {
    let bytes = content.as_bytes();
    let mut count = 0usize;
    for (idx, byte) in bytes.iter().enumerate() {
        if *byte != b'*' {
            continue;
        }
        let at_token_start = match idx.checked_sub(1).map(|i| bytes[i]) {
            None => true,
            Some(prev) => matches!(
                prev,
                b' ' | b'\t' | b'\r' | b'\n' | b'[' | b'{' | b',' | b'-' | b':'
            ),
        };
        if at_token_start {
            count += 1;
        }
    }
    count
}

/// 单节点接入串（代理商给的一条上游代理）。
#[derive(Debug, Clone, PartialEq, Eq)]
struct NodeSpec {
    /// mihomo 的 proxy type：socks5 / http
    protocol: &'static str,
    server: String,
    port: u16,
    username: String,
    password: String,
    tls: bool,
}

impl NodeSpec {
    fn default_name(&self) -> String {
        format!("{}:{}", self.server, self.port)
    }

    /// 生成只有单个节点的 Clash 配置：proxies + 一个 select 组 + MATCH 规则。
    fn to_yaml(&self, name: Option<&str>) -> Result<String> {
        use serde_yaml::Mapping;
        let name = name
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| self.default_name());
        let mut proxy = Mapping::new();
        proxy.insert("name".into(), name.clone().into());
        proxy.insert("type".into(), self.protocol.into());
        proxy.insert("server".into(), self.server.clone().into());
        proxy.insert("port".into(), self.port.into());
        if !self.username.is_empty() {
            proxy.insert("username".into(), self.username.clone().into());
        }
        if !self.password.is_empty() {
            proxy.insert("password".into(), self.password.clone().into());
        }
        if self.tls {
            proxy.insert("tls".into(), true.into());
        }
        // 手工/住宅节点通常没有 UDP 转发：显式关闭，TUN 全局时就不会悄悄吞掉 UDP。
        proxy.insert("udp".into(), false.into());
        let mut group = Mapping::new();
        group.insert("name".into(), "节点选择".into());
        group.insert("type".into(), "select".into());
        group.insert("proxies".into(), Value::Sequence(vec![name.clone().into()]));
        let mut root = Mapping::new();
        root.insert(
            "proxies".into(),
            Value::Sequence(vec![Value::Mapping(proxy)]),
        );
        root.insert(
            "proxy-groups".into(),
            Value::Sequence(vec![Value::Mapping(group)]),
        );
        root.insert(
            "rules".into(),
            Value::Sequence(vec!["MATCH,节点选择".into()]),
        );
        Ok(serde_yaml::to_string(&Value::Mapping(root))?)
    }
}

fn valid_node_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '[' | ']'))
}

fn node_port(text: &str) -> Result<u16> {
    let port: u16 = text
        .trim()
        .parse()
        .map_err(|_| anyhow!("端口无效：{text}"))?;
    if port == 0 {
        return Err(anyhow!("端口不能为 0"));
    }
    Ok(port)
}

/// 拆 `host:port`，兼容 `[2001:db8::1]:1080` 这种带方括号的 IPv6。
fn split_host_port(text: &str) -> Result<(String, u16)> {
    if let Some(rest) = text.strip_prefix('[') {
        let (host, tail) = rest
            .split_once(']')
            .ok_or_else(|| anyhow!("IPv6 地址缺少右方括号"))?;
        let port = tail.strip_prefix(':').ok_or_else(|| anyhow!("缺少端口"))?;
        return Ok((host.to_string(), node_port(port)?));
    }
    let (host, port) = text
        .rsplit_once(':')
        .ok_or_else(|| anyhow!("需要 host:port 形式"))?;
    Ok((host.to_string(), node_port(port)?))
}

fn build_node(
    protocol: &'static str,
    server: String,
    port: u16,
    username: String,
    password: String,
    tls: bool,
) -> Result<NodeSpec> {
    if !valid_node_host(&server) {
        return Err(anyhow!("服务器地址不合法：{server}"));
    }
    Ok(NodeSpec {
        protocol,
        server,
        port,
        username,
        password,
        tls,
    })
}

/// 解析节点串。支持：
/// - `socks5://user:pass@host:port` / `http://host:port` / `https://...`
/// - `user:pass@host:port`
/// - `host:port:username:password` / `host:port:token` / `host:port`
fn parse_node_spec(raw: &str, protocol: Option<&str>) -> Result<NodeSpec> {
    let text = raw.trim();
    if text.is_empty() {
        return Err(anyhow!("节点串不能为空"));
    }
    if text.len() > MAX_SUBSCRIPTION_URL_LEN {
        return Err(anyhow!("节点串超过 {MAX_SUBSCRIPTION_URL_LEN} 字节上限"));
    }
    if text.chars().any(char::is_control) {
        return Err(anyhow!("节点串包含不可见的控制字符"));
    }
    let hint = match protocol.map(|value| value.trim().to_ascii_lowercase()) {
        None => None,
        Some(value) if value == "socks5" || value == "socks5h" => Some(("socks5", false)),
        Some(value) if value == "http" => Some(("http", false)),
        Some(value) if value == "https" => Some(("http", true)),
        Some(other) => return Err(anyhow!("不支持的协议：{other}（支持 socks5 / http）")),
    };

    if text.contains("://") {
        let scheme = text.split_once("://").map(|(s, _)| s.to_ascii_lowercase());
        let (proto, tls) = match scheme.as_deref() {
            Some("socks5") | Some("socks5h") | Some("socks") => ("socks5", false),
            Some("http") => ("http", false),
            Some("https") => ("http", true),
            other => {
                return Err(anyhow!(
                    "不支持的协议：{}（支持 socks5 / http）",
                    other.unwrap_or("未知")
                ));
            }
        };
        let url = Url::parse(text).map_err(|e| anyhow!("节点串格式不正确：{e}"))?;
        let host = url
            .host_str()
            .filter(|host| !host.is_empty())
            .ok_or_else(|| anyhow!("缺少服务器地址"))?;
        let port = url.port().ok_or_else(|| anyhow!("缺少端口"))?;
        return build_node(
            proto,
            host.to_string(),
            port,
            url.username().to_string(),
            url.password().unwrap_or_default().to_string(),
            tls,
        );
    }

    let (proto, tls) = hint.unwrap_or(("socks5", false));

    // user:pass@host:port
    if let Some((userinfo, hostpart)) = text.rsplit_once('@') {
        let (server, port) = split_host_port(hostpart)?;
        let (username, password) = match userinfo.split_once(':') {
            Some((user, pass)) => (user.to_string(), pass.to_string()),
            None => (userinfo.to_string(), String::new()),
        };
        return build_node(proto, server, port, username, password, tls);
    }

    // host:port[:user[:pass]]
    if text.starts_with('[') {
        let (server, port) = split_host_port(text)?;
        return build_node(proto, server, port, String::new(), String::new(), tls);
    }
    let parts: Vec<&str> = text.split(':').collect();
    match parts.len() {
        2 => build_node(
            proto,
            parts[0].to_string(),
            node_port(parts[1])?,
            String::new(),
            String::new(),
            tls,
        ),
        3 => build_node(
            proto,
            parts[0].to_string(),
            node_port(parts[1])?,
            parts[2].to_string(),
            String::new(),
            tls,
        ),
        4 => build_node(
            proto,
            parts[0].to_string(),
            node_port(parts[1])?,
            parts[2].to_string(),
            parts[3].to_string(),
            tls,
        ),
        _ => Err(anyhow!(
            "节点串格式不正确：应为 host:port[:账号[:密码]] 或 socks5://账号:密码@host:port"
        )),
    }
}

/// 订阅地址只做最基本的规范化与长度限制。
/// 允许 http、内网地址与非 80/443 端口：自建面板常常就是这种形态，
/// 把用户自己填的订阅地址当作不可信输入去拦内网，会先打断合法用法。
fn validated_subscription_url(raw: &str) -> Result<Url> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("订阅地址不能为空"));
    }
    if trimmed.len() > MAX_SUBSCRIPTION_URL_LEN {
        return Err(anyhow!("订阅地址超过 {MAX_SUBSCRIPTION_URL_LEN} 字节上限"));
    }
    let url = Url::parse(trimmed).map_err(|e| anyhow!("订阅地址格式不正确：{e}"))?;
    match url.scheme() {
        "http" | "https" => Ok(url),
        other => Err(anyhow!("订阅地址只支持 http/https，当前为 {other}")),
    }
}

/// 订阅下载客户端：只加连接超时与重定向跳数上限，不改变用户的实际出网路径。
fn subscription_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::limited(
            MAX_SUBSCRIPTION_REDIRECTS,
        ))
        .user_agent("clash-verge/1.0")
        .build()
        .context("订阅下载客户端初始化失败")
}

async fn download_text(url: &Url) -> Result<(String, Option<TrafficInfo>)> {
    let client = subscription_client()?;
    // 错误统一剥掉 URL：订阅地址常带 token，不能让凭据随错误信息进日志或弹窗。
    let resp = client
        .get(url.clone())
        .send()
        .await
        .map_err(|e| anyhow!("订阅下载失败：{}", e.without_url()))?;
    if !resp.status().is_success() {
        return Err(anyhow!("下载失败: HTTP {}", resp.status()));
    }
    let userinfo = resp
        .headers()
        .get("subscription-userinfo")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let traffic = parse_userinfo(userinfo.as_deref());
    let text = read_limited_text(resp, MAX_PROFILE_BYTES).await?;
    Ok((text, traffic))
}

async fn read_limited_text(mut resp: reqwest::Response, limit: usize) -> Result<String> {
    if resp.content_length().is_some_and(|n| n > limit as u64) {
        return Err(anyhow!("文件过大"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| anyhow!("订阅下载失败：{}", e.without_url()))?
    {
        if chunk.len() > limit - bytes.len() {
            return Err(anyhow!("文件过大"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// 解析 subscription-userinfo: upload=..; download=..; total=..; expire=..
fn parse_userinfo(header: Option<&str>) -> Option<TrafficInfo> {
    let header = header?;
    if header.is_empty() {
        return None;
    }
    let mut t = TrafficInfo::default();
    for part in header.split(';') {
        let mut kv = part.splitn(2, '=');
        let key = kv.next().unwrap_or("").trim().to_lowercase();
        let val = kv.next().unwrap_or("").trim();
        let v = match val.split('.').next().unwrap_or("").parse::<u64>() {
            Ok(x) => x,
            Err(_) => continue,
        };
        match key.as_str() {
            "upload" => t.upload = v,
            "download" => t.download = v,
            "total" => t.total = v,
            "expire" => t.expire = v,
            _ => {}
        }
    }
    if t.total == 0 && t.expire == 0 {
        return None;
    }
    Some(t)
}

fn now_string() -> String {
    chrono::Utc::now().to_rfc3339()
}

pub fn update_due(item: &ProfileItem, now: i64) -> bool {
    if item.item_type != "remote" || item.update_interval == 0 {
        return false;
    }
    // 旧版本无时区时间不能可靠还原，安排一次更新后迁移为带时区时间。
    let Some(at) = item
        .updated_at
        .as_deref()
        .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
    else {
        return true;
    };
    let interval = i64::try_from(item.update_interval.saturating_mul(3600)).unwrap_or(i64::MAX);
    now.saturating_sub(at.timestamp()) >= interval
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn node_spec_accepts_vendor_strings_and_rejects_junk() {
        let spec = parse_node_spec("geo.miyaip.app:8001:token123", None).unwrap();
        assert_eq!(spec.protocol, "socks5");
        assert_eq!(spec.server, "geo.miyaip.app");
        assert_eq!(spec.port, 8001);
        assert_eq!(spec.username, "token123");
        assert!(spec.password.is_empty());

        let spec = parse_node_spec("1.2.3.4:1080:user:pass", Some("http")).unwrap();
        assert_eq!(spec.protocol, "http");
        assert_eq!(spec.password, "pass");

        let spec = parse_node_spec("socks5://u:p@example.com:1080", None).unwrap();
        assert_eq!(spec.server, "example.com");
        assert_eq!(spec.port, 1080);
        assert_eq!(spec.username, "u");
        assert_eq!(spec.password, "p");

        let spec = parse_node_spec("[2001:db8::1]:1080", None).unwrap();
        assert_eq!(spec.server, "2001:db8::1");

        assert!(parse_node_spec("", None).is_err());
        assert!(parse_node_spec("example.com", None).is_err());
        assert!(parse_node_spec("h:99999", None).is_err());
        assert!(parse_node_spec("h:0", None).is_err());
        assert!(parse_node_spec("ftp://h:21", None).is_err());
        assert!(parse_node_spec("h:21:a:b:c", None).is_err());

        let yaml = parse_node_spec("example.com:1080:tok", None)
            .unwrap()
            .to_yaml(Some("住宅IP"))
            .unwrap();
        validate_yaml(&yaml).unwrap();
        assert!(yaml.contains("住宅IP") && yaml.contains("MATCH,节点选择"));
    }

    #[tokio::test]
    async fn local_import_is_bounded_and_does_not_commit_on_failure() {
        let fixture = crate::test_support::FixtureDir::new();
        let input = fixture.path().join("oversized.yaml");
        std::fs::File::create(&input)
            .unwrap()
            .set_len(MAX_PROFILE_BYTES as u64 + 1)
            .unwrap();
        let manager = ProfileManager::new(&fixture.path().join("managed"));
        let error = manager
            .create_local(input.to_str().unwrap(), None)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("10 MiB"));
        assert!(!manager.dir.exists());
        assert!(manager.load().unwrap().items.is_empty());
        std::fs::write(&input, "proxies: []\n").unwrap();
        let imported = manager
            .create_local(input.to_str().unwrap(), None)
            .await
            .unwrap();
        assert_eq!(
            manager.current_summary().unwrap().unwrap().uid,
            imported.uid
        );
        assert_eq!(manager.current_content().unwrap().unwrap(), "proxies: []\n");
    }

    #[tokio::test]
    async fn download_limit_rejects_headers_and_chunked_overflow() {
        for chunked in [false, true] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                    .unwrap();
                let _ = socket.read(&mut [0; 1024]);
                if chunked {
                    let _ = socket.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n");
                    // 超限后不发送结束块，若读取器等待完整响应就会超时。
                    let _ =
                        socket.write_all(b"10\r\n0123456789abcdef\r\n10\r\n0123456789abcdef\r\n");
                } else {
                    let _ = socket.write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 1000000\r\nConnection: close\r\n\r\n",
                    );
                }
                let mut byte = [0; 1];
                let _ = socket.read(&mut byte);
            });
            let response = reqwest::Client::builder()
                .no_proxy()
                .timeout(std::time::Duration::from_secs(2))
                .build()
                .unwrap()
                .get(format!("http://{address}/"))
                .send()
                .await
                .unwrap();
            let started = std::time::Instant::now();
            let error = read_limited_text(response, 24).await.unwrap_err();
            assert!(error.to_string().contains("文件过大"));
            assert!(started.elapsed() < std::time::Duration::from_secs(1));
            server.join().unwrap();
        }
    }

    #[test]
    fn update_schedule_uses_absolute_time_and_migrates_legacy_dates() {
        let mut item = ProfileItem {
            item_type: "remote".into(),
            update_interval: 6,
            updated_at: Some("2026-10-14T08:00:00+08:00".into()),
            ..Default::default()
        };
        let at = chrono::DateTime::parse_from_rfc3339("2026-10-14T00:00:00Z")
            .unwrap()
            .timestamp();
        assert!(!update_due(&item, at + 6 * 3600 - 1));
        assert!(update_due(&item, at + 6 * 3600));
        item.updated_at = Some("2026-10-14 08:00:00".into());
        assert!(update_due(&item, at));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn deleting_during_download_cannot_restore_subscription() {
        // 订阅地址现在必须在建立连接之前通过公网校验，测试不能再连本地服务器制造下载窗口；
        // 这里保留原有的并发形状：删除与更新交错时，任何交错都不得恢复已删除的订阅，
        // 也不得留下订阅文件。全程不产生真实网络请求。
        let fixture = crate::test_support::FixtureDir::new();
        let pm = ProfileManager::new(fixture.path());
        pm.save(&ProfileStore {
            current: Some("fixture".into()),
            items: vec![ProfileItem {
                uid: "fixture".into(),
                item_type: "remote".into(),
                file: "fixture.yaml".into(),
                url: "http://127.0.0.1:9/fixture".into(),
                ..Default::default()
            }],
        })
        .unwrap();
        let update_pm = ProfileManager::new(fixture.path());
        let update = tokio::spawn(async move { update_pm.update("fixture").await });
        pm.delete("fixture").unwrap();
        let result = update.await.unwrap();
        // 两种交错都必须失败：校验阶段被拒，或在锁内发现订阅已被删除。
        assert!(result.is_err(), "更新失败时不得提交任何订阅内容");
        let store: ProfileStore =
            serde_yaml::from_slice(&std::fs::read(fixture.path().join("profiles.yaml")).unwrap())
                .unwrap();
        assert!(store.items.is_empty());
        assert!(store.current.is_none());
        assert!(!fixture.path().join("profiles/fixture.yaml").exists());
    }

    #[test]
    fn subscription_url_allows_self_hosted_panels_and_rejects_other_schemes() {
        // 自建面板的常见形态：明文 http、内网地址、非 80/443 端口，都必须可用。
        for raw in [
            "http://192.168.1.10:8080/sub?token=abc",
            "http://nas.lan:9090/sub",
            "http://127.0.0.1:2096/sub",
            "https://example.com:8443/sub",
            "https://sub.example.com/api/v1/client/subscribe?token=SECRET",
        ] {
            let url = validated_subscription_url(raw).unwrap_or_else(|e| panic!("{raw}: {e}"));
            assert!(matches!(url.scheme(), "http" | "https"));
        }
        // 非 HTTP(S) 与空串不是订阅地址。
        for raw in ["file:///C:/Windows/win.ini", "ftp://example.com/sub", ""] {
            assert!(validated_subscription_url(raw).is_err(), "{raw} 应被拒绝");
        }
    }

    #[test]
    fn yaml_alias_bomb_is_rejected_before_expansion() {
        let bomb = format!(
            "proxies: []\ncommon: &anchored {{ k: v }}\nrepeated: [{}]\n",
            vec!["*anchored"; MAX_YAML_ALIASES + 1].join(", ")
        );
        let error = validate_yaml(&bomb).unwrap_err().to_string();
        assert!(error.contains("别名"), "{error}");
        let harmless = format!(
            "proxies: []\ncommon: &anchored {{ k: v }}\nrepeated: [{}]\n",
            vec!["*anchored"; 4].join(", ")
        );
        assert!(validate_yaml(&harmless).is_ok());
        // 正则过滤里的星号不是别名引用，不能被计数误伤。
        let regex_heavy = format!(
            "proxies: []\nproxy-groups:\n  - name: g\n    type: url-test\n    filter: \"{}\"\n",
            vec![".*"; MAX_YAML_ALIASES + 16].join("|")
        );
        assert!(validate_yaml(&regex_heavy).is_ok());
    }

    #[tokio::test]
    async fn local_import_rejects_device_paths_and_non_utf8_files() {
        let fixture = crate::test_support::FixtureDir::new();
        let manager = ProfileManager::new(&fixture.path().join("managed"));
        // 设备路径必须在 open 之前被拒绝，否则读取线程会在 open 处阻塞并长期占用全局锁。
        for path in [
            r"\\.\pipe\dsh-test",
            r"\\?\C:\Windows\win.ini",
            "//./pipe/dsh-test",
        ] {
            let error = manager.create_local(path, None).await.unwrap_err();
            assert!(error.to_string().contains("设备路径"), "{path}: {error}");
        }
        let broken = fixture.path().join("broken.yaml");
        std::fs::write(&broken, b"proxies: []\nname: \xff\xfe\n").unwrap();
        let error = manager
            .create_local(broken.to_str().unwrap(), None)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("UTF-8"), "{error}");
        assert!(!manager.dir.exists());
        assert!(manager.load().unwrap().items.is_empty());
    }

    #[test]
    fn manual_quota_round_trips_and_blank_clears() {
        let fixture = crate::test_support::FixtureDir::new();
        let pm = ProfileManager::new(fixture.path());
        pm.save(&ProfileStore {
            current: Some("p1".into()),
            items: vec![ProfileItem {
                uid: "p1".into(),
                item_type: "local".into(),
                file: "p1.yaml".into(),
                ..Default::default()
            }],
        })
        .unwrap();

        let quota = ManualQuota {
            unlimited: false,
            total: 210 * 1024 * 1024 * 1024,
            expire: 1_800_000_000,
        };
        pm.set_quota("p1", Some(quota)).unwrap();
        assert_eq!(pm.current_summary().unwrap().unwrap().quota, Some(quota));

        // 什么都没填：清掉，而不是留下「不限量 · 不到期」。
        pm.set_quota("p1", Some(ManualQuota::default())).unwrap();
        assert!(pm.current_summary().unwrap().unwrap().quota.is_none());
        pm.set_quota("p1", None).unwrap();
        assert!(pm.current_summary().unwrap().unwrap().quota.is_none());

        // 不限量 + 不到期是合法组合，不能因为两项都为空就被当成没填。
        let perpetual = ManualQuota {
            unlimited: true,
            total: 0,
            expire: 0,
        };
        pm.set_quota("p1", Some(perpetual)).unwrap();
        assert_eq!(
            pm.current_summary().unwrap().unwrap().quota,
            Some(perpetual)
        );

        // 只有到期、没有总量：旧写法等于不限量，读回时补上标志并清空总量。
        pm.set_quota(
            "p1",
            Some(ManualQuota {
                unlimited: false,
                total: 0,
                expire: 1_800_000_000,
            }),
        )
        .unwrap();
        let stored = pm.current_summary().unwrap().unwrap().quota.unwrap();
        assert!(stored.unlimited);
        assert_eq!(stored.expire, 1_800_000_000);
        assert_eq!(stored.total, 0);

        assert!(pm.set_quota("missing", None).is_err());
    }

    #[test]
    fn legacy_profile_store_without_quota_still_loads() {
        // 旧 profiles.yaml 没有 quota 键，升级后不得因为反序列化失败而丢订阅。
        let store: ProfileStore = serde_yaml::from_str(
            "current: old\nitems:\n  - uid: old\n    type: remote\n    name: 旧订阅\n    file: old.yaml\n    update_interval: 0\n",
        )
        .unwrap();
        assert_eq!(store.items.len(), 1);
        assert!(store.items[0].quota.is_none());
        assert!(store.items[0].traffic.is_none());
    }
}
