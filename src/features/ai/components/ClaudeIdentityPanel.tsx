import { useCallback, useEffect, useState } from "react";
import {
  Archive,
  Loader2,
  RefreshCw,
  Trash2,
  Undo2,
  UserX,
  Users,
} from "lucide-react";
import {
  identityEnvironment as environment,
  type IdentityStatus,
  type Retention,
} from "../services/identities";
import { notifyEnvironmentInvalidated } from "../../../shared/events/environment";
import { api } from "../../../services/api";
import { useConfirm } from "../../../shared/ui/useConfirm";
import {
  AccountCard as Box,
  AccountMessage as Message,
  useAccountOperation as useOperation,
  accountButton as button,
} from "./AccountUi";

const primary = button + " btn-primary";
const input =
  "w-full rounded-lg border border-zinc-700 bg-zinc-950 px-3 py-2 text-sm text-zinc-200";

const EMPTY_POLICY: Retention = {
  history: false,
  cache: false,
  settings: false,
  skills: false,
  plugins: false,
  mcp: false,
  projects: false,
};
const labels: Record<keyof Retention, string> = {
  history: "会话历史",
  cache: "缓存与日志",
  settings: "个人设置",
  skills: "Skills / 命令",
  plugins: "插件",
  mcp: "MCP 配置",
  projects: "项目记录",
};
export default function ClaudeIdentityPanel({
  compact = false,
  onChanged,
  onBusyChange,
}: {
  compact?: boolean;
  onChanged?: () => void;
  onBusyChange?: (busy: boolean) => void;
}) {
  const [state, setState] = useState<IdentityStatus | null>(null);
  const [name, setName] = useState("");
  const [policy, setPolicy] = useState(EMPTY_POLICY);
  const op = useOperation(onBusyChange);
  const { confirm, dialog } = useConfirm();
  const load = useCallback(
    async () => setState(await environment.identities()),
    [],
  );
  useEffect(() => {
    load().catch((e) => op.setMessage(String(e)));
  }, [load]);
  const act = (action: string, id: string | null = null, target = "") =>
    op.run(async () => {
      const scopes =
        Object.entries(policy)
          .filter(([, v]) => v)
          .map(([k]) => labels[k as keyof Retention])
          .join("、") || "仅登录和本地身份资料";
      const saved = state?.catalog.records.find(
        (r) => r.id === (action === "recover" ? state.catalog.recovery : id),
      );
      const restoredScopes = saved
        ? [
            "登录和本地身份资料",
            ...Object.entries(saved.policy)
              .filter(([, v]) => v)
              .map(([k]) => labels[k as keyof Retention]),
          ].join("、")
        : scopes;
      if (
        !(await confirm(
          action === "delete"
            ? {
                tone: "danger",
                icon: Trash2,
                eyebrow: "Claude 身份 · 删除备份",
                title: `删除备份「${target}」？`,
                description:
                  "永久删除这份本地备份，不会新增备份；对应的浏览器资料保留。",
                note: {
                  tone: "warning",
                  text: "删除后如果需要恢复，只能重新备份当前身份。",
                },
                confirmLabel: "删除备份",
              }
            : action === "archive"
              ? {
                  tone: "neutral",
                  icon: Archive,
                  eyebrow: "Claude 身份 · 备份",
                  title: "备份当前身份？",
                  description: `备份范围：${scopes}；当前登录保持不变。`,
                  note: {
                    tone: "neutral",
                    text: "归档采用 Zstandard + AES-256-GCM，主密钥保存在当前用户专属权限的本地保险库。",
                  },
                  confirmLabel: "开始备份",
                }
              : {
                  tone: "warning",
                  icon:
                    action === "recover"
                      ? Undo2
                      : action === "empty"
                        ? UserX
                        : Users,
                  eyebrow: "Claude 身份 · 切换",
                  title:
                    action === "recover"
                      ? "恢复未完成的切换？"
                      : action === "empty"
                        ? "切换为空身份？"
                        : `切换到「${target}」？`,
                  description: `本次恢复 / 移开的范围：${restoredScopes}。会先加密保存本次涉及类别的当前资料，再修改工作区。`,
                  note: {
                    tone: "warning",
                    text: "请先退出 Claude Code、Claude Desktop 和指纹浏览器；服务端已失效的登录仍需重新认证。",
                  },
                  subject:
                    action === "switch" && target
                      ? { label: target, tag: "身份备份", tagTone: "blue" }
                      : null,
                  confirmLabel: action === "recover" ? "恢复" : "切换",
                },
        ))
      )
        return;
      notifyEnvironmentInvalidated();
      try {
        const result = await environment.identityAction(
          action,
          id,
          name || "本地身份备份",
          policy,
        );
        await api.recordWebScan(null);
        onChanged?.();
        op.setMessage(result.message);
      } finally {
        await load();
      }
    });
  return (
    <div className="space-y-5">
      <Box title="Claude 资料备份与切换">
        <p className="text-xs leading-6 text-zinc-400">
          当前本地账号：{state?.currentEmail ?? "未读取到可显示的账号"}。
          {!compact &&
            "仅管理默认 Claude Code / Desktop 数据目录。归档采用 Zstandard + AES-256-GCM；主密钥保存在当前用户专属权限的本地保险库中。"}
        </p>
        {state?.customDirectory && (
          <p className="text-xs text-amber-400">
            检测到自定义 CLAUDE_CONFIG_DIR，已阻止身份修改。
          </p>
        )}
        <label className="block space-y-2 text-xs text-zinc-400">
          备份名称
          <input
            className={input}
            value={name}
            onChange={(e) => setName(e.target.value)}
            maxLength={80}
            placeholder="例如：工作账号"
          />
        </label>
        <details className="account-help" open={compact ? undefined : true}>
          <summary className="text-xs text-zinc-400">备份范围与说明</summary>
          <p className="mb-3 text-xs text-zinc-400">
            备份或切换为空身份时，勾选资料随身份移动，未勾选的留在工作区。切换到已保存身份时，恢复该备份包含的类别，并自动备份涉及的当前资料。登录资料始终随身份移动。
          </p>
          <div className="flex flex-wrap gap-x-5 gap-y-3">
            {Object.entries(labels).map(([k, label]) => (
              <label key={k} className="flex items-center gap-2 text-xs">
                <input
                  type="checkbox"
                  checked={policy[k as keyof Retention]}
                  onChange={(e) =>
                    setPolicy({ ...policy, [k]: e.target.checked })
                  }
                />
                {label}
              </label>
            ))}
          </div>
          {compact && (
            <p className="mt-3 text-xs leading-6 text-zinc-500">
              管理默认 Claude Code / Desktop 数据目录。归档采用 Zstandard +
              AES-256-GCM；主密钥保存在当前用户专属权限的本地保险库中。
            </p>
          )}
        </details>
        <div className="flex flex-wrap gap-2">
          <button
            className={primary}
            disabled={!state || op.busy || state.customDirectory}
            onClick={() => act("archive")}
          >
            备份当前身份
          </button>
          <button
            className={button}
            disabled={
              !state ||
              op.busy ||
              state.customDirectory ||
              !!state.catalog.recovery
            }
            onClick={() => act("empty")}
          >
            备份并切换为空身份
          </button>
          {state?.catalog.recovery && (
            <button
              className={`${button} text-amber-300`}
              disabled={op.busy}
              onClick={() => act("recover")}
            >
              恢复未完成的切换
            </button>
          )}
          <button
            className={button}
            disabled={op.busy}
            aria-label="刷新 Claude 资料"
            onClick={() => op.run(load)}
          >
            <RefreshCw size={13} />
          </button>
        </div>
      </Box>
      <div className="grid gap-4 sm:grid-cols-2">
        {state?.catalog.records.map((r) => (
          <Box key={r.id} title={r.name || "未命名备份"}>
            <p className="text-xs text-zinc-400">{r.email ?? "未知本地账号"}</p>
            <p className="text-xs text-zinc-500">
              {new Date(r.createdAt).toLocaleString("zh-CN")} ·{" "}
              {(r.bytes / 1024 / 1024).toFixed(2)} MB
              {state.catalog.active === r.id ? " · 当前选择" : ""}
            </p>
            <p className="text-xs text-zinc-500">
              资料包含：登录
              {Object.entries(r.policy)
                .filter(([, v]) => v)
                .map(([k]) => `、${labels[k as keyof Retention]}`)
                .join("")}
            </p>
            <div className="flex gap-2">
              <button
                className={button}
                disabled={
                  op.busy || !!state.catalog.recovery || state.customDirectory
                }
                onClick={() => act("switch", r.id, r.name)}
              >
                切换到此身份
              </button>
              <button
                className={`${button} text-red-300`}
                disabled={
                  op.busy ||
                  state.catalog.active === r.id ||
                  state.catalog.recovery === r.id
                }
                onClick={() => act("delete", r.id, r.name)}
              >
                删除备份
              </button>
            </div>
          </Box>
        ))}
      </div>
      {state?.catalog.records.length === 0 && (
        <p className="text-center text-xs text-zinc-500">
          暂无备份。首次备份会绑定当前浏览器资料，后续空身份使用新的独立资料目录。
        </p>
      )}
      <p className="text-xs leading-6 text-zinc-500">
        实际项目源码不在扫描范围内。浏览器 Cookie
        与密码留在各身份独立资料中，不进入身份归档；首次网页登录需自行完成。本地同用户或管理员的访问能力不受加密格式绝对隔离。
      </p>
      {op.busy && (
        <p className="flex items-center gap-2 text-xs text-orange-300">
          <Loader2 size={14} className="animate-spin" />
          正在处理和验证身份数据，请勿退出程序…
        </p>
      )}
      <Message text={op.message} />
      {dialog}
    </div>
  );
}
