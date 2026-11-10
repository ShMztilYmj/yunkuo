import { useCallback, useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import {
  Plus,
  RefreshCw,
  FolderInput,
  Plug,
  BookOpen,
  Search,
  Trash2,
  Check,
  Copy,
  Sparkles,
  Terminal,
  type LucideIcon,
} from "lucide-react";
import {
  capabilities,
  type Capability,
  type CapabilityAction,
  type CapabilityDiff,
  type CapabilitySnapshot,
  type CapabilitySource,
  type ConflictChoice,
  type SyncResult,
} from "../services/capabilities";
import { usePolling } from "../../../shared/hooks/usePolling";
import { useConfirm } from "../../../shared/ui/useConfirm";
import CapabilityCard from "./CapabilityCard";

const example = JSON.stringify(
  { type: "stdio", command: "npx", args: ["-y", "your-mcp-package"] },
  null,
  2,
);
const labels = {
  synced: "已同步",
  pending: "待同步",
  off: "未启用",
  error: "读取失败",
  unsupported: "不兼容",
};
/** 同步目标图标：认不出的工具退回通用插头，新增工具时不用改这里。 */
const TARGET_ICONS: Record<string, LucideIcon> = {
  claude: Sparkles,
  codex: Terminal,
};
export default function CapabilityPanel() {
  const [data, setData] = useState<CapabilitySnapshot>(),
    [kind, setKind] = useState<"mcp" | "skill">("mcp"),
    [query, setQuery] = useState("");
  const [error, setError] = useState(""),
    [busy, setBusy] = useState(false),
    [results, setResults] = useState<SyncResult[]>([]),
    [notice, setNotice] = useState("");
  /** 刚复制过的路径：按钮短暂变成对勾，避免用户不确定有没有复制到。 */
  const [copiedPath, setCopiedPath] = useState<string | null>(null);
  const copyPath = async (value: string) => {
    try {
      await navigator.clipboard.writeText(value);
      setCopiedPath(value);
      window.setTimeout(
        () => setCopiedPath((current) => (current === value ? null : current)),
        1600,
      );
    } catch {
      /* 剪贴板不可用时保持原样：路径本身可以手动选中 */
    }
  };
  const [editor, setEditor] = useState<{
    id: string | null;
    name: string;
    spec: string;
  }>();
  const [skillEditor, setSkillEditor] = useState<{
    name: string;
    path: string;
  }>();
  const [infoEditor, setInfoEditor] = useState<{
    id: string;
    kind: string;
    name: string;
    title: string;
    summary: string;
    detail: string;
    tags: string;
    repository: string;
    shared: string[];
  }>();
  const [agentSources, setAgentSources] = useState<
      Record<string, CapabilitySource[]>
    >({}),
    [sourcesLoading, setSourcesLoading] = useState(false),
    [sourcesTick, setSourcesTick] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);
  const [diff, setDiff] = useState<{ id: string; data: CapabilityDiff }>(),
    [conflict, setConflict] = useState<{
      id: string;
      name: string;
      target: string;
      targetName: string;
    }>(),
    [duplicateName, setDuplicateName] = useState("");
  const [pickedId, setPickedId] = useState<string | null>(null);
  const [flipped, setFlipped] = useState(false);
  const picked = data?.entries.find((entry) => entry.id === pickedId) ?? null;
  /** Skills 视图不列只有 MCP 的工具（Cursor 等），否则会出现一列永远同步不了的开关。 */
  const visibleTargets = (data?.targets ?? []).filter(
    (target) => kind === "mcp" || target.skillsSupported !== false,
  );
  /** 已添加的 Agent 全表：旧快照没有 `agents` 时退回同步目标，保证列表不空。 */
  const agentRows: { id: string; name: string; target: string | null }[] = data
    ?.agents?.length
    ? data.agents
    : (data?.targets ?? []).map((target) => ({
        id: target.id,
        name: target.name,
        target: target.id,
      }));
  const sharedAgents = picked
    ? (data?.targets ?? [])
        .filter((target) => picked.enabled[target.id])
        .map((target) => target.name)
    : [];
  const { confirm, dialog } = useConfirm();
  const load = useCallback(async (isCurrent: () => boolean = () => true) => {
    try {
      const value = await capabilities.snapshot();
      if (isCurrent()) {
        setData(value);
        setError("");
      }
    } catch (e) {
      if (isCurrent()) setError(String(e));
    }
  }, []);
  const targetKey = data?.targets.map((target) => target.id).join("|") ?? "";
  useEffect(() => {
    if (!data?.targets.length) {
      setAgentSources({});
      return;
    }
    let active = true;
    setSourcesLoading(true);
    void Promise.allSettled(
      data.targets.map(async (target) => {
        try {
          const list = await capabilities.sources(target.id);
          return [target.id, Array.isArray(list) ? list : []] as [
            string,
            CapabilitySource[],
          ];
        } catch (error) {
          return [
            target.id,
            [
              {
                kind: "error" as const,
                name: String(error),
              },
            ],
          ] as [string, CapabilitySource[]];
        }
      }),
    )
      .then((results) => {
        if (!active) return;
        const next: Record<string, CapabilitySource[]> = {};
        for (const result of results) {
          if (result.status === "fulfilled") {
            const [id, list] = result.value;
            next[id] = list;
          }
        }
        setAgentSources(next);
      })
      .finally(() => {
        if (active) setSourcesLoading(false);
      });
    return () => {
      active = false;
    };
  }, [targetKey, sourcesTick]);
  usePolling(
    load,
    15000,
    true,
    !busy && !editor && !skillEditor && !infoEditor && !diff && !conflict,
  );
  async function run(task: () => Promise<unknown>) {
    if (busy) return;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      await task();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  async function change(action: CapabilityAction) {
    const value = await capabilities.action(action);
    setData(value.snapshot);
    setResults(value.results);
    const failed = value.results.filter((r) => !r.ok).length;
    const conflicts = value.results.filter((r) => r.conflict).length;
    setNotice(
      conflicts
        ? `${conflicts} 项有冲突，请在下方选择处理方式。`
        : failed
          ? `${failed} 项未同步，请查看下方结果后重试。`
          : value.results.length
            ? "同步完成。重新启动相关 CLI 或新建会话后加载。"
            : "已保存到能力库，可按工具启用。",
    );
    return value;
  }
  /** 单个来源接管：MCP 按目标配置导入，Skill 按目录导入，成功后落到上方能力库。 */
  async function takeOver(source: CapabilitySource, target: string) {
    const path = source.kind === "skill" ? source.path : undefined;
    if (source.kind === "skill" && !path) {
      setError(`读不到 ${source.name} 的目录，请用「导入 Skill」选择文件夹。`);
      return;
    }
    await run(async () => {
      const value =
        source.kind === "mcp"
          ? await change({
              kind: "importMcp",
              target,
              name: source.name,
              as_name: source.name,
            })
          : await change({
              kind: "importSkill",
              id: null,
              name: source.name,
              path: path ?? "",
            });
      const hit = value.results.find((result) => result.name === source.name);
      setKind(source.kind === "mcp" ? "mcp" : "skill");
      setNotice(
        hit && !hit.ok
          ? `${source.name} 没有接管成功：${hit.error ?? "请查看下方结果"}`
          : hit?.conflict
            ? `${source.name} 与能力库同名，请在下方选择处理方式。`
            : `已接管 ${source.name}，可在上方能力库统一启停。`,
      );
    });
  }
  async function importSkill(entry?: Capability, path?: string, name?: string) {
    const selected =
      path ??
      (await open({
        directory: true,
        multiple: false,
        title: "选择直接包含 SKILL.md 的文件夹",
      }));
    if (typeof selected !== "string") return;
    const inferred =
      selected
        .replace(/[\\/]$/, "")
        .split(/[\\/]/)
        .pop() || "skill";
    await change({
      kind: "importSkill",
      id: entry?.id ?? null,
      name: entry?.name ?? name ?? inferred,
      path: selected,
    });
  }
  async function chooseSkill() {
    const selected = await open({
      directory: true,
      multiple: false,
      title: "选择直接包含 SKILL.md 的文件夹",
    });
    if (typeof selected !== "string") return;
    const inferred =
      selected
        .replace(/[\\/]$/, "")
        .split(/[\\/]/)
        .pop() || "skill";
    setSkillEditor({
      name: /^[A-Za-z0-9_-]{1,80}$/.test(inferred) ? inferred : "my-skill",
      path: selected,
    });
  }
  const entries = (data?.entries ?? []).filter((e) => {
    const needle = query.trim().toLowerCase();
    return (
      e.kind === kind &&
      (!needle ||
        e.name.toLowerCase().includes(needle) ||
        (e.title ?? "").toLowerCase().includes(needle))
    );
  });
  const libraryKeys = new Set(
    (data?.entries ?? []).map((entry) => `${entry.kind}:${entry.name}`),
  );
  function targetName(id: string) {
    return data?.targets.find((t) => t.id === id)?.name ?? id;
  }
  function sourceName(entry: Capability) {
    if (!entry.source) return "手工配置";
    if (entry.source.kind === "folder") return "本地文件夹";
    return targetName(entry.source.path);
  }
  const displayTitle = (entry: Capability) => entry.title?.trim() || entry.name;
  function openInfo(entry: Capability) {
    setInfoEditor({
      id: entry.id,
      kind: entry.kind,
      name: entry.name,
      title: entry.title ?? "",
      summary: entry.summary ?? "",
      detail: entry.detail ?? "",
      tags: (entry.tags ?? []).join("，"),
      repository: entry.repository ?? "",
      shared: (data?.targets ?? [])
        .filter((target) => entry.enabled[target.id])
        .map((target) => target.name),
    });
  }
  /** 点一行 MCP / Skill：牌面翻到正面；再点同一行翻回背面。 */
  function pickEntry(entry: Capability) {
    if (picked?.id === entry.id && flipped) {
      setFlipped(false);
      return;
    }
    setPickedId(entry.id);
    setFlipped(true);
  }
  function toggleCard() {
    if (!picked) return;
    setFlipped((value) => !value);
  }
  async function previewUpdate(entry: Capability) {
    if (!entry.source) {
      // 旧条目没有记录来源：退回原来的「重新选文件夹」，选完会补上来源。
      await importSkill(entry);
      return;
    }
    await run(async () => {
      setDiff({ id: entry.id, data: await capabilities.diff(entry.id) });
    });
  }
  async function applyDiff() {
    if (!diff) return;
    await run(async () => {
      await change({
        kind: "applyUpdate",
        id: diff.id,
        expected: diff.data.hash,
      });
      setDiff(undefined);
    });
  }
  async function resolve(choice: ConflictChoice) {
    if (!conflict) return;
    const name =
      choice === "duplicate"
        ? duplicateName.trim() || `${conflict.name}-local`
        : undefined;
    await run(async () => {
      await change({
        kind: "resolveConflict",
        id: conflict.id,
        target: conflict.target,
        choice,
        name,
      });
      setConflict(undefined);
      setDuplicateName("");
    });
  }
  return (
    <section className="capability-panel" aria-label="MCP 与 Skills 能力库">
      {error && (
        <div className="hub-notice is-error" role="alert">
          {error}
        </div>
      )}
      {notice && (
        <div className="hub-notice" role="status">
          {notice}
        </div>
      )}
      <div className="capability-toolbar" ref={listRef}>
        <div className="hub-subtabs" aria-label="能力类型">
          <button
            aria-pressed={kind === "mcp"}
            className={kind === "mcp" ? "is-active" : ""}
            onClick={() => {
              setKind("mcp");
              setSkillEditor(undefined);
            }}
          >
            <Plug size={15} />
            MCP 服务{" "}
            <small>
              {data?.entries.filter((e) => e.kind === "mcp").length ?? 0}
            </small>
          </button>
          <button
            aria-pressed={kind === "skill"}
            className={kind === "skill" ? "is-active" : ""}
            onClick={() => {
              setKind("skill");
              setEditor(undefined);
            }}
          >
            <BookOpen size={15} />
            Skills{" "}
            <small>
              {data?.entries.filter((e) => e.kind === "skill").length ?? 0}
            </small>
          </button>
        </div>
        <div className="capability-toolbar-end">
          {!!data?.entries.length && (
            <label className="capability-search">
              <Search size={15} />
              <input
                aria-label="搜索能力"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="搜索"
              />
            </label>
          )}
          <div className="hub-actions">
            <button
              className="capability-icon-button"
              disabled={busy}
              onClick={() =>
                void run(async () => {
                  await load();
                  setSourcesTick((tick) => tick + 1);
                })
              }
              aria-label="刷新能力库"
              data-tooltip="刷新能力库"
            >
              <RefreshCw size={14} />
            </button>
            {!!data?.entries.length && (
              <button
                disabled={busy}
                onClick={() =>
                  void run(() =>
                    change({ kind: "sync", id: null, target: null }),
                  )
                }
              >
                <RefreshCw size={14} />
                同步全部
              </button>
            )}
            {kind === "mcp" ? (
              <button
                className="hub-primary"
                disabled={busy}
                onClick={() => setEditor({ id: null, name: "", spec: example })}
              >
                <Plus size={14} />
                添加 MCP
              </button>
            ) : (
              <button
                className="hub-primary"
                disabled={busy}
                onClick={() => void run(chooseSkill)}
              >
                <FolderInput size={14} />
                导入 Skill
              </button>
            )}
          </div>
        </div>
      </div>
      {infoEditor && (
        <form
          className="hub-editor capability-info-editor"
          aria-label="编辑能力信息"
          onSubmit={(e) => {
            e.preventDefault();
            void run(async () => {
              await change({
                kind: "updateInfo",
                id: infoEditor.id,
                title: infoEditor.title,
                summary: infoEditor.summary,
                detail: infoEditor.detail,
                tags: infoEditor.tags
                  .split(/[，,]/)
                  .map((tag) => tag.trim())
                  .filter(Boolean),
                repository: infoEditor.repository,
              });
              setNotice("能力信息已保存。");
              setInfoEditor(undefined);
            });
          }}
        >
          <header>
            <h3>编辑 {infoEditor.kind === "mcp" ? "MCP" : "Skill"} 信息</h3>
            <button
              type="button"
              disabled={busy}
              onClick={() => setInfoEditor(undefined)}
            >
              取消
            </button>
          </header>
          <div className="capability-info-grid">
            <label className="is-wide">
              ID
              <input readOnly value={infoEditor.id} />
            </label>
            <label>
              名称
              <input
                value={infoEditor.title}
                placeholder={infoEditor.name}
                maxLength={80}
                onChange={(e) =>
                  setInfoEditor({ ...infoEditor, title: e.target.value })
                }
              />
            </label>
            <label>
              能力
              <input
                value={infoEditor.summary}
                placeholder="例如：文件系统与浏览器自动化"
                maxLength={500}
                onChange={(e) =>
                  setInfoEditor({ ...infoEditor, summary: e.target.value })
                }
              />
            </label>
            <label>
              开源项目地址
              <input
                type="url"
                value={infoEditor.repository}
                placeholder="https://github.com/..."
                onChange={(e) =>
                  setInfoEditor({
                    ...infoEditor,
                    repository: e.target.value,
                  })
                }
              />
            </label>
            <label>
              标签
              <input
                value={infoEditor.tags}
                placeholder="用逗号分隔，例如：浏览器，自动化"
                onChange={(e) =>
                  setInfoEditor({ ...infoEditor, tags: e.target.value })
                }
              />
            </label>
            <label className="is-wide">
              Skill / MCP 详情
              <textarea
                rows={4}
                value={infoEditor.detail}
                placeholder="补充这个能力的用途、限制或使用说明"
                maxLength={8000}
                onChange={(e) =>
                  setInfoEditor({ ...infoEditor, detail: e.target.value })
                }
              />
            </label>
          </div>
          <div className="capability-info-shared">
            <span>共用 Agent</span>
            <div>
              {infoEditor.shared.length ? (
                infoEditor.shared.map((name) => (
                  <span className="capability-agent-item" key={name}>
                    {name}
                  </span>
                ))
              ) : (
                <span className="capability-agent-empty">
                  尚未在 Agent 中启用
                </span>
              )}
            </div>
            <small>由主列表中的 Agent 启用开关决定。</small>
          </div>
          <p className="hub-hint">
            只修改能力库中的展示信息，不改写 MCP 配置或 Skill 文件。
          </p>
          <button className="hub-primary" disabled={busy} type="submit">
            保存信息
          </button>
        </form>
      )}
      {skillEditor && (
        <form
          className="hub-editor"
          aria-label="Skill 文件夹导入"
          onSubmit={(e) => {
            e.preventDefault();
            void run(async () => {
              await importSkill(undefined, skillEditor.path, skillEditor.name);
              setSkillEditor(undefined);
            });
          }}
        >
          <header>
            <h3>导入 Skill 文件夹</h3>
            <button
              type="button"
              disabled={busy}
              onClick={() => setSkillEditor(undefined)}
            >
              取消
            </button>
          </header>
          <label>
            Skill 编号
            <input
              required
              pattern="[A-Za-z0-9_-]{1,80}"
              value={skillEditor.name}
              onChange={(e) =>
                setSkillEditor({ ...skillEditor, name: e.target.value })
              }
            />
          </label>
          <label>
            来源文件夹
            <input readOnly value={skillEditor.path} />
          </label>
          <p className="hub-hint">
            复制 SKILL.md
            和文件夹内资源到能力库。源文件夹保留；以后使用“更新”重新导入。
          </p>
          <button disabled={busy} className="hub-primary" type="submit">
            保存 Skill
          </button>
        </form>
      )}
      {diff && (
        <section className="hub-editor capability-diff" aria-label="更新预览">
          <header>
            <h3>「{diff.data.name}」的更新</h3>
            <button
              type="button"
              disabled={busy}
              onClick={() => setDiff(undefined)}
            >
              取消
            </button>
          </header>
          <p className="hub-hint">
            来源：
            {diff.data.source.kind === "tool"
              ? `工具 ${targetName(diff.data.source.path)}`
              : diff.data.source.path}
            {diff.data.changed ? " · 有变化，确认后再写入" : " · 与库里一致"}
          </p>
          {!!diff.data.files.length && (
            <ul className="capability-changes">
              {diff.data.files.map((change) => (
                <li key={change.path} data-status={change.status}>
                  <code>{change.path}</code>
                  <small>
                    {change.status === "added"
                      ? "新增"
                      : change.status === "removed"
                        ? "删除"
                        : "修改"}
                    {change.before !== undefined && change.after !== undefined
                      ? ` · ${change.before} → ${change.after} B`
                      : ""}
                  </small>
                </li>
              ))}
            </ul>
          )}
          {!!diff.data.fields.length && (
            <ul className="capability-changes">
              {diff.data.fields.map((change) => (
                <li key={change.field} data-status={change.status}>
                  <code>{change.field || "(根)"}</code>
                  <small>
                    {change.status === "added"
                      ? "新增"
                      : change.status === "removed"
                        ? "删除"
                        : "修改"}
                  </small>
                  <span className="capability-values">
                    {change.before !== undefined && <del>{change.before}</del>}
                    {change.after !== undefined && <ins>{change.after}</ins>}
                  </span>
                </li>
              ))}
            </ul>
          )}
          {!diff.data.changed && (
            <p className="hub-hint">来源没有新内容，不需要更新。</p>
          )}
          <div className="hub-actions">
            <button
              className="hub-primary"
              disabled={busy || !diff.data.changed}
              onClick={() => void applyDiff()}
            >
              应用更新（{diff.data.files.length + diff.data.fields.length} 项）
            </button>
          </div>
        </section>
      )}
      {conflict && (
        <section
          className="hub-editor capability-conflict"
          aria-label="冲突处理"
        >
          <header>
            <h3>
              「{conflict.name}」在 {conflict.targetName} 里已有同名内容
            </h3>
            <button
              type="button"
              disabled={busy}
              onClick={() => setConflict(undefined)}
            >
              关闭
            </button>
          </header>
          <p className="hub-hint">
            这份内容不是能力库写的，或者同步之后被改过。能力库不会自动覆盖，选一种处理方式：
          </p>
          <div className="capability-choices">
            <label>
              另存用的新编号
              <input
                value={duplicateName}
                disabled={busy}
                placeholder={`${conflict.name}-local`}
                onChange={(e) => setDuplicateName(e.target.value)}
              />
            </label>
            <button
              className="hub-primary"
              disabled={busy}
              onClick={() => void resolve("duplicate")}
            >
              另存为新编号（不动工具）
            </button>
            <button disabled={busy} onClick={() => void resolve("adopt")}>
              保留工具里的版本（写回能力库）
            </button>
            <button disabled={busy} onClick={() => void resolve("overwrite")}>
              以能力库为准覆盖工具
            </button>
          </div>
        </section>
      )}
      {editor && (
        <form
          className="hub-editor"
          aria-label="MCP 配置编辑"
          onSubmit={(e) => {
            e.preventDefault();
            void run(async () => {
              const spec: unknown = JSON.parse(editor.spec);
              await change({
                kind: "saveMcp",
                id: editor.id,
                name: editor.name,
                spec,
              });
              setEditor(undefined);
            });
          }}
        >
          <header>
            <h3>{editor.id ? "编辑 MCP" : "添加 MCP"}</h3>
            <button
              type="button"
              disabled={busy}
              onClick={() => setEditor(undefined)}
            >
              取消
            </button>
          </header>
          <label>
            编号
            <input
              required
              pattern="[A-Za-z0-9_-]{1,80}"
              readOnly={!!editor.id}
              value={editor.name}
              onChange={(e) => setEditor({ ...editor, name: e.target.value })}
              placeholder="例如 browser-tools"
            />
          </label>
          <label>
            MCP 配置 JSON
            <textarea
              required
              rows={10}
              spellCheck={false}
              value={editor.spec}
              onChange={(e) => setEditor({ ...editor, spec: e.target.value })}
            />
          </label>
          <p className="hub-hint">
            填写单个服务对象。支持 stdio 的 command / args / env / cwd，或
            HTTP、SSE 的 type / url /
            headers。密钥在能力库里加密保存；启用后按工具格式写入对应配置文件，工具自身仍以明文保存。
          </p>
          <button className="hub-primary" disabled={busy} type="submit">
            保存 MCP
          </button>
        </form>
      )}
      <div className="capability-body">
        <div className="capability-main">
          {!data && !error && <p className="hub-empty">正在读取能力库…</p>}
          {data && !entries.length && (
            <div className="hub-empty capability-empty">
              {kind === "mcp" ? <Plug size={27} /> : <BookOpen size={27} />}
              <h3>
                {query
                  ? "没有匹配的能力"
                  : kind === "mcp"
                    ? "添加第一个 MCP 服务"
                    : "导入第一个 Skill"}
              </h3>
              <p>
                {query
                  ? "换个名称搜索。"
                  : kind === "mcp"
                    ? "从已有工具导入，或添加一个服务配置，再选择需要启用的工具。"
                    : "选择包含 SKILL.md 和相关资源的文件夹，集中分发到各工具。"}
              </p>
            </div>
          )}
          {!!entries.length && (
            <div className="capability-table-wrap">
              <table className="capability-table">
                <thead>
                  <tr>
                    <th scope="col">{kind === "mcp" ? "MCP 服务" : "Skill"}</th>
                    {visibleTargets.map((t) => (
                      <th
                        scope="col"
                        key={t.id}
                        title={kind === "mcp" ? t.configPath : t.skillsPath}
                      >
                        {t.name}
                      </th>
                    ))}
                    <th scope="col">管理</th>
                  </tr>
                </thead>
                <tbody>
                  {entries.map((entry) => (
                    <tr
                      key={entry.id}
                      data-kind={entry.kind}
                      data-picked={
                        picked?.id === entry.id && flipped ? "true" : undefined
                      }
                      onClick={() => pickEntry(entry)}
                    >
                      <th scope="row">
                        <strong data-tooltip={entry.name}>
                          <span
                            className="capability-row-kind"
                            data-kind={entry.kind}
                            aria-hidden="true"
                          >
                            {entry.kind === "mcp" ? (
                              <Plug size={11} />
                            ) : (
                              <BookOpen size={11} />
                            )}
                          </span>
                          {displayTitle(entry)}
                          {entry.title?.trim() &&
                            entry.title.trim() !== entry.name && (
                              <em className="capability-row-realname">
                                {entry.name}
                              </em>
                            )}
                        </strong>
                        <small>
                          {entry.summary?.trim()
                            ? `${entry.summary.trim()} · `
                            : ""}
                          {entry.kind === "mcp"
                            ? entry.transport?.toUpperCase()
                            : `${entry.fileCount} 个文件 · 复制分发`}
                          {" · 来源："}
                          {sourceName(entry)}
                        </small>
                        {(!!entry.tags?.length || entry.repository) && (
                          <span className="capability-tags-row">
                            {entry.tags?.map((tag) => (
                              <span
                                className="capability-tag"
                                key={`${entry.id}:${tag}`}
                              >
                                {tag}
                              </span>
                            ))}
                            {entry.repository && (
                              <a
                                className="capability-repo-link"
                                href={entry.repository}
                                target="_blank"
                                rel="noreferrer"
                                onClick={(event) => event.stopPropagation()}
                              >
                                开源项目 ↗
                              </a>
                            )}
                          </span>
                        )}
                      </th>
                      {visibleTargets.map((target) => (
                        <td
                          key={target.id}
                          onClick={(event) => event.stopPropagation()}
                        >
                          <label className="capability-switch">
                            <input
                              type="checkbox"
                              role="switch"
                              aria-label={`${displayTitle(entry)} 在 ${target.name} 启用`}
                              disabled={busy}
                              checked={!!entry.enabled[target.id]}
                              onChange={(e) =>
                                void run(() =>
                                  change({
                                    kind: "toggle",
                                    id: entry.id,
                                    target: target.id,
                                    enabled: e.target.checked,
                                  }),
                                )
                              }
                            />
                            <span />
                          </label>
                          <small
                            className={
                              entry.errors[target.id] ? "is-error" : ""
                            }
                          >
                            {entry.errors[target.id]
                              ? "同步失败"
                              : (labels[entry.states[target.id]] ?? "待检查")}
                          </small>
                          {entry.errors[target.id] && (
                            <details>
                              <summary>查看原因</summary>
                              <p>{entry.errors[target.id]}</p>
                            </details>
                          )}
                        </td>
                      ))}
                      <td onClick={(event) => event.stopPropagation()}>
                        <div className="capability-row-actions">
                          <button
                            disabled={busy}
                            onClick={() => openInfo(entry)}
                          >
                            信息
                          </button>
                          <button
                            disabled={busy}
                            onClick={() => {
                              if (entry.kind === "mcp") {
                                void run(async () =>
                                  setEditor({
                                    id: entry.id,
                                    name: entry.name,
                                    spec: JSON.stringify(
                                      await capabilities.detail(entry.id),
                                      null,
                                      2,
                                    ),
                                  }),
                                );
                              } else {
                                void previewUpdate(entry);
                              }
                            }}
                          >
                            {entry.kind === "mcp" ? "编辑" : "更新"}
                          </button>
                          {entry.kind === "mcp" && entry.source && (
                            <button
                              disabled={busy}
                              onClick={() => void previewUpdate(entry)}
                            >
                              更新
                            </button>
                          )}
                          <button
                            disabled={busy}
                            onClick={() =>
                              void run(() =>
                                change({
                                  kind: "sync",
                                  id: entry.id,
                                  target: null,
                                }),
                              )
                            }
                          >
                            重试同步
                          </button>
                          <button
                            disabled={busy}
                            onClick={() =>
                              void run(async () => {
                                if (
                                  await confirm({
                                    tone: "danger",
                                    icon: Trash2,
                                    eyebrow: "能力库 · 移除",
                                    title: `移除「${entry.name}」？`,
                                    description:
                                      "会从能力库移除这一项，并撤下已经同步到各个工具的副本。",
                                    note: {
                                      tone: "neutral",
                                      text: "同步之后被外部改动过的副本会保留，并在下次同步时提示冲突。",
                                    },
                                    subject: {
                                      label: entry.name,
                                      tag:
                                        entry.kind === "mcp" ? "MCP" : "Skill",
                                      tagTone:
                                        entry.kind === "mcp" ? "blue" : "green",
                                    },
                                    confirmLabel: "移除",
                                  })
                                )
                                  await change({
                                    kind: "delete",
                                    id: entry.id,
                                  });
                              })
                            }
                          >
                            移除
                          </button>
                        </div>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
          {!!results.length && (
            <details
              className="capability-results"
              open={results.some((r) => !r.ok)}
            >
              <summary>
                最近同步结果 · {results.filter((r) => r.ok).length} 成功，
                {results.filter((r) => !r.ok).length} 未完成
              </summary>
              {results.map((r, i) => (
                <p key={i} data-failed={!r.ok}>
                  <strong>
                    {r.name} → {r.target}
                  </strong>
                  ：{r.ok ? "已同步" : r.error}
                  {r.conflict && r.entryId && (
                    <button
                      className="capability-inline"
                      disabled={busy}
                      onClick={() => {
                        setDuplicateName(`${r.name}-local`);
                        setConflict({
                          id: r.entryId,
                          name: r.name,
                          target: r.targetId,
                          targetName: r.target,
                        });
                      }}
                    >
                      处理冲突
                    </button>
                  )}
                </p>
              ))}
            </details>
          )}
        </div>
        <CapabilityCard
          entry={picked}
          sharedAgents={sharedAgents}
          flipped={flipped}
          sourceLabel={picked ? sourceName(picked) : ""}
          onToggle={toggleCard}
        />
      </div>
      <section
        className="capability-agents-card"
        aria-labelledby="capability-agents-title"
      >
        <header className="capability-agents-head">
          <h2 id="capability-agents-title">
            Agent 列表
            <span className="capability-paths-count">
              {agentRows.length} 个
            </span>
          </h2>
        </header>
        <div className="capability-target-grid">
          {agentRows.map((agent) => {
            const t = data?.targets.find(
              (target) => target.id === agent.target,
            );
            if (!t) {
              return (
                <article
                  className="capability-target"
                  data-unsupported="true"
                  key={agent.id}
                >
                  <header>
                    <span className="capability-target-icon" aria-hidden="true">
                      <Plug size={12} />
                    </span>
                    <strong>{agent.name}</strong>
                    <span className="capability-target-state">
                      未接入能力同步
                    </span>
                  </header>
                  <p className="capability-target-hint">
                    该工具暂未提供 MCP / Skills
                    的同步位置，接入后会出现在这里，也能单独接管。
                  </p>
                </article>
              );
            }
            const Icon = TARGET_ICONS[t.id] ?? Plug;
            const list = agentSources[t.id] ?? [];
            const mcp = list.filter((source) => source.kind === "mcp");
            const skills = list.filter((source) => source.kind === "skill");
            const errors = list.filter((source) => source.kind === "error");
            const total = mcp.length + skills.length;
            return (
              <article className="capability-target" key={t.id}>
                <header>
                  <span className="capability-target-icon" aria-hidden="true">
                    <Icon size={12} />
                  </span>
                  <strong>{t.name}</strong>
                  <span className="capability-target-state">
                    {sourcesLoading
                      ? "读取中…"
                      : total
                        ? t.skillsSupported === false
                          ? `${mcp.length} MCP`
                          : `${mcp.length} MCP · ${skills.length} Skills`
                        : t.skillsSupported === false
                          ? "无 MCP 服务"
                          : "无 MCP / Skills"}
                  </span>
                </header>
                {errors.map((error) => (
                  <p
                    className="capability-target-error"
                    key={`error:${error.name}`}
                  >
                    {error.name}
                  </p>
                ))}
                <div className="capability-target-paths">
                  <span className="capability-target-paths-title">
                    配置路径
                  </span>
                  {[
                    ["mcp", "MCP", t.configPath, `${t.name} 的 MCP 配置路径`],
                    [
                      "skill",
                      "Skills",
                      t.skillsPath,
                      `${t.name} 的 Skills 目录`,
                    ],
                  ]
                    .filter(
                      ([group]) =>
                        group === kind &&
                        (group !== "skill" || t.skillsSupported !== false),
                    )
                    .map(([, tag, path, description]) => (
                      <div className="capability-target-row" key={tag}>
                        <span className="capability-target-tag">{tag}</span>
                        <code title={path}>{path}</code>
                        <button
                          type="button"
                          className="capability-copy"
                          aria-label={`复制${description}`}
                          onClick={() => void copyPath(path)}
                        >
                          {copiedPath === path ? (
                            <Check size={11} aria-hidden="true" />
                          ) : (
                            <Copy size={11} aria-hidden="true" />
                          )}
                        </button>
                      </div>
                    ))}
                </div>
                {(
                  [
                    { key: "mcp", label: "MCP", icon: Plug, items: mcp },
                    {
                      key: "skill",
                      label: "Skills",
                      icon: BookOpen,
                      items: skills,
                    },
                  ] as const
                )
                  .filter((group) => group.key === kind)
                  .map(({ key, label, icon: GroupIcon, items }) => (
                    <div className="capability-agent-group" key={key}>
                      <span className="capability-agent-label">
                        <GroupIcon size={11} aria-hidden="true" />
                        {label}
                        <em className="capability-agent-count">
                          {items.length}
                        </em>
                      </span>
                      {label === "Skills" && t.skillsSupported === false ? (
                        <span className="capability-agent-empty">
                          该工具只支持 MCP
                        </span>
                      ) : items.length ? (
                        <ul className="capability-agent-items">
                          {items.map((source) => {
                            const taken = libraryKeys.has(
                              `${source.kind}:${source.name}`,
                            );
                            return (
                              <li
                                className="capability-agent-row"
                                data-taken={taken || undefined}
                                key={source.name}
                              >
                                <span
                                  className="capability-agent-name"
                                  title={source.name}
                                >
                                  {source.name}
                                </span>
                                {taken ? (
                                  <span className="capability-agent-taken">
                                    <Check size={10} aria-hidden="true" />
                                    已接管
                                  </span>
                                ) : (
                                  <button
                                    type="button"
                                    className="capability-take"
                                    disabled={busy}
                                    aria-label={`接管 ${source.name}`}
                                    onClick={() => void takeOver(source, t.id)}
                                  >
                                    接管
                                  </button>
                                )}
                              </li>
                            );
                          })}
                        </ul>
                      ) : (
                        <span className="capability-agent-empty">
                          {sourcesLoading ? "读取中…" : "无"}
                        </span>
                      )}
                    </div>
                  ))}
                {t.hasBackup && (
                  <div className="capability-target-foot">
                    <button
                      type="button"
                      className="capability-inline"
                      disabled={busy}
                      onClick={() =>
                        void run(async () => {
                          const ok = await confirm({
                            tone: "warning",
                            eyebrow: "能力库 · 还原",
                            title: `还原 ${t.name} 首次同步前的配置？`,
                            description:
                              "会用第一次同步前保存的原文件覆盖当前配置，之后需要重新启用要用的条目。",
                            confirmLabel: "还原",
                          });
                          if (ok)
                            await change({
                              kind: "restoreBackup",
                              target: t.id,
                            });
                        })
                      }
                    >
                      还原首次同步前的配置
                    </button>
                  </div>
                )}
              </article>
            );
          })}
        </div>
      </section>
      {dialog}
    </section>
  );
}
