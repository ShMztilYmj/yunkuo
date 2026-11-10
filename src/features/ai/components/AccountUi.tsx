import { type ReactNode } from "react";
import { useAsyncAction } from "../../../shared/hooks/useAsyncAction";

export const accountButton =
  "inline-flex items-center justify-center gap-2 rounded-lg border border-zinc-700 bg-zinc-900 px-3 py-2 text-xs text-zinc-200 hover:bg-zinc-800 disabled:cursor-not-allowed disabled:opacity-40";

export function AccountCard({
  title,
  children,
}: {
  title: string;
  children: ReactNode;
}) {
  return (
    <section
      aria-label={title}
      className="ui-panel space-y-4 border border-zinc-800 p-5"
    >
      <h2 className="text-sm font-semibold">{title}</h2>
      {children}
    </section>
  );
}

export function AccountMessage({ text }: { text: string | null }) {
  return text ? (
    <p
      role="status"
      className="whitespace-pre-wrap rounded-lg border border-zinc-700 bg-zinc-900/60 p-3 text-xs leading-6 text-zinc-300"
    >
      {text}
    </p>
  ) : null;
}

// 账号面板沿用 message 命名，行为完全由 useAsyncAction 承担。
export function useAccountOperation(onBusyChange?: (busy: boolean) => void) {
  const { busy, error, setError, run } = useAsyncAction(onBusyChange);
  return { busy, message: error, setMessage: setError, run };
}
