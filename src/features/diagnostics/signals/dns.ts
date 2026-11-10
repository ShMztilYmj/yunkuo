/**
 * DNS 区域泄露检测。
 *
 * 思路参考开源项目 Hype-yolo/claude-ban-check（MIT）的「DNS 解析器」检查与
 * fourierwang66666/claude-ban-risk-check 的「DNS 泄露 = DNS 出口 vs IP 出口国家不一致」：
 * 向内核 DNS 查询解析器身份域名（whoami.akamai.net / o-o.myaddr.l.google.com），
 * 得到实际使用的 DNS 出口 IP，再与出口 IP 国家比对。
 */

import { api } from "../../../services/api";
import { geoOf, type Verdict } from "./exit";

/** 公网地址判定：DNS 解析器与出口候选只看公网地址。 */
export function isPublic(ip: string): boolean {
  if (ip.includes(".local")) return false;
  if (ip.includes(":")) return /^[23][0-9a-f]{0,3}:/i.test(ip); // 包含全球单播 IPv6；不排除所有 IPv6
  const parts = ip.split(".").map(Number);
  if (
    parts.length !== 4 ||
    parts.some((x) => !Number.isInteger(x) || x < 0 || x > 255)
  )
    return false;
  if (parts[0] === 0 || parts[0] >= 224) return false;
  if (parts[0] === 127) return false;
  if (parts[0] === 10) return false;
  if (parts[0] === 192 && parts[1] === 168) return false;
  if (parts[0] === 172 && parts[1] >= 16 && parts[1] <= 31) return false;
  if (parts[0] === 100 && parts[1] >= 64 && parts[1] <= 127) return false; // CGNAT
  if (parts[0] === 169 && parts[1] === 254) return false;
  if (parts[0] === 198 && (parts[1] === 18 || parts[1] === 19)) return false;
  return true;
}

export interface DnsResult {
  state: "ok" | "fail" | "loading";
  /** 实际使用的 DNS 出口 IP（可能为内网/内核地址） */
  resolverIp: string | null;
  /** 查询用的身份域名 */
  via: string | null;
  /** 域名解析结果（fake-ip 时展示） */
  answerNote: string | null;
  verdict: Verdict;
  msg: string;
  at: number;
}

/** 国内公共 DNS（命中即视为运营商/国内 DNS） */
const CN_DNS = new Set([
  "114.114.114.114",
  "114.114.115.115",
  "223.5.5.5",
  "223.6.6.6",
  "119.29.29.29",
  "182.254.116.116",
  "180.76.76.76",
  "1.2.4.8",
  "210.2.4.8",
  "101.226.4.6",
  "117.50.11.11",
]);

interface DnsQueryAnswer {
  name?: string;
  type?: number;
  TTL?: number;
  data?: string;
}

/** 通过原生代理的 DNS 接口查询解析结果。 */
async function dnsQuery(name: string, type: string): Promise<DnsQueryAnswer[]> {
  const data = (await api.nativeRequest(
    "GET",
    `/dns/query?name=${encodeURIComponent(name)}&type=${type}`,
  )) as { Answer?: DnsQueryAnswer[] };
  return data?.Answer ?? [];
}

/** 从解析器身份域名的应答中提取 DNS 出口 IP */
function extractResolverIps(
  answers: DnsQueryAnswer[],
  name: string,
  type: string,
): string[] {
  const ips: string[] = [];
  for (const a of answers) {
    if (a.type !== (type === "TXT" ? 16 : 1)) continue;
    const d = (a.data ?? "").trim();
    const fields = d.replace(/"/g, "").trim().split(/\s+/);
    if (name === "whoami.ds.akahelp.net") {
      // ECS、ip 等字段不代表递归解析器；只接受 ns 身份字段。
      if (fields.length === 2 && fields[0].toLowerCase() === "ns")
        ips.push(fields[1]);
    } else if (fields.length === 1) {
      ips.push(fields[0]);
    }
  }
  return ips;
}

export async function runDnsLayer(exitCc?: string | null): Promise<DnsResult> {
  const identities: Array<{ name: string; type: string; label: string }> = [
    {
      name: "o-o.myaddr.l.google.com",
      type: "TXT",
      label: "o-o.myaddr.l.google.com (TXT)",
    },
    {
      name: "whoami.ds.akahelp.net",
      type: "TXT",
      label: "whoami.ds.akahelp.net (TXT)",
    },
    { name: "whoami.akamai.net", type: "A", label: "whoami.akamai.net (A)" },
  ];

  let filteredAddress = false;
  for (const id of identities) {
    let answers: DnsQueryAnswer[] = [];
    try {
      answers = await dnsQuery(id.name, id.type);
    } catch {
      continue; // 内核离线 / 接口不可用，试下一个
    }
    const candidates = extractResolverIps(answers, id.name, id.type);
    filteredAddress ||= candidates.some((ip) => !isPublic(ip));
    const ip = candidates.find(isPublic);
    if (!ip) continue;

    if (CN_DNS.has(ip)) {
      return {
        state: "ok",
        resolverIp: ip,
        via: `本软件内核 · ${id.label}`,
        answerNote: null,
        verdict: "r",
        msg: `本次内核查询使用国内公共 DNS（${ip}），存在区域泄露风险`,
        at: Date.now(),
      };
    }

    const g = await geoOf(ip).catch(() => null);
    if (!g) {
      return {
        state: "ok",
        resolverIp: ip,
        via: `本软件内核 · ${id.label}`,
        answerNote: null,
        verdict: "y",
        msg: `DNS 出口 IP（${ip}）归属地查询失败，请人工复核`,
        at: Date.now(),
      };
    }
    if (g.cc === "CN") {
      return {
        state: "ok",
        resolverIp: ip,
        via: `本软件内核 · ${id.label}`,
        answerNote: null,
        verdict: "r",
        msg: `DNS 解析器落在中国（${ip} · ${g.isp || g.country}）—— 区域泄露隐患`,
        at: Date.now(),
      };
    }
    if (exitCcMismatch(g.cc, exitCc)) {
      return {
        state: "ok",
        resolverIp: ip,
        via: `本软件内核 · ${id.label}`,
        answerNote: null,
        verdict: "y",
        msg: `DNS 出口国家（${g.cc}）与代理出口国家（${exitCc}）不一致`,
        at: Date.now(),
      };
    }
    return {
      state: "ok",
      resolverIp: ip,
      via: `本软件内核 · ${id.label}`,
      answerNote: null,
      verdict: "g",
      msg: `本次解析器探测为境外出口（${ip} · ${g.cc}）；不代表所有 DNS 请求均无绕行`,
      at: Date.now(),
    };
  }

  return {
    state: "fail",
    resolverIp: null,
    via: null,
    answerNote: filteredAddress ? "已过滤 Fake-IP、内网或无效地址" : null,
    verdict: "n",
    msg: filteredAddress
      ? "DNS 返回 Fake-IP、内网或无效地址，TXT 回退未取得公网解析器；请复核解析路径"
      : "无法完成内核 DNS 解析器探测（接口不可用或身份记录未返回解析器）",
    at: Date.now(),
  };
}

function exitCcMismatch(
  dnsCc: string,
  exitCc: string | null | undefined,
): boolean {
  return !!(exitCc && exitCc !== "CN" && dnsCc && dnsCc !== exitCc);
}
