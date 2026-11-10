import { useState } from "react";
import { api, type DefaultBrowserStatus } from "../../../services/api";
import { usePolling } from "../../../shared/hooks/usePolling";

/** 全局代理 + TUN 时使用受控指纹浏览器，退出或撤销时还原。 */
export default function DefaultBrowserRow() {
  const [status, setStatus] = useState<DefaultBrowserStatus | null>(null);
  usePolling(async (isCurrent) => {
    try {
      const next = await api.defaultBrowserStatus();
      if (isCurrent()) setStatus(next);
    } catch {
      if (isCurrent()) setStatus(null);
    }
  }, 5000);
  const name = (progid: string | null | undefined): string => {
    if (!progid) return "未知";
    if (progid === "Yunkuo.FingerprintBrowser")
      return "指纹 Chromium（受控入口）";
    if (
      progid === "ClaudeOC.FingerprintBrowser" ||
      progid === "FirefoxBuiltin.HTML"
    )
      return "旧版关联（迁移至受控入口）";
    if (progid.startsWith("FirefoxURL") || progid.startsWith("FirefoxHTML"))
      return "Firefox";
    if (
      progid.startsWith("MSEdge") ||
      progid.startsWith("AppXq0fcvsc2xg7m8mvrugwzrcrwc8cjm9me")
    )
      return "Microsoft Edge";
    if (progid.startsWith("ChromeHTML")) return "Google Chrome";
    return progid;
  };
  return (
    <div className="settings-browser">
      <div className="settings-browser-line">
        <span className="settings-browser-key">HTTP</span>
        <span>{name(status?.current)}</span>
      </div>
      <div className="settings-browser-line">
        <span className="settings-browser-key">HTTPS</span>
        <span>{name(status?.https)}</span>
      </div>
      <p className="settings-browser-note">
        {status?.switched
          ? "原默认关联已备份，退出或关闭联动后自动还原。"
          : "全局代理 + TUN 运行时联动至指纹 Chromium。"}
        外部链接打开前会检查系统代理、虚拟网卡与出站规则，无需浏览器实测。
      </p>
    </div>
  );
}
