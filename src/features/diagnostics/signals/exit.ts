/**
 * 网络出口一致性层：出口 IP（v4/v6）、IP×时区×语言三角矛盾、机房/住宅推断。
 *
 * 检测思路移植自开源项目 Hype-yolo/claude-ban-check（MIT License）
 * https://github.com/Hype-yolo/claude-ban-check —— 探测端点与 ASN 关键词保持一致。
 * 仅查询公开的出口 IP（任何网站都能看到的公开信息），其余全部本地计算。
 */

export type Verdict = "g" | "y" | "r" | "n"; // 绿 / 黄 / 红 / 未知

export interface GeoInfo {
  ip: string;
  cc: string; // ISO 国家码，如 US / CN
  country: string;
  city: string;
  asn: string;
  isp: string;
  tzId: string;
  src: string;
}

export interface ExitResult {
  state: "ok" | "fail" | "loading";
  /** 主出口（v4 优先）地理信息 */
  primary: GeoInfo | null;
  /** 公网 IPv6（可直连时存在） */
  ip6: string | null;
  /** IPv6 判定 */
  v6Verdict: Verdict;
  v6Msg: string;
  /** 三角矛盾 */
  triangleVerdict: Verdict;
  triangleMsg: string;
  /** ASN 机房/住宅推断（供参考） */
  asnClass: "dc" | "res" | "unknown";
  at: number;
}

/** ASN / 运营商关键词（推断机房 vs 住宅，供参考） */
const DC_KW = [
  "alibaba",
  "aliyun",
  "alicloud",
  "tencent",
  "huawei",
  "ucloud",
  "kingsoft",
  "volc",
  "bytedance",
  "amazon",
  "aws",
  "ec2",
  "google",
  "gcp",
  "microsoft",
  "azure",
  "oracle cloud",
  "ovh",
  "m247",
  "datacamp",
  "psychz",
  "quadranet",
  "zenlayer",
  "digitalocean",
  "linode",
  "akamai",
  "vultr",
  "choopa",
  "hetzner",
  "leaseweb",
  "contabo",
  "gcore",
  "g-core",
  "cogent",
  "hostwinds",
  "colocation",
  "colocrossing",
  "hosting",
  "datacenter",
  "data center",
  "cloud",
  "server",
  "dedicated",
  "vps",
  "limelight",
  "fastly",
  "cloudflare",
  "incapsula",
  "constant company",
  "nexeon",
  "hostroyale",
  "ipxo",
  "packethub",
  "prime security",
  "xtom",
  "reliablesite",
  "hostodo",
  "frantech",
  "buyvm",
  "tzulo",
];
const RES_KW = [
  "telecom",
  "chinanet",
  "unicom",
  "china mobile",
  "cmcc",
  "comcast",
  "verizon",
  "at&t",
  "spectrum",
  "charter",
  "centurylink",
  "frontier",
  "optimum",
  "cablevision",
  "ntt",
  "kddi",
  "softbank",
  "docomo",
  "biglobe",
  "so-net",
  "deutsche telekom",
  "vodafone",
  "orange",
  "telefonica",
  "movistar",
  "virgin media",
  "sky broadband",
  "telstra",
  "optus",
  "rogers",
  "bell canada",
  "shaw",
  "telus",
  "sk broadband",
  "chunghwa",
  "hinet",
  "broadband",
  "fiber",
  "fibre",
  "cable",
  "dsl",
  "residential",
  "fttx",
  "gpon",
  "communications",
];

export function systemTzInfo(): { cnTz: boolean; tz: string } {
  let tz = "";
  try {
    tz = Intl.DateTimeFormat().resolvedOptions().timeZone || "";
  } catch {
    tz = "";
  }
  if (!tz) {
    const off = -new Date().getTimezoneOffset() / 60;
    return { cnTz: off === 8, tz: `UTC${off >= 0 ? "+" : ""}${off}(推测)` };
  }
  return { cnTz: tz === "Asia/Shanghai" || tz === "Asia/Urumqi", tz };
}

export function browserLangCn(): boolean {
  const langs =
    navigator.languages && navigator.languages.length
      ? navigator.languages
      : [navigator.language || ""];
  return langs.some((l) => /^zh\b/i.test(l || ""));
}

function fetchJSON(url: string, ms = 6500): Promise<unknown> {
  return new Promise((resolve, reject) => {
    const ctl = new AbortController();
    const t = setTimeout(() => ctl.abort(), ms);
    fetch(url, {
      signal: ctl.signal,
      cache: "no-store",
      referrerPolicy: "no-referrer",
    })
      .then((r) =>
        r.ok ? r.json() : Promise.reject(new Error(`http ${r.status}`)),
      )
      .then((j) => {
        clearTimeout(t);
        resolve(j);
      })
      .catch((e) => {
        clearTimeout(t);
        reject(e);
      });
  });
}

