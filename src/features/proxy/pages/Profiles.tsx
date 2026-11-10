import { useCallback, useEffect, useRef, useState } from "react";
import {
  CheckCircle2,
  ChevronDown,
  Clock,
  FolderOpen,
  Link2,
  Loader2,
  Plus,
  RefreshCw,
  SlidersHorizontal,
  Sparkles,
  Trash2,
  TriangleAlert,
} from "lucide-react";
import { api, type ProfileItem } from "../../../services/api";
import { proxyApi, type ProxyOverlay } from "../services/api";
import { useAddProfile } from "../addProfile";
import { useProxyStore } from "../state";
import AddProfileDialog from "../components/AddProfileDialog";
import ProfileQuotaDialog from "../components/ProfileQuotaDialog";
import ProxyTabs from "../components/ProxyTabs";
import SubscriptionCard from "../components/SubscriptionCard";
import ConfirmDialog from "../../../shared/ui/ConfirmDialog";
import PageHeader from "../../../shared/ui/PageHeader";

const OVERLAY_TEMPLATE = `# 订阅覆写：只识别 rules、proxy-groups、dns，其余键会被忽略。
# 写在这里的内容叠加在订阅之上，更新订阅不会覆盖。
rules:
  - DOMAIN-SUFFIX,corp.example,DIRECT
proxy-groups:
  - name: 我的分组
    type: select
    proxies: [DIRECT]
dns:
  nameserver-policy:
    "geosite:cn": [223.5.5.5, 119.29.29.29]
`;

