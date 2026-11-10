import { call } from "../../../services/api/invoke";

export interface ResumeCompatibility {
  supported: boolean;
  engineVersion: string | null;
  message: string;
  checkedAt: number | null;
}
export interface ResumeWatch {
  threadId: string;
  title: string;
  project: string;
  enabled: boolean;
  consumed: boolean;
}
export interface ResumeRecord {
  id: string;
  threadId: string;
  title: string;
  project: string;
  state: string;
  reason: string;
  resetAt: number | null;
  checkedAt: number | null;
  updatedAt: number;
  cancelRequested: boolean;
  mayBeQueued: boolean;
}
export interface ResumeStatus {
  enabled: boolean;
  compatibility: ResumeCompatibility;
  watches: ResumeWatch[];
  recoveries: ResumeRecord[];
  events: { recoveryId: string; at: number; state: string; reason: string }[];
}
export interface ResumeCandidate {
  threadId: string;
  title: string;
  project: string;
  quotaStopped: boolean;
}
export const codexResume = {
  status: () => call<ResumeStatus>("codex_resume_status"),
  candidates: () => call<ResumeCandidate[]>("codex_resume_candidates"),
  check: () => call<ResumeStatus>("codex_resume_check"),
  enable: (enabled: boolean) =>
    call<ResumeStatus>("codex_resume_enable", { enabled }),
  watch: (threadId: string, enabled = true) =>
    call<ResumeStatus>("codex_resume_watch", { threadId, enabled }),
  cancel: (threadId: string) =>
    call<ResumeStatus>("codex_resume_cancel", { threadId }),
  forget: (threadId: string) =>
    call<ResumeStatus>("codex_resume_forget", { threadId }),
};
export const claudeResume: typeof codexResume = {
  status: () => call<ResumeStatus>("claude_resume_status"),
  candidates: () => call<ResumeCandidate[]>("claude_resume_candidates"),
  check: () => call<ResumeStatus>("claude_resume_check"),
  enable: (enabled) => call<ResumeStatus>("claude_resume_enable", { enabled }),
  watch: (threadId, enabled = true) =>
    call<ResumeStatus>("claude_resume_watch", { threadId, enabled }),
  cancel: (threadId) =>
    call<ResumeStatus>("claude_resume_cancel", { threadId }),
  forget: (threadId) =>
    call<ResumeStatus>("claude_resume_forget", { threadId }),
};
const labels: Record<string, string> = {
  waiting_reset: "等待额度恢复",
  waiting_quota: "等待额度核验",
  waiting_thread: "等待会话就绪",
  waiting_environment: "等待环境就绪",
  waiting_history: "等待记录同步",
  submitting: "正在提交",
  queued: "已排队 · 等待送达",
  submission_unknown: "送达待核实",
  running: "已开始续跑",
  completed: "本轮已完成",
  stopped: "本轮已停止",
  superseded: "已由用户接手",
  cancelled: "已取消",
  expired: "计划已过期",
  failed: "恢复未能启动",
  paused_identity: "账号变化 · 已暂停",
  paused_configuration: "配置变化 · 已暂停",
};
export const resumeStateLabel = (state: string) =>
  labels[state] ?? "状态待核实";
export function resumeCountdown(reset: number, now: number) {
  const seconds = Math.ceil(reset - now);
  if (seconds <= 0) return "重置时间已到，等待实时核验";
  const minutes = Math.ceil(seconds / 60),
    hours = Math.floor(minutes / 60);
  return hours > 0
    ? `还剩 ${hours} 小时 ${minutes % 60} 分钟`
    : `还剩 ${minutes} 分钟`;
}
