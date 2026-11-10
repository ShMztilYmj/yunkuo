import { Clock, Loader2, Undo2 } from "lucide-react";
import type { AppConfig, TzEntry, TzStatus } from "../../../services/api";
import Card from "./Card";
import Note from "./Note";
import Row from "./Row";
import Switch from "./Switch";
import type { SettingsUpdater } from "./types";

/** 选择器只用到这三个字段，后端条目与内置条目都能直接塞进同一个列表。 */
type TzOption = Pick<TzEntry, "key_name" | "display" | "bias">;

/**
 * 内置常用时区：`get_timezone_status` 拿不到系统时区表时兜底。
 * 注册表 Time Zones 键被系统精简工具清空、后端枚举异常、旧包没带 `available`
 * 字段时，手动指定不至于只剩一个空下拉；`key_name` 都是系统自带的时区键。
 */
const FALLBACK_TZ: readonly TzOption[] = [
  {
    key_name: "Hawaiian Standard Time",
    display: "(UTC-10:00) 夏威夷",
    bias: 600,
  },
  {
    key_name: "Alaskan Standard Time",
    display: "(UTC-09:00) 阿拉斯加",
    bias: 540,
  },
  {
    key_name: "Pacific Standard Time",
    display: "(UTC-08:00) 太平洋时间（美国和加拿大）",
    bias: 480,
  },
  {
    key_name: "Mountain Standard Time",
    display: "(UTC-07:00) 山地时间（美国和加拿大）",
    bias: 420,
  },
  {
    key_name: "Central Standard Time",
    display: "(UTC-06:00) 中部时间（美国和加拿大）",
    bias: 360,
  },
  {
    key_name: "Eastern Standard Time",
    display: "(UTC-05:00) 东部时间（美国和加拿大）",
    bias: 300,
  },
  {
    key_name: "Atlantic Standard Time",
    display: "(UTC-04:00) 大西洋时间（加拿大）",
    bias: 240,
  },
  {
    key_name: "SA Pacific Standard Time",
    display: "(UTC-05:00) 波哥大，利马，基多",
    bias: 300,
  },
  {
    key_name: "E. South America Standard Time",
    display: "(UTC-03:00) 巴西利亚，圣保罗",
    bias: 180,
  },
  {
    key_name: "Argentina Standard Time",
    display: "(UTC-03:00) 布宜诺斯艾利斯",
    bias: 180,
  },
  {
    key_name: "GMT Standard Time",
    display: "(UTC+00:00) 都柏林，爱丁堡，里斯本，伦敦",
    bias: 0,
  },
  {
    key_name: "W. Europe Standard Time",
    display: "(UTC+01:00) 阿姆斯特丹，柏林，伯尔尼，罗马，斯德哥尔摩，维也纳",
    bias: -60,
  },
  {
    key_name: "Romance Standard Time",
    display: "(UTC+01:00) 布鲁塞尔，哥本哈根，马德里，巴黎",
    bias: -60,
  },
  {
    key_name: "Central European Standard Time",
    display: "(UTC+01:00) 萨拉热窝，斯科普里，华沙，萨格勒布",
    bias: -60,
  },
  {
    key_name: "GTB Standard Time",
    display: "(UTC+02:00) 雅典，布加勒斯特",
    bias: -120,
  },
  {
    key_name: "FLE Standard Time",
    display: "(UTC+02:00) 赫尔辛基，基辅，里加，索菲亚，塔林，维尔纽斯",
    bias: -120,
  },
  {
    key_name: "Israel Standard Time",
    display: "(UTC+02:00) 耶路撒冷",
    bias: -120,
  },
  {
    key_name: "South Africa Standard Time",
    display: "(UTC+02:00) 比勒陀利亚，哈拉雷",
    bias: -120,
  },
  {
    key_name: "Russian Standard Time",
    display: "(UTC+03:00) 莫斯科，圣彼得堡，伏尔加格勒",
    bias: -180,
  },
  {
    key_name: "Turkey Standard Time",
    display: "(UTC+03:00) 伊斯坦布尔",
    bias: -180,
  },
  { key_name: "Iran Standard Time", display: "(UTC+03:30) 德黑兰", bias: -210 },
  {
    key_name: "Arabian Standard Time",
    display: "(UTC+04:00) 阿布扎比，马斯喀特",
    bias: -240,
  },
  {
    key_name: "Pakistan Standard Time",
    display: "(UTC+05:00) 伊斯兰堡，卡拉奇",
    bias: -300,
  },
  {
    key_name: "India Standard Time",
    display: "(UTC+05:30) 金奈，加尔各答，孟买，新德里",
    bias: -330,
  },
  {
    key_name: "Bangladesh Standard Time",
    display: "(UTC+06:00) 达卡",
    bias: -360,
  },
  {
    key_name: "SE Asia Standard Time",
    display: "(UTC+07:00) 曼谷，河内，雅加达",
    bias: -420,
  },
  {
    key_name: "China Standard Time",
    display: "(UTC+08:00) 北京，重庆，香港特别行政区，乌鲁木齐",
    bias: -480,
  },
  {
    key_name: "Singapore Standard Time",
    display: "(UTC+08:00) 吉隆坡，新加坡",
    bias: -480,
  },
  { key_name: "Taipei Standard Time", display: "(UTC+08:00) 台北", bias: -480 },
  {
    key_name: "Tokyo Standard Time",
    display: "(UTC+09:00) 大阪，札幌，东京",
    bias: -540,
  },
  { key_name: "Korea Standard Time", display: "(UTC+09:00) 首尔", bias: -540 },
  {
    key_name: "Cen. Australia Standard Time",
    display: "(UTC+09:30) 阿德莱德",
    bias: -570,
  },
  {
    key_name: "AUS Eastern Standard Time",
    display: "(UTC+10:00) 堪培拉，墨尔本，悉尼",
    bias: -600,
  },
  {
    key_name: "E. Australia Standard Time",
    display: "(UTC+10:00) 布里斯班",
    bias: -600,
  },
  {
    key_name: "West Pacific Standard Time",
    display: "(UTC+10:00) 关岛，莫尔斯比港",
    bias: -600,
  },
  {
    key_name: "New Zealand Standard Time",
    display: "(UTC+12:00) 奥克兰，惠灵顿",
    bias: -720,
  },
];

