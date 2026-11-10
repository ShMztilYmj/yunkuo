import { useCallback, useState } from "react";
import type { IpIntel } from "../../../services/api";
import type { ExitResult } from "../signals/exit";
import {
  readExitSnapshot,
  saveExitSnapshot,
  type ExitSnapshot,
} from "../exitSnapshot";

/**
 * 出口快照的读写钩子：挂载时读一次，之后每拿到新情报就覆盖。
 *
 * 只服务出口通道卡的展示，不参与任何判定。情报取不到时不覆盖上一条 ——
 * 一次失败的情报查询不该把上一轮的好数据抹掉。
 */
export function useExitSnapshot() {
  const [snapshot, setSnapshot] = useState<ExitSnapshot | null>(() =>
    readExitSnapshot(),
  );

  const remember = useCallback(
    (
      exit: ExitResult | null,
      intel: IpIntel | null,
      countryAllowed: boolean | null,
    ) => {
      if (!intel) return;
      const saved = saveExitSnapshot({
        ip: intel.ip,
        exit,
        intel,
        countryAllowed,
      });
      if (saved) setSnapshot(saved);
    },
    [],
  );

  return { snapshot, remember };
}
