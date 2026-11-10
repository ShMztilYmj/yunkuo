import { Clock } from "lucide-react";
import type { TzStatus } from "../../../services/api";
import Card from "./Card";
import Note from "./Note";
import Row from "./Row";

/** 当前系统时区：Windows 时区名与 IANA 推断，中国标准时间给出风控提示。 */
export default function TimezoneStatusCard({
  tzStatus,
}: {
  tzStatus: TzStatus | null;
}) {
  const china = tzStatus?.current.key_name === "China Standard Time";
  return (
    <Card
      title="当前系统时区"
      icon={<Clock />}
      description="系统时间与日期当前使用的区域设置，以及对应的 IANA 时区。"
      badge={tzStatus == null ? "读取中" : china ? "建议同步" : "时区已就绪"}
      badgeTone={tzStatus == null ? "idle" : china ? "warn" : "ok"}
    >
      <Row label="Windows 时区" hint="系统设置中生效的时区">
        <span className="settings-value">
          {tzStatus?.current.key_name ?? "读取中…"}
          {tzStatus?.current.display && (
            <small> · {tzStatus.current.display}</small>
          )}
        </span>
      </Row>
      <Row label="IANA 推断" hint="用于与出口 IP 归属地比对">
        {tzStatus?.current.iana_guess ? (
          <code className="settings-code">{tzStatus.current.iana_guess}</code>
        ) : (
          <span className="settings-value">读取中…</span>
        )}
      </Row>
      {china && (
        <Note tone="warn">
          中国标准时间与常见 AI
          服务的出口地区不一致，容易被判定为环境异常。建议在下方按出口 IP 同步。
        </Note>
      )}
    </Card>
  );
}
