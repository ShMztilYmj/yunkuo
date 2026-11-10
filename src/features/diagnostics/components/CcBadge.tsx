/** 国家码小徽标：Windows 下缺国旗 emoji 字体，统一用字母码比国旗更稳。 */
export default function CcBadge({ cc }: { cc?: string | null }) {
  const code = (cc ?? "").toUpperCase();
  return (
    <span className="inline-flex h-[15px] min-w-[20px] shrink-0 items-center justify-center rounded-[3px] border border-zinc-700/80 bg-zinc-800/70 px-[3px] text-[9px] font-medium leading-none tracking-wide text-zinc-400">
      {code || "--"}
    </span>
  );
}
