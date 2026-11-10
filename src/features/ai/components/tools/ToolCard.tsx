import ToolCardAccounts from "./ToolCardAccounts";
import ToolCardDetails from "./ToolCardDetails";
import ToolCardFace from "./ToolCardFace";
import ToolCardFooter from "./ToolCardFooter";
import ToolCardResume from "./ToolCardResume";
import ToolCardShortcut from "./ToolCardShortcut";
import ToolCardTopline from "./ToolCardTopline";
import type { ToolCardProps } from "./types";

/** 单张工具卡：顶栏 + 卡面 / 详情 / 账号 / 续跑 + 启动入口 + 页脚。 */
export default function ToolCard({
  meta,
  maker,
  view,
  data,
  actions,
}: ToolCardProps) {
  return (
    <section
      aria-label={`${meta.label}工具卡片`}
      data-tool-card={meta.key}
      className={`tool-playing-card ${view.showDetails ? "is-detail" : ""}`}
    >
      <ToolCardTopline
        meta={meta}
        entry={data.entry}
        configured={view.configured}
        supportsAccounts={view.supportsAccounts}
        showResume={view.showResume}
        showAccounts={view.showAccounts}
        showDetails={view.showDetails}
        cardBusy={data.cardBusy}
        launching={data.launching}
        configReady={data.configReady}
        isDefaultCard={data.isDefaultCard}
        onToggleResume={actions.onToggleResume}
        onOpenAccounts={actions.onOpenAccounts}
        onToggleDetails={actions.onToggleDetails}
        onRemove={actions.onRemove}
      />

      <div className="tool-card-body">
        <ToolCardFace
          meta={meta}
          maker={maker}
          busy={view.busy}
          launchState={data.launchState}
          item={data.item}
          entry={data.entry}
          setupFailure={data.setupFailure}
          showDetails={view.showDetails}
          usageLoading={data.usageLoading}
          usageError={data.usageError}
        />
        <ToolCardDetails
          meta={meta}
          maker={maker}
          showDetails={view.showDetails}
          showAccounts={view.showAccounts}
          showResume={view.showResume}
          entry={data.entry}
          inspection={data.inspection}
          connectionTest={data.connectionTest}
          setupBusy={data.setupBusy}
          setupError={data.setupError}
          item={data.item}
          usageLoading={data.usageLoading}
          usageError={data.usageError}
          usageRefreshing={data.usageRefreshing}
          onInspect={actions.onInspect}
          onTest={actions.onTest}
          onUseShortcut={actions.onUseShortcut}
          onRefreshUsage={actions.onRefreshUsage}
        />
        {view.supportsAccounts &&
          (view.showAccounts || data.accountPanelOpened) && (
            <ToolCardAccounts
              meta={meta}
              showAccounts={view.showAccounts}
              onAccountBusyChange={actions.onAccountBusyChange}
              onCodexAccountsChanged={actions.onCodexAccountsChanged}
              onClaudeIdentityChanged={actions.onClaudeIdentityChanged}
            />
          )}
        {view.supportsAccounts && data.resumeMounted && (
          <ToolCardResume
            meta={meta}
            showResume={view.showResume}
            onResumeBusyChange={actions.onResumeBusyChange}
          />
        )}
      </div>

      <ToolCardShortcut
        meta={meta}
        shortcut={view.shortcut}
        configured={view.configured}
        inspectBusy={!!data.setupBusy}
        autoBusy={data.setupBusy === "autodetect"}
        onInspect={actions.onInspect}
        onAutoDetect={actions.onAutoDetectShortcut}
        onPick={actions.onPickShortcut}
      />
      <ToolCardFooter
        meta={meta}
        accountWorking={view.accountWorking}
        message={data.message}
        configured={view.configured}
        scanned={data.scanned}
        pass={data.pass}
        entry={data.entry}
        canLaunch={view.canLaunch}
        busy={view.busy}
        onLaunch={actions.onLaunch}
      />
    </section>
  );
}
