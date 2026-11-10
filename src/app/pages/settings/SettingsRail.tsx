import { ArrowUpRight } from "lucide-react";
import { Link } from "react-router-dom";
import { useAppStore } from "../../../store";
import { SETTINGS_GROUPS } from "./sections";
import type { SettingsTab } from "./types";

/** 分类栏：左侧常驻导航，逐项标出该分类的当前状态。 */
export default function SettingsRail({
  tab,
  onChange,
  appVersion,
}: {
  tab: SettingsTab;
  onChange: (tab: SettingsTab) => void;
  appVersion: string;
}) {
  const core = useAppStore((s) => s.coreStatus);
  const config = useAppStore((s) => s.config);

  const stateOf = (
    key: SettingsTab,
  ): { tone: "on" | "off" | "warn"; label: string } | null => {
    switch (key) {
      case "proxy":
        return core == null
          ? { tone: "warn", label: "内核状态未知" }
          : core.running
            ? { tone: "on", label: "内核运行中" }
            : { tone: "off", label: "内核已停止" };
      case "timezone":
        return config?.tz_auto_sync
          ? { tone: "on", label: "自动同步已开启" }
          : { tone: "off", label: "自动同步已关闭" };
      case "detect":
        return config?.require_claude_reachable
          ? { tone: "on", label: "工具服务可达校验已开启" }
          : { tone: "off", label: "仅按环境评分放行" };
      case "ai":
        return config?.ai_usage_scan
          ? { tone: "on", label: "本地用量扫描已开启" }
          : { tone: "off", label: "本地用量扫描已关闭" };
      default:
        return null;
    }
  };

  return (
    <nav className="settings-rail" aria-label="设置分类">
      {SETTINGS_GROUPS.map((group) => (
        <div className="settings-rail-group" key={group.label}>
          <span className="settings-rail-label">{group.label}</span>
          {group.items.map(({ key, label, hint, icon: Icon }) => {
            const state = stateOf(key);
            const active = tab === key;
            return (
              <button
                key={key}
                type="button"
                onClick={() => onChange(key)}
                aria-current={active ? "page" : undefined}
                className="settings-rail-item"
              >
                <span className="settings-rail-icon" aria-hidden="true">
                  <Icon />
                </span>
                <span className="settings-rail-copy">
                  <span className="settings-rail-title">{label}</span>
                  <span className="settings-rail-hint">{hint}</span>
                </span>
                {state && (
                  <>
                    <span
                      className="settings-rail-state"
                      data-tone={state.tone}
                      title={state.label}
                      aria-hidden="true"
                    />
                    <span className="sr-only">{state.label}</span>
                  </>
                )}
              </button>
            );
          })}
        </div>
      ))}
      <div className="settings-rail-foot">
        <span>云阔 v{appVersion}</span>
        <Link className="settings-rail-link" to="/maintenance">
          首次使用与维护
          <ArrowUpRight aria-hidden="true" />
        </Link>
      </div>
    </nav>
  );
}