/** 按出口 IP 自动同步：同步 / 还原按钮、备份与最近同步信息、手动指定时区。 */
export default function TimezoneSyncCard({
  config,
  onUpdate,
  tzStatus,
  tzBusy,
  tzMsg,
  tzError,
  manualTz,
  onManualTzChange,
  onSync,
  onRestore,
  onApplyManual,
}: {
  config: AppConfig;
  onUpdate: SettingsUpdater;
  tzStatus: TzStatus | null;
  tzBusy: boolean;
  tzMsg: string | null;
  /** 读取时区状态失败的原因（与操作结果分开，避免互相覆盖） */
  tzError: string | null;
  manualTz: string;
  onManualTzChange: (value: string) => void;
  onSync: () => void;
  onRestore: () => void;
  onApplyManual: () => void;
}) {
  const available = tzStatus?.available ?? [];
  // 首次加载（还没拿到结果也没报错）不显示兜底，避免"读取中"时闪一下内置表。
  const usingFallback =
    available.length === 0 && (tzStatus !== null || tzError !== null);
  const options: readonly TzOption[] = [
    ...(usingFallback ? FALLBACK_TZ : available),
  ].sort((a, b) => a.bias - b.bias);

  return (
    <Card
      title="按出口 IP 同步"
      icon={<Clock />}
      description="先读取当前出口 IP 的归属地，再把系统时区切换到对应地区；关闭代理时可还原。"
      badge={config.tz_auto_sync ? "自动同步" : "仅手动"}
      badgeTone={config.tz_auto_sync ? "ok" : "idle"}
    >
      <Row
        label="内核启动后自动同步"
        hint="内核启动或切换节点后重新核对出口时区"
      >
        <Switch
          v={config.tz_auto_sync}
          label="切换内核启动后自动同步时区"
          on={(b) => onUpdate("tz_auto_sync", b)}
        />
      </Row>
      <Row label="关闭系统代理时还原" hint="停止接管后恢复同步前的原始时区">
        <Switch
          v={config.tz_restore_on_proxy_off}
          label="切换关闭代理时还原时区"
          on={(b) => onUpdate("tz_restore_on_proxy_off", b)}
        />
      </Row>
      <Row label="立即操作" hint="同步会修改系统时区，需要相应权限">
        <div className="settings-actions">
          <button
            type="button"
            onClick={onSync}
            disabled={tzBusy}
            aria-busy={tzBusy}
            className="settings-action"
            data-variant="primary"
          >
            {tzBusy ? <Loader2 className="animate-spin" /> : <Clock />}
            {tzBusy ? "处理中…" : "按 IP 同步时区"}
          </button>
          <button
            type="button"
            onClick={onRestore}
            disabled={tzBusy || !tzStatus?.backup}
            className="settings-action"
          >
            <Undo2 />
            还原原时区
          </button>
        </div>
      </Row>
      {tzStatus?.backup && (
        <Row label="原始时区备份" hint="还原时使用">
          <span className="settings-value">
            {tzStatus.backup.key_name}
            <small> · {tzStatus.backup.backed_up_at}</small>
          </span>
        </Row>
      )}
      {tzStatus?.last_sync && (
        <Row label="最近同步" hint="出口 IP 对应的时区记录">
          <span className="settings-value">
            {tzStatus.last_sync.ip}（{tzStatus.last_sync.country}）→{" "}
            {tzStatus.last_sync.windows_tz}
            <small> · {tzStatus.last_sync.at}</small>
          </span>
        </Row>
      )}
      <Row
        label="手动指定"
        hint={
          usingFallback
            ? "读不到系统时区表，这里列出的是内置常用时区"
            : "按 IP 同步不可用时，可直接选择 Windows 时区"
        }
        wide
      >
        <div className="settings-actions settings-field settings-field--grow">
          <select
            className="settings-select settings-field--grow"
            aria-label="手动指定 Windows 时区"
            value={manualTz}
            onChange={(e) => onManualTzChange(e.target.value)}
          >
            <option value="">选择 Windows 时区…</option>
            {options.map((t) => (
              <option key={t.key_name} value={t.key_name}>
                {t.display || t.key_name}
              </option>
            ))}
          </select>
          <button
            type="button"
            onClick={onApplyManual}
            disabled={tzBusy || !manualTz}
            className="settings-action"
          >
            应用
          </button>
        </div>
      </Row>
      {usingFallback && (
        <Note role="status">
          <span>
            读不到系统时区表，当前使用内置常用时区；应用时仍按系统时区库校验。
          </span>
        </Note>
      )}
      {tzError && (
        <Note tone="error" role="alert">
          <span>{tzError}</span>
        </Note>
      )}
      {tzMsg && (
        <Note role="status">
          <span>{tzMsg}</span>
        </Note>
      )}
    </Card>
  );
}
