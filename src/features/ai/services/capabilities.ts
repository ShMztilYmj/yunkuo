import { call } from "../../../services/api/invoke";
export interface CapabilityTarget {
  id: string;
  name: string;
  configPath: string;
  skillsPath: string;
  /** false 表示该工具只有 MCP（例如 Cursor），没有 Skills 目录。旧快照缺省为 true。 */
  skillsSupported?: boolean;
  hasBackup: boolean;
}
/** 条目来源：本地文件夹或某个工具的现有配置，带导入时的内容指纹。 */
export interface CapabilitySourceRef {
  kind: "folder" | "tool";
  path: string;
  name: string;
  hash: string;
}
export interface Capability {
  id: string;
  name: string;
  kind: "mcp" | "skill";
  transport?: string;
  fileCount: number;
  source: CapabilitySourceRef | null;
  enabled: Record<string, boolean>;
  errors: Record<string, string>;
  states: Record<
    string,
    "synced" | "pending" | "off" | "error" | "unsupported"
  >;
  /** 能力库展示信息，不参与项目投影与同步。 */
  title?: string;
  summary?: string;
  detail?: string;
  tags?: string[];
  repository?: string;
}
/** 已添加的 Agent：`target` 为空表示还没接入 MCP / Skills 同步。 */
export interface CapabilityAgent {
  id: string;
  name: string;
  target: string | null;
}
export interface CapabilitySnapshot {
  targets: CapabilityTarget[];
  /** 旧快照没有这个字段，前端要按 targets 兜底。 */
  agents?: CapabilityAgent[];
  entries: Capability[];
}
export interface SyncResult {
  entryId: string;
  targetId: string;
  name: string;
  target: string;
  ok: boolean;
  /** 被拒是因为目标里已有同名内容，可以交给用户选处理方式。 */
  conflict: boolean;
  error?: string;
}
export interface CapabilitySource {
  name: string;
  kind: "mcp" | "skill" | "error";
  path?: string;
}
export type ChangeStatus = "added" | "modified" | "removed";
export interface FileChange {
  path: string;
  status: ChangeStatus;
  before?: number;
  after?: number;
}
export interface FieldChange {
  field: string;
  status: ChangeStatus;
  before?: string;
  after?: string;
}
/** 「更新」预览：来源现在的内容和库里的条目差在哪。 */
export interface CapabilityDiff {
  name: string;
  kind: "mcp" | "skill";
  source: CapabilitySourceRef;
  changed: boolean;
  hash: string;
  files: FileChange[];
  fields: FieldChange[];
}
export interface SkillFile {
  path: string;
  size: number;
}
/** 左侧扑克牌正面用的 Skill 牌面：文件清单 + SKILL.md 开头。 */
export interface SkillDetail {
  id: string;
  name: string;
  source: CapabilitySourceRef | null;
  fileCount: number;
  files: SkillFile[];
  preview: string;
}
export type ConflictChoice = "adopt" | "overwrite" | "duplicate";
export type CapabilityAction =
  | { kind: "saveMcp"; id: string | null; name: string; spec: unknown }
  | { kind: "importSkill"; id: string | null; name: string; path: string }
  | { kind: "importMcp"; target: string; name: string; as_name: string }
  | { kind: "toggle"; id: string; target: string; enabled: boolean }
  | { kind: "sync"; id: string | null; target: string | null }
  | { kind: "delete"; id: string }
  | {
      kind: "updateInfo";
      id: string;
      title: string;
      summary: string;
      detail: string;
      tags: string[];
      repository: string;
    }
  | { kind: "applyUpdate"; id: string; expected: string }
  | {
      kind: "resolveConflict";
      id: string;
      target: string;
      choice: ConflictChoice;
      name?: string;
    }
  | { kind: "restoreBackup"; target: string };
export const capabilities = {
  snapshot: () => call<CapabilitySnapshot>("capability_snapshot"),
  action: (action: CapabilityAction) =>
    call<{ snapshot: CapabilitySnapshot; results: SyncResult[] }>(
      "capability_action",
      {
        action,
      },
    ),
  sources: (target: string) =>
    call<CapabilitySource[]>("capability_sources", { target }),
  detail: (id: string) => call<unknown>("capability_mcp_detail", { id }),
  diff: (id: string) => call<CapabilityDiff>("capability_diff", { id }),
  skill: (id: string) => call<SkillDetail>("capability_skill_detail", { id }),
};
