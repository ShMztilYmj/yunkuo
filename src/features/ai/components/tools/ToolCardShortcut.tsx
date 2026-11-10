import { ArrowUpRight, FolderOpen, Loader2, ScanSearch } from "lucide-react";
import type { AiToolMeta } from "../../services/aiToolCards";

/** 卡片底部启动入口行：检测安装 / 自动定位 / 更换快捷方式 + 当前路径。 */
export default function ToolCardShortcut({
  meta,
  shortcut,
  configured,
  inspectBusy,
  autoBusy,
  onInspect,
  onAutoDetect,
  onPick,
}: {
  meta: AiToolMeta;
  shortcut: string;
  configured: boolean;
  inspectBusy: boolean;
  autoBusy: boolean;
  onInspect: () => void;
  onAutoDetect: () => void;
  onPick: () => void;
}) {
  return (
    <div className="tool-card-shortcut">
      <div className="tool-card-shortcut-heading">
        <span>
          <FolderOpen size={12} />
          启动入口
        </span>
        <div>
          <button type="button" disabled={inspectBusy} onClick={onInspect}>
            检测安装
          </button>
          <button
            type="button"
            disabled={inspectBusy}
            aria-label={`自动检测 ${meta.label} 的安装位置并选择快捷方式`}
            data-tooltip="自动检测安装位置并选用可执行入口"
            onClick={onAutoDetect}
          >
            {autoBusy ? (
              <Loader2 size={11} className="animate-spin" />
            ) : (
              <ScanSearch size={11} />
            )}
            自动定位
          </button>
          <button type="button" onClick={onPick}>
            {configured ? "更换" : "选择快捷方式"}
            <ArrowUpRight size={11} />
          </button>
        </div>
      </div>
      <p
        data-tooltip={
          configured ? shortcut : `选择指向 ${meta.label} 的快捷方式`
        }
      >
        {configured ? shortcut : "尚未选择启动快捷方式"}
      </p>
    </div>
  );
}