export default function Profiles() {
  const [items, setItems] = useState<ProfileItem[]>([]);
  const [current, setCurrent] = useState<ProfileItem | null>(null);
  const [busy, setBusy] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);
  const [pendingDelete, setPendingDelete] = useState<ProfileItem | null>(null);
  const [editing, setEditing] = useState<ProfileItem | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const [overlay, setOverlay] = useState<ProxyOverlay | null>(null);
  const [overlayText, setOverlayText] = useState("");
  const [overlayOpen, setOverlayOpen] = useState(false);
  const [overlayMsg, setOverlayMsg] = useState<string | null>(null);
  const [overlayBusy, setOverlayBusy] = useState(false);
  const refreshProfile = useProxyStore((s) => s.refreshProfile);
  const refreshCore = useProxyStore((s) => s.refreshCore);
  const coreStatus = useProxyStore((s) => s.coreStatus);
  const showAddProfile = useAddProfile((s) => s.show);
  const addProfileOpen = useAddProfile((s) => s.open);
  /** 右下角「+」导入成功后自增，用来重拉列表；同一路由不会重新挂载页面。 */
  const profileRevision = useAddProfile((s) => s.revision);
  /** 首次加载填充编辑器；之后的刷新不覆盖用户正在改的草稿。 */
  const overlayLoaded = useRef(false);
  const overlaySection = useRef<HTMLElement>(null);

  /**
   * 展开后把整块面板带进视野：它排在卡片下方，窗口矮时容易落在折叠线以外。
   * 直接滚到滚动容器底部，脚注与圆钮之间留出的空位不会被覆盖。
   */
  useEffect(() => {
    if (!overlayOpen) return;
    const node = overlaySection.current;
    const viewport = node?.closest(".proxy-data-content");
    if (!node || !viewport) return;
    const frame = requestAnimationFrame(() =>
      viewport.scrollTo({ top: viewport.scrollHeight, behavior: "smooth" }),
    );
    return () => cancelAnimationFrame(frame);
  }, [overlayOpen]);

  const load = useCallback(async () => {
    const [arr, cur, layer] = await Promise.all([
      api.listProfiles(),
      api.currentProfile(),
      proxyApi.overlay(),
    ]);
    setItems(arr);
    setCurrent(cur);
    setOverlay(layer);
    if (!overlayLoaded.current) {
      overlayLoaded.current = true;
      setOverlayText(layer.text);
    }
  }, []);

  useEffect(() => {
    void load().catch((error) => setMsg(String(error)));
  }, [load, profileRevision]);

  const refreshList = async () => {
    setRefreshing(true);
    try {
      await load();
    } catch (error) {
      setMsg(String(error));
    } finally {
      setRefreshing(false);
    }
  };

  /** 改名称 / 补额度只写本地记录：重拉列表与当前配置即可，不需要动内核。 */
  const reloadAfterEdit = async () => {
    try {
      await load();
      await refreshProfile();
    } catch (error) {
      setMsg(String(error));
    }
  };

  /** 卡片上就地改名：只提交名称字段，不涉及额度，也不打开任何弹窗。 */
  const renameProfile = async (profile: ProfileItem, name: string) => {
    await api.patchProfile(profile.uid, name, undefined);
    await reloadAfterEdit();
  };

  const act = async (fn: () => Promise<unknown>, ok: string) => {
    setBusy(true);
    setMsg(null);
    try {
      await fn();
      setMsg(ok);
    } catch (e) {
      setMsg(String(e));
    } finally {
      try {
        await load();
        await refreshProfile();
        await refreshCore(true);
      } catch (error) {
        setMsg(String(error));
      }
      setBusy(false);
    }
  };

  /** 删除走自己的一条路径：失败原因要留在确认框里，不能像 act 那样只写页面顶部。 */
  const removeProfile = async () => {
    const target = pendingDelete;
    if (!target || deleting) return;
    setDeleting(true);
    setDeleteError(null);
    try {
      await api.deleteProfile(target.uid);
      setPendingDelete(null);
      setMsg("已删除配置");
      try {
        await load();
        await refreshProfile();
        await refreshCore(true);
      } catch (error) {
        setMsg(String(error));
      }
    } catch (error) {
      setDeleteError(String(error));
    } finally {
      setDeleting(false);
    }
  };

  /** 覆写的保存与清空共用一条路径：成功才回填编辑器，失败原因留在卡片里。 */
  const changeOverlay = async (
    run: () => Promise<ProxyOverlay>,
    ok: string,
  ) => {
    setOverlayBusy(true);
    setOverlayMsg(null);
    try {
      const layer = await run();
      setOverlay(layer);
      setOverlayText(layer.text);
      setOverlayMsg(ok);
      await refreshCore(true);
    } catch (error) {
      setOverlayMsg(String(error));
    } finally {
      setOverlayBusy(false);
    }
  };

  const overlayActive = Boolean(
    overlay &&
    (overlay.rules > 0 ||
      overlay.groups.length > 0 ||
      overlay.dnsPolicy.length > 0),
  );
  const overlayEntries = overlay
    ? overlay.groups.length + overlay.dnsPolicy.length
    : 0;
  const overlayLines = overlayText.trim()
    ? overlayText.replace(/\s+$/, "").split("\n").length
    : 0;

  return (
    <div className="proxy-data-page profiles-page">
      <PageHeader
        icon={Link2}
        title="订阅管理"
        badge={`${items.length} 个配置`}
        meta={
          <>
            <span>
              {current ? (
                <>
                  当前 <strong>{current.name}</strong>
                </>
              ) : (
                "未选择配置"
              )}
            </span>
            <i className="proxy-meta-dot" />
            <span>订阅、本地配置与单节点统一管理</span>
            <i className="proxy-meta-dot" />
            <span>自动更新每 30 分钟检查</span>
            {coreStatus?.last_apply && (
              <>
                <i className="proxy-meta-dot" />
                <span title={coreStatus.last_apply.note}>
                  上次应用：{coreStatus.last_apply.note}
                </span>
              </>
            )}
          </>
        }
        actions={
          <button
            type="button"
            className="proxy-action"
            onClick={() => void refreshList()}
            disabled={refreshing}
          >
            <RefreshCw className={refreshing ? "proxy-spin" : ""} />
            刷新
          </button>
        }
        tabs={<ProxyTabs />}
      />

      <div className="proxy-data-content is-scroll">
        {msg && (
          <div className="proxy-form-msg" role="status">
            {msg}
          </div>
        )}

        {items.length === 0 ? (
          <section className="proxy-panel proxy-empty">
            <span className="proxy-empty-icon">
              <FolderOpen size={20} />
            </span>
            <strong>还没有订阅</strong>
            <p>
              添加 Clash 订阅、单节点串，或从本地 YAML
              导入；之后都能在卡片上切换与更新。
            </p>
            <button
              type="button"
              className="proxy-action proxy-primary-small"
              onClick={showAddProfile}
            >
              <Plus size={13} /> 添加订阅
            </button>
          </section>
        ) : (
          <div className="proxy-sub-grid">
            {items.map((profile) => (
              <SubscriptionCard
                key={profile.uid}
                profile={profile}
                active={current?.uid === profile.uid}
                busy={busy}
                onUse={() =>
                  act(
                    () => api.selectProfile(profile.uid),
                    "已切换配置并重载内核",
                  )
                }
                onUpdate={() =>
                  act(() => api.updateProfile(profile.uid), "订阅已更新")
                }
                onInterval={(hours) =>
                  act(
                    () => api.patchProfile(profile.uid, undefined, hours),
                    "已更新自动更新间隔",
                  )
                }
                onDelete={() => {
                  setDeleteError(null);
                  setPendingDelete(profile);
                }}
                onRename={(name) => renameProfile(profile, name)}
                onEdit={() => setEditing(profile)}
              />
            ))}
          </div>
        )}

        <section
          ref={overlaySection}
          className="proxy-panel proxy-overlay"
          aria-label="订阅覆写"
          data-open={overlayOpen}
        >
          <button
            type="button"
            className="proxy-overlay-toggle"
            aria-expanded={overlayOpen}
            aria-controls="proxy-overlay-body"
            onClick={() => setOverlayOpen((open) => !open)}
          >
            <span className="proxy-overlay-icon" aria-hidden="true">
              <SlidersHorizontal size={15} />
            </span>
            <span className="proxy-overlay-title">
              <span className="proxy-overlay-heading">
                <strong>订阅覆写</strong>
                <span className="proxy-overlay-level">高级</span>
              </span>
              <em>
                {overlayActive
                  ? "已叠加在订阅之上，更新订阅不会清除。"
                  : "给所有订阅追加规则、分组与 DNS 策略。"}
              </em>
            </span>
            <span className="proxy-overlay-summary" aria-hidden="true">
              <span>
                <b>{overlay?.rules ?? 0}</b> 规则
              </span>
              <span>
                <b>{overlay?.groups.length ?? 0}</b> 分组
              </span>
              <span>
                <b>{overlay?.dnsPolicy.length ?? 0}</b> DNS
              </span>
            </span>
            <span className="proxy-overlay-state" data-on={overlayActive}>
              {overlayActive ? "已启用" : "未启用"}
            </span>
            <ChevronDown
              className="proxy-overlay-chevron"
              size={15}
              aria-hidden="true"
            />
          </button>

          {overlayOpen && (
            <div className="proxy-overlay-body" id="proxy-overlay-body">
              <div className="proxy-overlay-guide">
                <p className="proxy-overlay-lead">
                  订阅每次更新都会整体替换配置。写在这里的内容会叠加在订阅之上，
                  <strong>更新后不会被覆盖</strong>。
                </p>
                <dl className="proxy-overlay-schema">
                  <div>
                    <dt>
                      <code>rules</code>
                    </dt>
                    <dd>前插规则，优先于订阅里的规则匹配</dd>
                  </div>
                  <div>
                    <dt>
                      <code>proxy-groups</code>
                    </dt>
                    <dd>追加自定义分组，供 rules 引用</dd>
                  </div>
                  <div>
                    <dt>
                      <code>dns</code>
                    </dt>
                    <dd>只补充 nameserver-policy 解析策略</dd>
                  </div>
                </dl>
                <p className="proxy-overlay-limit">
                  <TriangleAlert size={12} aria-hidden="true" />
                  <span>
                    只识别上面三类键，其余内容会被忽略；严格保护下不可用。
                  </span>
                </p>
                {overlayEntries > 0 && (
                  <div className="proxy-overlay-applied">
                    <span className="proxy-overlay-applied-label">已生效</span>
                    {overlay?.groups.map((group) => (
                      <span
                        key={`group-${group}`}
                        className="proxy-overlay-chip"
                      >
                        {group}
                      </span>
                    ))}
                    {overlay?.dnsPolicy.map((policy) => (
                      <span
                        key={`dns-${policy}`}
                        className="proxy-overlay-chip is-dns"
                      >
                        {policy}
                      </span>
                    ))}
                  </div>
                )}
              </div>

              <div className="proxy-overlay-editor">
                <div className="proxy-overlay-editor-head">
                  <span>覆盖层 YAML</span>
                  <span>{overlayLines} 行 · 保存后立即生效</span>
                </div>
                <textarea
                  aria-label="覆写 YAML"
                  rows={9}
                  spellCheck={false}
                  value={overlayText}
                  onChange={(event) => setOverlayText(event.target.value)}
                  placeholder={OVERLAY_TEMPLATE}
                />
                <div className="proxy-overlay-actions">
                  <button
                    type="button"
                    className="proxy-action proxy-primary-small"
                    disabled={overlayBusy}
                    onClick={() =>
                      void changeOverlay(
                        () => proxyApi.overlaySave(overlayText),
                        "覆写已保存并应用",
                      )
                    }
                  >
                    {overlayBusy ? (
                      <Loader2 className="proxy-spin" size={13} />
                    ) : (
                      <CheckCircle2 size={13} />
                    )}
                    保存并应用
                  </button>
                  <button
                    type="button"
                    className="proxy-action"
                    disabled={overlayBusy}
                    onClick={() => setOverlayText(OVERLAY_TEMPLATE)}
                  >
                    <Sparkles size={13} />
                    插入示例
                  </button>
                  <button
                    type="button"
                    className="proxy-action"
                    disabled={overlayBusy}
                    onClick={() =>
                      void changeOverlay(
                        () => proxyApi.overlayReset(),
                        "覆写已清空",
                      )
                    }
                  >
                    清空覆写
                  </button>
                  <span className="proxy-overlay-note">
                    优先原地热更新，不重启内核
                  </span>
                </div>
                {overlayMsg && (
                  <p className="proxy-form-msg" role="status">
                    {overlayMsg}
                  </p>
                )}
              </div>
            </div>
          )}
        </section>
      </div>

      {/* 页面右下角：圆形添加入口；固定在页脚，不压住卡片 */}
      <footer className="profiles-foot">
        <p className="profiles-foot-note">
          <Clock size={12} aria-hidden="true" />
          <span>
            应用运行期间每 30
            分钟检查到期订阅；严格保护启用时暂停自动更新，手动刷新后可能需要重新启用保护。
          </span>
        </p>
        <button
          type="button"
          className="profiles-add"
          aria-label="添加订阅"
          aria-haspopup="dialog"
          data-tooltip="添加订阅链接、节点串或本地 YAML"
          onClick={showAddProfile}
        >
          <Plus size={18} aria-hidden="true" />
        </button>
      </footer>

      {addProfileOpen && <AddProfileDialog />}

      {editing && (
        <ProfileQuotaDialog
          profile={editing}
          onCancel={() => setEditing(null)}
          onSaved={() => {
            setEditing(null);
            setMsg("配置已更新");
            void reloadAfterEdit();
          }}
        />
      )}

      {pendingDelete && (
        <ConfirmDialog
          tone="danger"
          icon={Trash2}
          eyebrow="订阅管理"
          title="确定要删除这个配置吗？"
          description={
            pendingDelete.type === "remote"
              ? "订阅链接与拉取到的节点会从本机移除，需要时可以重新导入同一个链接。"
              : "这个本地配置 / 节点串会从本机移除，导入的 YAML 文件本身不会被删除。"
          }
          subject={{
            label: pendingDelete.name,
            tag: pendingDelete.type === "remote" ? "订阅" : "本地 / 节点串",
            tagTone: pendingDelete.type === "remote" ? "blue" : undefined,
            meta: `更新于 ${pendingDelete.updated_at ?? "—"} · 自动更新：${
              pendingDelete.update_interval
                ? `每 ${pendingDelete.update_interval} 小时`
                : "关闭"
            }`,
          }}
          note={
            current?.uid === pendingDelete.uid
              ? {
                  tone: "warning",
                  text: "这条配置正在使用，删除后系统代理会回落到直连，正在进行的连接可能中断。",
                }
              : null
          }
          cancelLabel="取消"
          confirmLabel="删除配置"
          busyLabel="正在删除…"
          busy={deleting}
          error={deleteError}
          onCancel={() => {
            if (deleting) return;
            setPendingDelete(null);
            setDeleteError(null);
          }}
          onConfirm={() => void removeProfile()}
        />
      )}
    </div>
  );
}
