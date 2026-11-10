import { Server, Trash2, Wrench } from "lucide-react";
import { useState } from "react";
import { useAsyncAction } from "../../../shared/hooks/useAsyncAction";
import { api } from "../../../services/api";
import Card from "./Card";
import Note from "./Note";
import Row from "./Row";
import ServiceStateBadge from "./ServiceStateBadge";
import { useProxyStore } from "../../../features/proxy/state";

/** TUN 服务模式（免 UAC）：安装 / 卸载系统服务，服务状态由 ServiceStateBadge 轮询。 */
export default function TunServiceCard({
  message,
  onMessage,
}: {
  message: string | null;
  onMessage: (value: string | null) => void;
}) {
  const { busy, run } = useAsyncAction();
  const [revision, setRevision] = useState(0);
  return (
    <Card
      title="TUN 服务模式"
      icon={<Server />}
      description="安装系统服务后由 SYSTEM 权限代管内核，日常切换 TUN 不再弹出 UAC。"
      badge="免 UAC"
      badgeTone="idle"
    >
      <Row
        label="服务状态"
        wide
        hint="安装一次即可长期使用；升级后可能需要修复"
      >
        <ServiceStateBadge key={revision} />
      </Row>
      <Row label="服务操作" hint="安装与卸载都会请求一次系统授权">
        <div className="settings-actions">
          <button
            type="button"
            disabled={busy}
            aria-busy={busy}
            onClick={() =>
              void run(async () => {
                onMessage("正在安装服务（如弹出 UAC 请允许）…");
                try {
                  await api.serviceInstall();
                  onMessage("TUN 服务安装成功，此后 TUN 开关免 UAC");
                } catch {
                  // 失败详情由后端写进右上角消息中心；卡片只给一行结论。
                  onMessage("安装失败，详情见右上角消息中心");
                  await useProxyStore
                    .getState()
                    .refreshErrors()
                    .catch(() => undefined);
                } finally {
                  setRevision((value) => value + 1);
                  await useProxyStore
                    .getState()
                    .refreshCore(true)
                    .catch(() => undefined);
                }
              })
            }
            className="settings-action"
            data-variant="primary"
          >
            <Wrench />
            安装或修复服务
          </button>
          <button
            type="button"
            disabled={busy}
            aria-busy={busy}
            onClick={() =>
              void run(async () => {
                onMessage("正在卸载服务（如弹出 UAC 请允许）…");
                try {
                  await api.serviceUninstall();
                  onMessage("TUN 服务已卸载");
                } catch {
                  onMessage("卸载失败，详情见右上角消息中心");
                  await useProxyStore
                    .getState()
                    .refreshErrors()
                    .catch(() => undefined);
                } finally {
                  setRevision((value) => value + 1);
                  await useProxyStore
                    .getState()
                    .refreshCore(true)
                    .catch(() => undefined);
                }
              })
            }
            className="settings-action"
          >
            <Trash2 />
            卸载服务
          </button>
        </div>
      </Row>
      {message && <Note role="status">{message}</Note>}
    </Card>
  );
}
