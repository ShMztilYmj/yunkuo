import type { AiUsageItem } from "../../../../services/api";
import { type AiToolMeta } from "../../services/aiToolCards";
import type {
  ToolConnectivity,
  ToolInspection,
  ToolSetup,
} from "../../services/toolSetup";
import ToolSetupPanel from "../ToolSetupPanel";
import UsagePanel from "./UsagePanel";

/** 详情面：连接方案面板 + 用量面板。 */
export default function ToolCardDetails({
  meta,
  maker,
  showDetails,
  showAccounts,
  showResume,
  entry,
  inspection,
  connectionTest,
  setupBusy,
  setupError,
  item,
  usageLoading,
  usageError,
  usageRefreshing,
  onInspect,
  onTest,
  onUseShortcut,
  onRefreshUsage,
}: {
  meta: AiToolMeta;
  maker: string;
  showDetails: boolean;
  showAccounts: boolean;
  showResume: boolean;
  entry?: ToolSetup;
  inspection?: ToolInspection;
  connectionTest?: ToolConnectivity;
  setupBusy?: string;
  setupError: string;
  item?: AiUsageItem;
  usageLoading: boolean;
  usageError: string | null;
  usageRefreshing: boolean;
  onInspect: () => void;
  onTest: () => void;
  onUseShortcut: (path: string) => void;
  onRefreshUsage: () => void;
}) {
  return (
    <div
      className="tool-card-details"
      id={`tool-card-details-${meta.key}`}
      hidden={!showDetails || showAccounts || showResume}
      role="region"
      aria-label={`${meta.label}账号与用量`}
      tabIndex={showDetails && !showAccounts && !showResume ? 0 : -1}
    >
      <div className="tool-card-detail-heading">
        <span>{maker}</span>
        <h2>{meta.label}</h2>
        <p>账号、用量与额度详情</p>
      </div>
      <ToolSetupPanel
        entry={entry}
        inspection={inspection}
        connectionTest={connectionTest}
        busy={setupBusy}
        error={setupError}
        onInspect={onInspect}
        onTest={onTest}
        onUse={onUseShortcut}
      />
      <UsagePanel
        toolKey={meta.key}
        item={item}
        loading={usageLoading}
        error={usageError}
        refreshing={usageRefreshing}
        onRefresh={onRefreshUsage}
      />
    </div>
  );
}
