import type { AiUsageItem } from "../../../../services/api";
import type { AgentBotState } from "../../services/agentBots";
import type { AiToolMeta } from "../../services/aiToolCards";
import type {
  ToolConnectivity,
  ToolInspection,
  ToolSetup,
} from "../../services/toolSetup";

/** 卡片展示态：全部由页面一次算出，子组件只读。 */
export interface ToolCardView {
  shortcut: string;
  configured: boolean;
  canLaunch: boolean;
  busy: boolean;
  accountWorking: boolean;
  supportsAccounts: boolean;
  showAccounts: boolean;
  showResume: boolean;
  showDetails: boolean;
}

/** 卡片引用的页面数据。 */
export interface ToolCardData {
  entry?: ToolSetup;
  item?: AiUsageItem;
  launchState?: AgentBotState;
  inspection?: ToolInspection;
  connectionTest?: ToolConnectivity;
  setupBusy?: string;
  setupError: string;
  setupFailure: string;
  message?: string | null;
  resumeMounted: boolean;
  accountPanelOpened: boolean;
  usageLoading: boolean;
  usageError: string | null;
  usageRefreshing: boolean;
  scanned: boolean;
  pass: boolean;
  launching: boolean;
  cardBusy: boolean;
  configReady: boolean;
  isDefaultCard: boolean;
}

/** 卡片交互回调，全部转发到页面状态。 */
export interface ToolCardActions {
  onToggleResume: () => void;
  onOpenAccounts: () => void;
  onToggleDetails: () => void;
  onRemove: () => void;
  onInspect: () => void;
  onTest: () => void;
  onUseShortcut: (path: string) => void;
  onPickShortcut: () => void;
  onAutoDetectShortcut: () => void;
  onRefreshUsage: () => void;
  onLaunch: () => void;
  onAccountBusyChange: (busy: boolean) => void;
  onResumeBusyChange: (busy: boolean) => void;
  onCodexAccountsChanged: () => void;
  onClaudeIdentityChanged: () => void;
}

export interface ToolCardProps {
  meta: AiToolMeta;
  maker: string;
  view: ToolCardView;
  data: ToolCardData;
  actions: ToolCardActions;
}
