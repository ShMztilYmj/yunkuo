import { KeyRound } from "lucide-react";
import type { AppConfig } from "../../../services/api";
import Card from "./Card";
import Note from "./Note";
import Row from "./Row";
import type { SettingsUpdater } from "./types";

/**
 * 出口情报数据源：ipapi.is 免费 Key（1,000 次/天）。
 * 留空时只用本地公开源（ip-api / proxycheck / RIPEstat / PeeringDB / DNSBL），
 * 填入后多一路独立的机房 / VPN / 代理 / Tor / 爬虫旗标与滥用分。
 */
export default function IntelKeyCard({
  config,
  onUpdate,
}: {
  config: AppConfig;
  onUpdate: SettingsUpdater;
}) {
  const key = config.ipapiis_key ?? "";
  return (
    <Card
      title="IP 情报数据源"
      icon={<KeyRound />}
      description="可选接入 ipapi.is，为出口 IP 补充独立的风险旗标与滥用分。"
      badge={key ? "已配置 Key" : "仅公开源"}
      badgeTone={key ? "ok" : "idle"}
    >
      <Row
        label="ipapi.is API Key"
        hint="免费层 1,000 次/天；留空则只走本地公开源"
        wide
      >
        <input
          className="settings-input is-mono"
          aria-label="ipapi.is API Key"
          key={`ipapiis-${key}`}
          defaultValue={key}
          placeholder="在 ipapi.is 控制台的 API Credentials 里复制"
          spellCheck={false}
          autoComplete="off"
          onBlur={(e) => {
            const next = e.target.value.trim();
            if (next !== key) onUpdate("ipapiis_key", next);
          }}
        />
      </Row>
      <Note>
        检出口卡片会显示每个源的结论。Key 只保存在本地配置；若线路到不了
        api.ipapi.is，风险深度检测会标注「未连通」，不会静默当作通过。
      </Note>
    </Card>
  );
}
