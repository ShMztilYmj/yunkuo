import {
  ArrowLeftRight,
  FileText,
  FolderDown,
  Gauge,
  Share2,
  ShieldCheck,
} from "lucide-react";

export const PROXY_SUB = [
  { to: "/overview", label: "概览", icon: Gauge },
  { to: "/proxies", label: "节点", icon: Share2 },
  { to: "/profiles", label: "订阅", icon: FolderDown },
  { to: "/connections", label: "连接", icon: ArrowLeftRight },
  { to: "/logs", label: "日志", icon: FileText },
  { to: "/rules", label: "规则", icon: ShieldCheck },
];

export const PROXY_ROUTES = PROXY_SUB.map((item) => item.to);