interface RawIpwho {
  success?: boolean;
  ip?: string;
  country?: string;
  country_code?: string;
  city?: string;
  connection?: { asn?: number; isp?: string; org?: string };
  timezone?: { id?: string };
}
interface RawIpapi {
  error?: boolean;
  ip?: string;
  country_code?: string;
  country_name?: string;
  city?: string;
  asn?: string | number;
  org?: string;
  org_name?: string;
  timezone?: string;
}
interface RawIpinfo {
  ip?: string;
  city?: string;
  region?: string;
  country?: string;
  timezone?: string;
  org?: string;
}
interface RawFree {
  ipAddress?: string;
  countryCode?: string;
  countryName?: string;
  cityName?: string;
  timeZone?: string;
}

function normIpwho(j: RawIpwho): GeoInfo | null {
  if (!j || j.success === false || !j.ip) return null;
  return {
    ip: j.ip,
    cc: (j.country_code || "").toUpperCase(),
    country: j.country || "",
    city: j.city || "",
    asn: j.connection?.asn ? `AS${j.connection.asn}` : "",
    isp: j.connection?.isp || j.connection?.org || "",
    tzId: j.timezone?.id || "",
    src: "ipwho.is",
  };
}
function normIpinfo(j: RawIpinfo): GeoInfo | null {
  if (!j || !j.ip) return null;
  const code = (j.country || "").toUpperCase();
  let country = code;
  if (code) {
    try {
      country =
        new Intl.DisplayNames(["en"], { type: "region" }).of(code) || code;
    } catch {
      /* 旧 WebView 没有 DisplayNames，保留国家码 */
    }
  }
  const org = j.org || "";
  const asn = org.match(/^AS\d+/i)?.[0] || "";
  return {
    ip: j.ip,
    cc: code,
    country,
    city: j.city || "",
    asn,
    isp: org.replace(/^AS\d+\s*/i, ""),
    tzId: j.timezone || "",
    src: "ipinfo.io",
  };
}
function normIpapi(j: RawIpapi): GeoInfo | null {
  if (!j || j.error || (!j.ip && !j.country_code)) return null;
  return {
    ip: j.ip || "",
    cc: (j.country_code || "").toUpperCase(),
    country: j.country_name || "",
    city: j.city || "",
    asn: j.asn ? String(j.asn) : "",
    isp: j.org || j.org_name || "",
    tzId: j.timezone || "",
    src: "ipapi.co",
  };
}
function normFree(j: RawFree): GeoInfo | null {
  if (!j || !j.ipAddress) return null;
  return {
    ip: j.ipAddress,
    cc: (j.countryCode || "").toUpperCase(),
    country: j.countryName || "",
    city: j.cityName || "",
    asn: "",
    isp: "",
    tzId: j.timeZone || "",
    src: "freeipapi.com",
  };
}

/** 归属地查询（带三个数据源回退） */
export async function geoOf(ip?: string): Promise<GeoInfo | null> {
  const q = ip ? encodeURIComponent(ip) : "";
  try {
    const r = normIpwho((await fetchJSON(`https://ipwho.is/${q}`)) as RawIpwho);
    if (r) return r;
  } catch {
    /* 回退 */
  }
  try {
    const r = normIpinfo(
      (await fetchJSON(
        `https://ipinfo.io/${ip ? q + "/json" : "json"}`,
      )) as RawIpinfo,
    );
    if (r) return r;
  } catch {
    /* 回退 */
  }
  try {
    const r = normIpapi(
      (await fetchJSON(
        `https://ipapi.co/${ip ? q + "/json/" : "json/"}`,
      )) as RawIpapi,
    );
    if (r) return r;
  } catch {
    /* 回退 */
  }
  try {
    const r = normFree(
      (await fetchJSON(`https://freeipapi.com/api/json/${q}`)) as RawFree,
    );
    if (r) return r;
  } catch {
    /* 全部失败 */
  }
  return null;
}

export function classifyASN(text: string): "dc" | "res" | "unknown" {
  const s = (text || "").toLowerCase();
  if (!s) return "unknown";
  for (const k of DC_KW) if (s.includes(k)) return "dc";
  for (const k of RES_KW) if (s.includes(k)) return "res";
  return "unknown";
}

