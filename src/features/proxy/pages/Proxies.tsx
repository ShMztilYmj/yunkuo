import { useCallback, useEffect, useRef, useState } from "react";
import {
  RefreshCw,
  Signal,
  Loader2,
  Waypoints,
  ArrowDownUp,
} from "lucide-react";
import { proxyApi, type ProxyNode } from "../services/api";
import { buildGroups, resolveChain, type ProxyGroupView } from "../grouping";
import {
  DELAY_TTL_MS,
  filterNodes,
  freshDelay,
  loadDelays,
  presentRegions,
  regionLabel,
  regionOf,
  sampleAge,
  saveDelays,
  sortNodes,
  type DelayMap,
  type RegionKey,
  type SortMode,
} from "../node-view";
import ProxyTabs from "../components/ProxyTabs";
import PageHeader from "../../../shared/ui/PageHeader";
import { usePolling } from "../../../shared/hooks/usePolling";

type DelayTone = "idle" | "good" | "warn" | "bad";

const delayTone = (delay: number | undefined): DelayTone => {
  if (delay == null || delay <= 0) return "idle";
  if (delay < 200) return "good";
  if (delay < 500) return "warn";
  return "bad";
};

const delayLabel = (delay: number | undefined) =>
  delay != null && delay > 0 ? `${delay}ms` : "—";

const SORT_LABELS: Record<SortMode, string> = {
  default: "配置顺序",
  delay: "延迟从低到高",
  name: "名称",
};

/** 把「几分钟前」压成一行说明，避免在卡片里堆时间戳。 */
function ageLabel(ms: number | null): string {
  if (ms == null) return "未测速";
  const minutes = Math.floor(ms / 60_000);
  if (minutes < 1) return "刚刚测速";
  if (minutes < 60) return `${minutes} 分钟前`;
  return `${Math.floor(minutes / 60)} 小时前`;
}

