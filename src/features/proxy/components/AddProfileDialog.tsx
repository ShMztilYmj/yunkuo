import { useId, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useNavigate } from "react-router-dom";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { FolderOpen, Link2, Loader2, PlugZap, Plus, X } from "lucide-react";
import { api } from "../../../services/api";
import { useProxyStore } from "../state";
import { useAddProfile } from "../addProfile";

type Mode = "url" | "node" | "local";

const TABS: { id: Mode; label: string }[] = [
  { id: "url", label: "订阅链接" },
  { id: "node", label: "节点串" },
  { id: "local", label: "本地文件" },
];

const URL_PATTERN = /^https?:\/\//i;

/**
 * 添加订阅弹窗：从订阅页右下角的圆形「+」打开，也承接空态按钮。
 *
 * 只负责「生成配置」这一步；生成成功后刷新当前配置与内核状态、通知订阅页重载，
 * 再统一落到订阅页，用户能立刻看到新增的卡片。
 */
export default function AddProfileDialog() {
  const hide = useAddProfile((state) => state.hide);
  const committed = useAddProfile((state) => state.committed);
  const nav = useNavigate();

  const dialog = useRef<HTMLDialogElement>(null);
  const urlInput = useRef<HTMLInputElement>(null);
  const titleId = useId();
  const formId = useId();

  const [mode, setMode] = useState<Mode>("url");
  const [url, setUrl] = useState("");
  const [name, setName] = useState("");
  const [protocol, setProtocol] = useState<"socks5" | "http">("socks5");
  const [nodeHost, setNodeHost] = useState("");
  const [nodePort, setNodePort] = useState("");
  const [nodeAccount, setNodeAccount] = useState("");
  const [nodePassword, setNodePassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useLayoutEffect(() => {
    const node = dialog.current;
    if (!node) return;
    if (!node.open) node.showModal();
    urlInput.current?.focus({ preventScroll: true });
    return () => {
      if (node.open) node.close();
    };
  }, []);

  const dismiss = () => {
    if (!busy) hide();
  };

  /** 导入成功后统一收敛：刷新共享状态 → 通知订阅页 → 关闭弹窗并落到订阅页。 */
  const run = async (task: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await task();
      const store = useProxyStore.getState();
      await Promise.allSettled([
        store.refreshProfile(),
        store.refreshCore(true),
      ]);
      committed();
      hide();
      nav("/profiles");
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };

  // ---- 节点串：把分开填写的字段还原成后端支持的节点串 ----
  const nodeHostValue = nodeHost.trim().replace(/^\[(.*)\]$/, "$1");
  const nodePortValue = nodePort.trim();
  const nodePortNumber = Number(nodePortValue);
  const nodeAccountValue = nodeAccount.trim();
  const nodePasswordValue = nodePassword.trim();
  const nodeHostInlinePort =
    /^[^:[\]]+:\d+$/.test(nodeHostValue) ||
    /^\[[^\]]+\]:\d+$/.test(nodeHostValue);
  const nodeHostIsIpv6 =
    nodeHostValue.includes(":") && /^[0-9a-fA-F:.]+$/.test(nodeHostValue);
  // URL 形式（IPv6）多一层 URL 保留字符限制；普通 host 用冒号串，仅需避开分隔符。
  const nodeUsesUrlForm = nodeHostValue.includes(":");
  const nodeCredentialInvalid = nodeUsesUrlForm
    ? /[\s@:/?#%]/.test(nodeAccountValue) ||
      /[\s@:/?#%]/.test(nodePasswordValue)
    : /[\s@:]/.test(nodeAccountValue) || /[\s@:]/.test(nodePasswordValue);
  const hasNodeInput = Boolean(
    nodeHost.trim() || nodePort.trim() || nodeAccount.trim() || nodePassword,
  );
  const nodeProblem = !nodeHostValue
    ? "请填写服务器地址"
    : nodeHostValue.includes("://") ||
        /[\s/?#@]/.test(nodeHostValue) ||
        nodeHostInlinePort ||
        (nodeHostValue.includes(":") && !nodeHostIsIpv6)
      ? "服务器地址只填域名或 IP，端口请填在右侧输入框"
      : !nodePortValue
        ? "请填写端口"
        : !/^\d+$/.test(nodePortValue) ||
            nodePortNumber < 1 ||
            nodePortNumber > 65535
          ? "端口需为 1–65535 的数字"
          : nodePasswordValue && !nodeAccountValue
            ? "填写密码时请同时填写账号"
            : nodeCredentialInvalid
              ? nodeUsesUrlForm
                ? "IPv6 节点的账号/密码不能包含空格、@、:、/、?、# 或 %"
                : "账号/密码不能包含空格、@ 或冒号"
              : null;
  const nodeReady = Boolean(
    nodeHostValue && nodePortValue && nodeProblem == null,
  );

  const buildNodeSpec = () => {
    const server = nodeHostValue.includes(":")
      ? `[${nodeHostValue}]`
      : nodeHostValue;
    if (!nodeAccountValue) return `${server}:${nodePortNumber}`;
    if (nodeHostValue.includes(":"))
      return `${protocol}://${nodeAccountValue}${
        nodePasswordValue ? `:${nodePasswordValue}` : ""
      }@${server}:${nodePortNumber}`;
    return `${nodeHostValue}:${nodePortNumber}:${nodeAccountValue}${
      nodePasswordValue ? `:${nodePasswordValue}` : ""
    }`;
  };

  const ready =
    mode === "local" ||
    (mode === "url" ? Boolean(url.trim()) : mode === "node" && nodeReady);

  const pickLocal = async () => {
    if (busy) return;
    setError(null);
    try {
      const path = await openFileDialog({
        multiple: false,
        filters: [{ name: "Clash 配置", extensions: ["yaml", "yml"] }],
      });
      if (typeof path !== "string") return;
      await run(() => api.createProfileLocal(path, undefined));
    } catch (reason) {
      setError(String(reason));
    }
  };

  const submit = () => {
    if (busy) return;
    if (mode === "local") {
      void pickLocal();
      return;
    }
    if (mode === "url") {
      const value = url.trim();
      if (!URL_PATTERN.test(value)) {
        setError("订阅链接需以 http:// 或 https:// 开头");
        return;
      }
      void run(() =>
        api.createProfileRedirect(value, name.trim() || undefined),
      );
      return;
    }
    if (nodeProblem) {
      setError(nodeProblem);
      return;
    }
    void run(() =>
      api.createProfileNode(
        buildNodeSpec(),
        protocol,
        name.trim() || undefined,
      ),
    );
  };

  const action =
    mode === "url"
      ? { label: "导入订阅", icon: Link2 }
      : mode === "node"
        ? { label: "导入节点", icon: PlugZap }
        : { label: "选择文件", icon: FolderOpen };
  const ActionIcon = action.icon;

  return createPortal(
    <dialog
      ref={dialog}
      className="confirm-dialog is-neutral profile-add-dialog"
      aria-labelledby={titleId}
      onCancel={(event) => {
        event.preventDefault();
        dismiss();
      }}
      onClick={(event) => {
        if (event.target === event.currentTarget) dismiss();
      }}
    >
      <div className="confirm-card" role="document">
        <header className="confirm-head">
          <span className="confirm-icon" aria-hidden="true">
            <Plus size={16} />
          </span>
          <div className="confirm-heading">
            <span className="confirm-eyebrow">订阅管理</span>
            <h2 id={titleId}>添加订阅</h2>
          </div>
          <button
            type="button"
            className="confirm-close"
            aria-label="关闭"
            disabled={busy}
            onClick={dismiss}
          >
            <X size={14} />
          </button>
        </header>

        <p className="confirm-desc">
          订阅链接、单节点或本地 YAML，导入后统一出现在订阅卡片里。
        </p>

        <div className="profile-add-tabs" role="tablist" aria-label="导入方式">
          {TABS.map((tab) => (
            <button
              key={tab.id}
              type="button"
              role="tab"
              aria-selected={mode === tab.id}
              data-active={mode === tab.id}
              className="profile-add-tab"
              onClick={() => {
                setMode(tab.id);
                setError(null);
              }}
            >
              {tab.label}
            </button>
          ))}
        </div>

        <form
          id={formId}
          className="profile-add-form"
          onSubmit={(event) => {
            event.preventDefault();
            submit();
          }}
        >
          {mode === "url" && (
            <>
              <input
                ref={urlInput}
                value={url}
                onChange={(event) => setUrl(event.target.value)}
                placeholder="粘贴 Clash 订阅链接 https://..."
                autoComplete="off"
                spellCheck={false}
                aria-label="订阅链接"
              />
              <input
                value={name}
                onChange={(event) => setName(event.target.value)}
                placeholder="备注名（可选，默认使用服务商名称）"
                aria-label="备注名"
              />
              <p className="profile-add-hint">
                支持 Clash / Mihomo 订阅。导入后可在卡片上设置自动更新间隔。
              </p>
            </>
          )}

          {mode === "node" && (
            <>
              <div className="profile-add-row">
                <select
                  className="profile-add-protocol"
                  aria-label="代理协议"
                  value={protocol}
                  onChange={(event) =>
                    setProtocol(event.target.value as "socks5" | "http")
                  }
                >
                  <option value="socks5">Socks5</option>
                  <option value="http">HTTP</option>
                </select>
                <input
                  className="profile-add-grow"
                  aria-label="服务器地址"
                  autoComplete="off"
                  spellCheck={false}
                  placeholder="服务器地址（域名 / IPv4 / IPv6）"
                  value={nodeHost}
                  onChange={(event) => setNodeHost(event.target.value)}
                />
                <input
                  className="profile-add-port"
                  aria-label="端口"
                  inputMode="numeric"
                  maxLength={5}
                  placeholder="端口"
                  value={nodePort}
                  onChange={(event) => setNodePort(event.target.value)}
                />
              </div>
              <div className="profile-add-row">
                <input
                  className="profile-add-grow"
                  aria-label="账号"
                  autoComplete="off"
                  placeholder="账号（可选）"
                  value={nodeAccount}
                  onChange={(event) => setNodeAccount(event.target.value)}
                />
                <input
                  className="profile-add-grow"
                  aria-label="密码"
                  type="password"
                  autoComplete="off"
                  placeholder="密码（可选）"
                  value={nodePassword}
                  onChange={(event) => setNodePassword(event.target.value)}
                />
              </div>
              <input
                aria-label="备注名"
                placeholder="备注名（可选）"
                value={name}
                onChange={(event) => setName(event.target.value)}
              />
              {hasNodeInput && nodeProblem && (
                <p className="profile-add-hint is-warn" role="alert">
                  {nodeProblem}
                </p>
              )}
              <p className="profile-add-hint">
                会生成单节点配置（节点选择 → MATCH）；住宅/静态 IP
                供应商多数不支持 UDP，导入的节点已显式关闭 UDP。
              </p>
            </>
          )}

          {mode === "local" && (
            <div className="profile-add-stage">
              <FolderOpen size={17} aria-hidden="true" />
              <div>
                <strong>选择一个 Clash 配置文件</strong>
                <p>支持 .yaml / .yml。导入后保存为独立配置，不随订阅更新。</p>
              </div>
            </div>
          )}
        </form>

        {error && (
          <p className="confirm-error" role="alert">
            {error}
          </p>
        )}

        <footer className="confirm-actions">
          <span className="confirm-hint">
            <kbd>Esc</kbd> 取消
          </span>
          <div className="confirm-buttons">
            <button
              type="button"
              className="confirm-btn is-plain"
              disabled={busy}
              onClick={dismiss}
            >
              取消
            </button>
            <button
              type="submit"
              form={formId}
              className="confirm-btn is-solid"
              disabled={busy || !ready}
            >
              {busy ? (
                <Loader2 className="confirm-spin" size={13} />
              ) : (
                <ActionIcon size={13} />
              )}
              {busy ? "正在导入…" : action.label}
            </button>
          </div>
        </footer>
      </div>
    </dialog>,
    document.body,
  );
}
