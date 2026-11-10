/* 随机令牌回环测量页，无 Tauri 权限。 */
import { probeWebRtc, probeDns } from "./leak.js";
const text = (value) => (value == null ? "未知" : String(value));
const probeOutcome = {
  complete: "已完成",
  timeout: "超时",
  error: "错误",
  unsupported: "不可用",
};
async function request(route, options = {}) {
  const response = await fetch(new URL(route, location.href), {
    cache: "no-store",
    credentials: "omit",
    redirect: "error",
    signal: AbortSignal.timeout(12000),
    ...options,
  });
  const value = await response.json();
  if (!response.ok)
    throw new Error(
      value.error || "本机检测会话不可用，请返回主程序重新启动检测",
    );
  return value;
}
async function exit() {
  for (const url of ["https://ipwho.is/", "https://ipapi.co/json/"]) {
    try {
      const response = await fetch(url, {
        cache: "no-store",
        credentials: "omit",
        redirect: "error",
        signal: AbortSignal.timeout(8000),
      });
      if (!response.ok) continue;
      const data = await response.json();
      if (
        data.success === false ||
        data.error ||
        typeof data.ip !== "string" ||
        typeof data.country_code !== "string"
      )
        continue;
      return {
        ip: data.ip,
        country: data.country_code,
        exitTimezone:
          typeof data.timezone === "string"
            ? data.timezone
            : (data.timezone?.id ?? null),
      };
    } catch {}
  }
  throw new Error("浏览器出口查询失败，尚未核验出口 IP");
}
function renderRows(rows) {
  const container = document.getElementById("results");
  container.replaceChildren();
  for (const item of rows) {
    const row = document.createElement("div");
    row.className = "row";
    for (const value of [
      item.name,
      item.name === "检测错误" && item.actual === null
        ? "无"
        : text(item.actual),
      text(item.expected),
      item.ok
        ? "通过"
        : item.state === "risk"
          ? "风险"
          : item.state === "unknown"
            ? "未知"
            : "待处理",
    ]) {
      const span = document.createElement("span");
      span.textContent = value;
      row.append(span);
    }
    row.lastChild.className = item.ok
      ? "pass"
      : item.state === "risk"
        ? "risk"
        : "fail";
    container.append(row);
  }
}
function renderEvidence(dns, webrtc) {
  const container = document.getElementById("evidence");
  container.replaceChildren();
  const add = (message) => {
    const p = document.createElement("p");
    p.textContent = message;
    container.append(p);
  };
  add(`DNS 观察服务看到的 HTTP 出口：${dns?.clientIp ?? "未知"}`);
  add(
    `DNS 权威观察：${dns?.observed ?? 0} / ${dns?.expected ?? 0} 个域名。A/AAAA 测试域名均会尝试；AAAA 查询不等于 IPv6 传输已经通过。`,
  );
  for (const resolver of dns?.resolvers ?? [])
    add(
      `解析器 ${resolver.ip} · ${resolver.operator || "运营商未知"} · ${resolver.country || "地区未知"} · 权威侧 ${resolver.transport || "传输未知"}${resolver.ecs?.length ? ` · ECS ${resolver.ecs.join("、")}` : ""}`,
    );
  for (const probe of webrtc.probes) {
    add(
      `WebRTC ${probe.server === "local" ? "本机候选" : probe.server}：${probeOutcome[probe.outcome] ?? probe.outcome}${probe.error ? `（${probe.error}）` : ""}`,
    );
    for (const c of probe.candidates)
      add(
        `  ${c.kind} / ${c.protocol}：${c.address}${c.relatedAddress ? ` · 关联地址 ${c.relatedAddress}` : ""}`,
      );
  }
}
async function run() {
  const button = document.getElementById("retry");
  button.disabled = true;
  document.body.classList.remove("success");
  document.getElementById("title").textContent =
    "正在核验浏览器、DNS 与 WebRTC…";
  document.getElementById("results").replaceChildren();
  document.getElementById("evidence").replaceChildren();
  document.getElementById("summary").textContent =
    "DNS 将访问随机测试域名；WebRTC 不申请摄像头或麦克风权限。";
  try {
    const { runId } = await request("begin", { method: "POST" });
    await request("context");
    const rtcTask = probeWebRtc();
    const dnsTask = probeDns(request, runId).catch(() => null);
    let network = { ip: null, country: null, exitTimezone: null },
      error = null;
    try {
      network = await exit();
    } catch (e) {
      error = e.message;
    }
    const high = await navigator.userAgentData
      ?.getHighEntropyValues(["uaFullVersion"])
      .catch(() => null);
    const webrtc = await rtcTask;
    await dnsTask;
    const report = {
      runId,
      language: navigator.language,
      languages: [...navigator.languages],
      timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
      platform: navigator.platform,
      cores: navigator.hardwareConcurrency,
      browserVersion:
        high?.uaFullVersion ||
        navigator.userAgent.match(/(?:Chrome|Chromium)\/([\d.]+)/)?.[1] ||
        "",
      ...network,
      error,
      webrtc,
    };
    const result = await request("report", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(report),
    });
    renderRows(result.checks);
    renderEvidence(result.dns, webrtc);
    document.getElementById("title").textContent = result.passed
      ? "✓ 本轮已测项目全部通过"
      : "检测存在风险或未知项";
    document.getElementById("summary").textContent =
      "实测结果已回传。严格模式下，DNS 或 WebRTC 未完成、未知或发现风险都会阻止主程序打开网页。";
    if (result.passed) document.body.classList.add("success");
  } catch (e) {
    document.getElementById("title").textContent = "本次核验未完成";
    document.getElementById("summary").textContent = e.message;
  } finally {
    button.disabled = false;
  }
}
document.getElementById("retry").addEventListener("click", run);
run();