export default function Proxies() {
  const [groups, setGroups] = useState<ProxyGroupView[]>([]);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [testing, setTesting] = useState<string | null>(null);
  const [delays, setDelays] = useState<DelayMap>(() => loadDelays());
  const [sort, setSort] = useState<SortMode>("default");
  const [region, setRegion] = useState<RegionKey | "all">("all");
  const [activeGroup, setActiveGroup] = useState<string | null>(null);
  const [now, setNow] = useState(() => Date.now());

  const loadingRef = useRef(false);
  const trackRef = useRef<HTMLDivElement>(null);
  const chipsRef = useRef<HTMLDivElement>(null);
  const scrollLock = useRef(false);
  const scrollTarget = useRef<string | null>(null);
  const scrollLockTimer = useRef<number | undefined>(undefined);
  /** 最近一次控制器快照：切换选择后立刻重算链路，不必等下一轮轮询。 */
  const nodesRef = useRef<Record<string, ProxyNode>>({});

  // 测速结果有时效；定时重绘让过期样本及时回到「未测速」。
  useEffect(() => {
    const timer = window.setInterval(
      () => setNow(Date.now()),
      Math.min(60_000, DELAY_TTL_MS / 2),
    );
    return () => window.clearInterval(timer);
  }, []);

  const load = useCallback(async (isCurrent: () => boolean = () => true) => {
    if (loadingRef.current) return;
    loadingRef.current = true;
    setLoading(true);
    setError(null);
    try {
      const data = await proxyApi.nodes();
      if (!isCurrent()) return;
      nodesRef.current = data;
      setGroups(buildGroups(data));
    } catch (e) {
      if (isCurrent()) setError(`无法连接内核控制器：${String(e)}`);
    } finally {
      loadingRef.current = false;
      if (isCurrent()) setLoading(false);
    }
  }, []);

  usePolling(load, 10000);

  const pinned = groups.find((group) => group.global) ?? null;
  const slides = groups.filter((group) => !group.global);
  const slideKey = slides.map((group) => group.id).join("|");
  const solo = pinned == null || slides.length === 0;
  const regions = presentRegions(groups.flatMap((group) => group.nodes));

  // 分组集合变化时只校正当前分组；轮询换数据、测速、切换节点都不打断滑动位置。
  useEffect(() => {
    setActiveGroup((current) =>
      current && slides.some((group) => group.id === current)
        ? current
        : (slides[0]?.id ?? null),
    );
  }, [slideKey]);

  // 当前分组被箭头/滑动改变时，把对应按钮带进可视区。
  useEffect(() => {
    if (!activeGroup) return;
    const index = slides.findIndex((group) => group.id === activeGroup);
    const chip = chipsRef.current?.children[index] as HTMLElement | undefined;
    chip?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, [activeGroup, slideKey]);

  // 平滑滚动期间不让 onScroll 改写高亮；滚完（或兜底超时）再落回刚点的分组。
  useEffect(() => {
    const track = trackRef.current;
    if (!track) return;
    const settle = () => {
      if (!scrollLock.current) return;
      scrollLock.current = false;
      const target = scrollTarget.current;
      scrollTarget.current = null;
      window.clearTimeout(scrollLockTimer.current);
      if (target) setActiveGroup(target);
    };
    track.addEventListener("scrollend", settle);
    return () => track.removeEventListener("scrollend", settle);
  }, [slideKey]);

  useEffect(
    () => () => {
      window.clearTimeout(scrollLockTimer.current);
    },
    [],
  );

  /** 轮询保持静默，只有手动刷新才让按钮进入忙碌态。 */
  const refresh = async () => {
    setRefreshing(true);
    try {
      await load();
    } finally {
      setRefreshing(false);
    }
  };

  /** 点按钮或滑到某张牌：横向轨道平滑定位到该组。 */
  const goToSlide = (index: number) => {
    const track = trackRef.current;
    const slide = track?.children[index] as HTMLElement | undefined;
    const target = slides[index];
    if (!track || !slide || !target) return;
    setActiveGroup(target.id);
    window.clearTimeout(scrollLockTimer.current);
    if (
      track.scrollWidth - track.clientWidth <= 2 ||
      Math.abs(track.scrollLeft - slide.offsetLeft) <= 2
    ) {
      scrollLock.current = false;
      scrollTarget.current = null;
      return;
    }
    scrollLock.current = true;
    scrollTarget.current = target.id;
    track.scrollTo({
      left: slide.offsetLeft,
      behavior: window.matchMedia("(prefers-reduced-motion: reduce)").matches
        ? "auto"
        : "smooth",
    });
    // scrollend 不支持时兜底；正常由上面的 settle 提前结束锁。
    scrollLockTimer.current = window.setTimeout(() => {
      scrollLock.current = false;
      const targetId = scrollTarget.current;
      scrollTarget.current = null;
      if (targetId) setActiveGroup(targetId);
    }, 1600);
  };

  /** 手动横滑时按最靠左的牌更新高亮；滑到尽头以最后一张为准。 */
  const syncActiveSlide = () => {
    const track = trackRef.current;
    if (!track || scrollLock.current || slides.length === 0) return;
    const position = track.scrollLeft;
    const max = track.scrollWidth - track.clientWidth;
    if (max > 2 && position >= max - 2) {
      setActiveGroup(slides[slides.length - 1].id);
      return;
    }
    let next = 0;
    let distance = Number.POSITIVE_INFINITY;
    slides.forEach((_, index) => {
      const slide = track.children[index] as HTMLElement | undefined;
      if (!slide) return;
      const delta = Math.abs(slide.offsetLeft - position);
      if (delta < distance) {
        distance = delta;
        next = index;
      }
    });
    setActiveGroup(slides[next].id);
  };

  const selectNode = async (group: string, name: string) => {
    try {
      await proxyApi.selectNode(group, name);
      const nodes = nodesRef.current;
      const target = nodes[group];
      if (target) nodes[group] = { ...target, selected: name };
      setGroups((gs) =>
        gs.map((g) => ({
          ...g,
          now: g.id === group ? name : g.now,
          chain: resolveChain(nodes, g.id).slice(1),
        })),
      );
    } catch (e) {
      setError(`切换节点失败：${String(e)}`);
    }
  };

  const testGroup = async (group: ProxyGroupView) => {
    setTesting(group.id);
    try {
      const result = (await proxyApi.delay(group.id, true)) as Record<
        string,
        number | Record<string, unknown>
      >;
      const at = Date.now();
      const fresh: DelayMap = {};
      for (const [name, value] of Object.entries(result)) {
        if (typeof value === "number" && value > 0) {
          fresh[name] = { delay: value, at };
        }
      }
      setDelays((current) => {
        const merged = { ...current, ...fresh };
        saveDelays(merged);
        return merged;
      });
      setNow(at);
    } catch {
      /* 组内部分节点测速失败属正常 */
    } finally {
      setTesting(null);
    }
  };

  /** 当前组在筛选 + 排序后的节点列表；空列表代表该地区没有节点。 */
  const arrange = (group: ProxyGroupView) =>
    sortNodes(filterNodes(group.nodes, region), delays, sort, now);

  const hasGroups = groups.length > 0;
  /** 页头副标题里的“当前”取正在滑到的那组；没有可滑动组时退回全局组。 */
  const focus =
    slides.find((group) => group.id === activeGroup) ?? slides[0] ?? pinned;
  // 同一个节点可能同时出现在多个分组里，按名字去重后再计数。
  const measured = new Set(
    groups
      .flatMap((group) => group.nodes)
      .filter((name) => freshDelay(delays, name, now) != null),
  ).size;

  return (
    <div className="proxies-page">
      <PageHeader
        icon={Waypoints}
        title="节点"
        badge={hasGroups ? `${groups.length} 个分组` : undefined}
        meta={
          hasGroups ? (
            <>
              <span>
                当前 <strong>{focus?.now || "—"}</strong>
              </span>
              <i className="proxy-meta-dot" />
              <span>
                {measured > 0 ? `${measured} 个节点测速有效` : "尚未测速"}
              </span>
              <i className="proxy-meta-dot" />
              <span>结果 {Math.round(DELAY_TTL_MS / 60_000)} 分钟内有效</span>
            </>
          ) : (
            <>
              <span>选择节点作为出口</span>
              <i className="proxy-meta-dot" />
              <span>尚无代理组</span>
            </>
          )
        }
        actions={
          <button
            type="button"
            onClick={() => void refresh()}
            disabled={refreshing}
            aria-busy={refreshing}
            className="proxy-action"
          >
            <RefreshCw className={refreshing ? "proxy-spin" : ""} />
            刷新
          </button>
        }
        tabs={<ProxyTabs />}
      />

      {!hasGroups ? (
        <div
          className={`proxies-empty${error ? " is-error" : ""}`}
          role={error ? "alert" : "status"}
        >
          {loading ? (
            <>
              <Loader2 className="animate-spin" aria-hidden="true" />
              <span>正在读取代理组…</span>
            </>
          ) : error ? (
            <>
              <span>{error}</span>
              <button type="button" onClick={() => void load()}>
                重试
              </button>
            </>
          ) : (
            <span>尚无代理组 —— 请先在「订阅」页导入配置</span>
          )}
        </div>
      ) : (
        <>
          {error && (
            <p className="proxies-alert" role="alert">
              <span>{error}</span>
              <button type="button" onClick={() => void load()}>
                重试
              </button>
            </p>
          )}
          <div
            className="proxy-filters"
            role="group"
            aria-label="节点筛选与排序"
          >
            <span className="proxy-filters-icon" aria-hidden="true">
              <ArrowDownUp size={13} />
            </span>
            <label>
              <span>排序</span>
              <select
                aria-label="节点排序"
                value={sort}
                onChange={(e) => setSort(e.target.value as SortMode)}
              >
                {(Object.keys(SORT_LABELS) as SortMode[]).map((mode) => (
                  <option key={mode} value={mode}>
                    {SORT_LABELS[mode]}
                  </option>
                ))}
              </select>
            </label>
            <label>
              <span>地区</span>
              <select
                aria-label="按地区筛选"
                value={region}
                onChange={(e) => setRegion(e.target.value as RegionKey | "all")}
              >
                <option value="all">全部地区</option>
                {regions.map((item) => (
                  <option key={item.key} value={item.key}>
                    {item.label}
                  </option>
                ))}
              </select>
            </label>
            {region !== "all" && (
              <span className="proxy-filters-note">
                只看{regionLabel(region)}节点，分组切换不影响筛选
              </span>
            )}
          </div>
          <div className={`proxies-main${solo ? " is-solo" : ""}`}>
            {pinned && (
              <div className="proxy-global">
                <GroupCard
                  group={pinned}
                  nodes={arrange(pinned)}
                  delays={delays}
                  now={now}
                  testing={testing === pinned.id}
                  onTest={() => testGroup(pinned)}
                  onSelect={(n) => selectNode(pinned.id, n)}
                />
              </div>
            )}
            {slides.length > 0 && (
              <section className="proxy-browser" aria-label="其他节点分组">
                <div
                  ref={chipsRef}
                  className="proxy-chips"
                  role="group"
                  aria-label="切换到节点分组"
                >
                  {slides.map((group, index) => (
                    <button
                      key={group.id}
                      type="button"
                      className="proxy-chip"
                      data-active={
                        activeGroup === group.id ? "true" : undefined
                      }
                      aria-pressed={activeGroup === group.id}
                      title={group.label}
                      onClick={() => goToSlide(index)}
                    >
                      <span>{group.label}</span>
                    </button>
                  ))}
                </div>
                <div className="proxy-slider">
                  <div
                    ref={trackRef}
                    className="proxy-track"
                    data-single={slides.length === 1 ? "true" : undefined}
                    onScroll={syncActiveSlide}
                  >
                    {slides.map((group) => (
                      <div className="proxy-slide" key={group.id}>
                        <GroupCard
                          group={group}
                          nodes={arrange(group)}
                          delays={delays}
                          now={now}
                          testing={testing === group.id}
                          onTest={() => testGroup(group)}
                          onSelect={(n) => selectNode(group.id, n)}
                        />
                      </div>
                    ))}
                  </div>
                </div>
              </section>
            )}
          </div>
        </>
      )}
    </div>
  );
}

const GLOBAL_HINT =
  "内核固定名 GLOBAL 不可改名；选全局路由时所有被接管流量都从这里出去";

function GroupCard({
  group,
  nodes,
  delays,
  now,
  testing,
  onTest,
  onSelect,
}: {
  group: ProxyGroupView;
  nodes: string[];
  delays: DelayMap;
  now: number;
  testing: boolean;
  onTest: () => void;
  onSelect: (name: string) => void;
}) {
  const current = group.chain.length > 0 ? group.chain.join(" → ") : "—";
  const newest = group.nodes.reduce<number | null>((latest, name) => {
    const age = sampleAge(delays, name, now);
    if (age == null) return latest;
    return latest == null ? age : Math.min(latest, age);
  }, null);
  return (
    <section
      className="proxy-group"
      data-global={group.global ? "true" : undefined}
      aria-label={`${group.label} 节点分组`}
    >
      <header className="proxy-group-head">
        <div className="proxy-group-title">
          <h2 title={group.id}>{group.label}</h2>
          {group.global && (
            <span className="proxy-group-badge">全局路由入口</span>
          )}
          <button
            type="button"
            onClick={onTest}
            disabled={testing}
            aria-busy={testing}
            data-tooltip={`测试「${group.label}」全部节点延迟`}
            className="proxy-group-test"
          >
            {testing ? (
              <Loader2 className="animate-spin" aria-hidden="true" />
            ) : (
              <Signal aria-hidden="true" />
            )}
            <span>{testing ? "测速中" : "测速"}</span>
          </button>
        </div>
        <p
          className="proxy-group-meta"
          title={`${group.type} · ${group.nodes.length} 节点 · 当前 ${current}`}
        >
          <span>{group.type}</span>
          <i aria-hidden="true">·</i>
          <span>{group.nodes.length} 节点</span>
          <i aria-hidden="true">·</i>
          <span>{ageLabel(newest)}</span>
          <i aria-hidden="true">·</i>
          <span className="proxy-group-now">
            当前 <strong>{current}</strong>
          </span>
        </p>
        {/* 非全局组保留一行占位，保证同一排牌面头部等高、节点区对齐。 */}
        <p
          className={`proxy-group-hint${group.global ? "" : " is-empty"}`}
          aria-hidden={group.global ? undefined : true}
          title={group.global ? GLOBAL_HINT : undefined}
        >
          {group.global ? GLOBAL_HINT : "\u00a0"}
        </p>
      </header>
      <div className="proxy-group-body">
        {nodes.length === 0 ? (
          <p className="proxy-group-empty" role="status">
            该地区没有节点，换个地区或选「全部地区」
          </p>
        ) : (
          <div className="proxy-node-grid">
            {nodes.map((name) => (
              <NodeTile
                key={name}
                name={name}
                region={regionOf(name)}
                selected={name === group.now}
                delay={freshDelay(delays, name, now) ?? undefined}
                age={sampleAge(delays, name, now)}
                onClick={() => onSelect(name)}
              />
            ))}
          </div>
        )}
      </div>
    </section>
  );
}

function NodeTile({
  name,
  region,
  selected,
  delay,
  age,
  onClick,
}: {
  name: string;
  region: RegionKey;
  selected: boolean;
  delay: number | undefined;
  age: number | null;
  onClick: () => void;
}) {
  const label = delayLabel(delay);
  const hint =
    delay != null
      ? `${name} · ${label} · ${ageLabel(age)}`
      : `${name} · ${regionLabel(region)} · 未测速或结果已过期`;
  return (
    <button
      type="button"
      onClick={onClick}
      title={hint}
      aria-pressed={selected}
      data-selected={selected ? "true" : undefined}
      className="proxy-node"
    >
      <span className="proxy-node-name">{name}</span>
      <span className="proxy-node-delay" data-tone={delayTone(delay)}>
        {label}
      </span>
    </button>
  );
}
