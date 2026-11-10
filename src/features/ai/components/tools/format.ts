/** 套餐标签配色，按套餐档位分档。 */
export function planChipClass(plan: string | null): string {
  switch (plan) {
    case "pro":
    case "max":
    case "team":
      return "bg-violet-500/15 text-violet-300";
    case "plus":
    case "business":
    case "claude_pro":
    case "claude_code":
      return "bg-sky-500/15 text-sky-300";
    case "free":
      return "bg-zinc-700/40 text-zinc-400";
    case "api":
      return "bg-amber-500/15 text-amber-300";
    default:
      return "bg-zinc-800 text-zinc-400";
  }
}
