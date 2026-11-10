import { useAppStore } from "../../../store";
import { computeGate, gateLimits, type GateVerdict } from "../gate";

export interface EnvironmentVerdict extends GateVerdict {
  /** 已经跑完一轮检测（含原生检测）：启动页据此区分「未检测」与「未达标」 */
  scanned: boolean;
  scanning: boolean;
}

/**
 * 全应用唯一的启动判定入口。
 *
 * 检测页、AI 卡片页、首页、维护页与侧边栏都从这里取结论：同一份 store 快照、
 * 同一组策略参数、同一次计算，避免「检测页说无需修复、卡片页说未达标」。
 * 这里保持实时计算而不是缓存快照——privacy 字段会被 projectProtection 随内核
 * 状态实时投影，缓存会让门控停留在上一轮的结论上。
 */
export function useEnvironmentVerdict(): EnvironmentVerdict {
  const detect = useAppStore((s) => s.detect);
  const config = useAppStore((s) => s.config);
  const limits = gateLimits(config);
  const verdict = computeGate(
    detect,
    limits.threshold,
    limits.requireClaudeReachable,
  );
  return {
    ...verdict,
    scanned: detect.finished,
    scanning: detect.scanning,
  };
}