/**
 * 三角矛盾判定。
 *
 * `proxied` 表示浏览器这一路是否真的走了代理：页面里的 IP 探测走的是 WebView
 * 自己的网络栈，代理没对浏览器生效时它看到的是本地线路。那种情况下拿它反推的
 * 时区去和「按出口同步过的系统时区」比，比的是两条不同的线路，不算矛盾——
 * 该由「系统代理 / TUN 未开启」那一条去提示，这里只做说明，不判黄。
 */
export function judgeTriangle(
  g: GeoInfo,
  ctx: { cnTz: boolean; tz: string; langCn: boolean; proxied: boolean },
): { cls: Verdict; msg: string } {
  if (g.cc === "CN")
    return { cls: "r", msg: "出口 IP 落在中国 —— 直接暴露在封锁地区" };
  const bad: string[] = [];
  if (ctx.cnTz) bad.push(`系统时区=${ctx.tz}`);
  if (ctx.langCn) bad.push("浏览器语言含中文");
  if (bad.length)
    return {
      cls: "r",
      msg: `身份矛盾：海外 IP（${g.cc || "?"}）配 ${bad.join(" + ")} —— 风控最爱抓这条`,
    };
  if (g.tzId && g.tzId !== ctx.tz) {
    if (!ctx.proxied)
      return {
        cls: "n",
        msg: `浏览器当前没走代理：这条探测看到的是本地线路（${g.tzId}），系统时区是 ${ctx.tz}。两条线路不同源，不按矛盾计分；要让浏览器里的 IP 与时区一致，请开启 TUN 或系统代理后重新检测。`,
      };
    return {
      cls: "y",
      msg: `IP 反推时区（${g.tzId}）与系统时区（${ctx.tz}）不一致，建议对齐`,
    };
  }
  return {
    cls: "g",
    msg: `IP / 时区 / 语言 三者一致（${g.cc}），是干净的海外身份`,
  };
}

/** 运行出口一致性层。`proxied` 由调用方按当前系统代理 / TUN 状态传入。 */
export async function runExitLayer(proxied = false): Promise<ExitResult> {
  const [probes, ctx] = await Promise.all([
    Promise.allSettled([
      fetchJSON("https://api4.ipify.org?format=json", 5000) as Promise<{
        ip?: string;
      }>,
      fetchJSON("https://api6.ipify.org?format=json", 5000) as Promise<{
        ip?: string;
      }>,
    ]),
    Promise.resolve({
      cnTz: systemTzInfo().cnTz,
      tz: systemTzInfo().tz,
      langCn: browserLangCn(),
      proxied,
    }),
  ]);

  const ip4 =
    probes[0].status === "fulfilled"
      ? ((probes[0].value as { ip?: string })?.ip ?? null)
      : null;
  const ip6 =
    probes[1].status === "fulfilled"
      ? ((probes[1].value as { ip?: string })?.ip ?? null)
      : null;

  const primary = await geoOf(ip4 || undefined);
  if (!primary) {
    return {
      state: "fail",
      primary: null,
      ip6,
      v6Verdict: "n",
      v6Msg: ip6
        ? `检测到公网 IPv6（${ip6}），但归属地查询失败`
        : "无公网 IPv6 暴露",
      triangleVerdict: "n",
      triangleMsg: "—",
      asnClass: "unknown",
      at: Date.now(),
    };
  }

  const asnClass = classifyASN(`${primary.isp} ${primary.asn}`);

  // IPv6 判定：有公网 v6 就 geo 它
  const hasV6 = !!(ip6 && ip6 !== primary.ip);
  let v6Verdict: Verdict = "g";
  let v6Msg = ip6 ? `检测到公网 IPv6：${ip6}` : "无公网 IPv6 暴露";
  if (hasV6) {
    const g6 = await geoOf(ip6!);
    if (g6 && g6.cc === "CN") {
      v6Verdict = "r";
      v6Msg = `IPv6 落在中国（${ip6}）—— 若代理只罩 v4，真实位置从 v6 泄漏`;
    } else if (g6 && primary.cc && g6.cc !== primary.cc) {
      v6Verdict = "y";
      v6Msg = `IPv6 出口国家（${g6.cc}）与主出口（${primary.cc}）不一致，可能绕过代理`;
    } else {
      v6Verdict = "y";
      v6Msg = `存在可直连的公网 IPv6（${ip6}）—— 确认代理已覆盖 v6`;
    }
  }

  const tri = judgeTriangle(primary, ctx);

  return {
    state: "ok",
    primary,
    ip6,
    v6Verdict,
    v6Msg,
    triangleVerdict: tri.cls,
    triangleMsg: tri.msg,
    asnClass,
    at: Date.now(),
  };
}
