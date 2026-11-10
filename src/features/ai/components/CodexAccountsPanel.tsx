import { invalidateAiUsage, readAiUsage } from "../services/aiUsage";
import { useCallback, useState } from "react";
import { KeyRound, RefreshCw, Save } from "lucide-react";
import {
  api,
  type AiAccountEntry,
  type AiAccountInfo,
} from "../../../services/api";
import { usePolling } from "../../../shared/hooks/usePolling";
import { useConfirm } from "../../../shared/ui/useConfirm";
import {
  AccountCard,
  AccountMessage,
  accountButton,
  useAccountOperation,
} from "./AccountUi";

export default function CodexAccountsPanel({
  onChanged,
  onBusyChange,
}: {
  onChanged: () => void;
  onBusyChange: (busy: boolean) => void;
}) {
  const [accounts, setAccounts] = useState<AiAccountEntry[]>([]);
  const [current, setCurrent] = useState<AiAccountInfo | null>(null);
  const [ready, setReady] = useState(false);
  const [errors, setErrors] = useState<string[]>([]);
  const op = useAccountOperation(onBusyChange);
  const { confirm, dialog } = useConfirm();
  const load = useCallback(
    async (isCurrent: () => boolean = () => true, force = false) => {
      const [snapshots, usage] = await Promise.allSettled([
        api.aiListAccounts("chatgpt"),
        readAiUsage(force),
      ]);
      if (!isCurrent()) return;
      setAccounts(snapshots.status === "fulfilled" ? snapshots.value : []);
      setCurrent(
        usage.status === "fulfilled"
          ? (usage.value.items.find((item) => item.key === "chatgpt")
              ?.account ?? null)
          : null,
      );
      setReady(snapshots.status === "fulfilled");
      setErrors([
        ...(snapshots.status === "rejected"
          ? [`账号快照读取失败：${String(snapshots.reason)}`]
          : []),
        ...(usage.status === "rejected"
          ? [`当前账号读取失败：${String(usage.reason)}`]
          : []),
      ]);
    },
    [],
  );
  usePolling(load, 60_000, true, !op.busy);
  const act = (action: "save" | "switch" | "migrate", which?: string) =>
    op.run(async () => {
      if (
        action === "migrate" &&
        !(await confirm({
          tone: "warning",
          icon: KeyRound,
          eyebrow: "Codex 账号 · 加密迁移",
          title: "加密迁移旧版快照？",
          description:
            "将逐个验证并加密旧版 Codex 快照，读回成功后删除对应的旧明文。",
          note: {
            tone: "neutral",
            text: "加密副本只能由当前 Windows 用户解密；迁移过程中请勿退出应用。",
          },
          confirmLabel: "开始迁移",
        }))
      )
        return;
      const result =
        action === "save"
          ? await api.aiSnapshotAccount("chatgpt")
          : action === "migrate"
            ? await api.aiMigrateAccounts()
            : await api.aiSwitchAccount("chatgpt", which!);
      if (!result.ok) throw Error(result.message);
      invalidateAiUsage();
      await load(undefined, true);
      onChanged();
      op.setMessage(result.message);
    });

  return (
    <AccountCard title="Codex 本地账号">
      <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl bg-zinc-950 p-3">
        <div className="min-w-0">
          <p className="text-[11px] text-zinc-500">当前本地登录</p>
          <p className="mt-1 break-all text-xs font-medium">
            {current?.email ?? "未读取到可显示的账号"}
          </p>
          {current?.planLabel && (
            <p className="mt-1 text-[11px] text-zinc-400">
              {current.planLabel}
            </p>
          )}
        </div>
        <button
          className={accountButton}
          disabled={!ready || op.busy}
          onClick={() => void act("save")}
        >
          <Save size={13} />
          保存当前
        </button>
      </div>
      <details className="account-help text-[11px] leading-6 text-zinc-500">
        <summary>账号切换说明</summary>
        <p>
          保存多个账号快照后，可在这里切换 Codex 本地登录。切换前请关闭 Codex
          窗口和 CLI，切换后重新启动。快照由当前 Windows 用户加密；ChatGPT
          桌面账号需在官方应用中切换。
        </p>
      </details>
      <div className="space-y-2">
        <div className="flex items-center justify-between gap-2">
          <h3 className="text-xs font-medium">已保存账号</h3>
          <button
            className={accountButton}
            disabled={op.busy}
            onClick={() => void op.run(() => load(undefined, true))}
          >
            <RefreshCw size={12} />
            刷新
          </button>
        </div>
        {accounts.some((account) => account.legacy) && (
          <button
            className={accountButton}
            disabled={op.busy}
            onClick={() => void act("migrate")}
          >
            加密迁移旧快照
          </button>
        )}
      </div>
      <div className="space-y-2">
        {accounts.map((account) => (
          <div
            key={account.which}
            className="min-w-0 rounded-xl border border-zinc-800 p-3"
          >
            <p className="break-all text-xs font-medium">{account.label}</p>
            <p className="mt-1 text-[11px] text-zinc-500">
              {account.legacy
                ? "旧版快照，请先加密迁移"
                : "已加密的本地账号快照"}
            </p>
            <button
              className={accountButton + " mt-2"}
              disabled={op.busy || account.legacy}
              aria-label={`切换到账号 ${account.label}`}
              onClick={() => void act("switch", account.which)}
            >
              切换到此账号
            </button>
          </div>
        ))}
      </div>
      {ready && accounts.length === 0 && (
        <p className="rounded-xl border border-dashed border-zinc-800 p-3 text-center text-xs text-zinc-500">
          暂无账号快照，点击“保存当前”保存本机已登录的 Codex 账号。
        </p>
      )}
      {errors.map((error) => (
        <p role="alert" key={error} className="text-xs text-amber-400">
          {error}
        </p>
      ))}
      <AccountMessage text={op.message} />
      {dialog}
    </AccountCard>
  );
}
