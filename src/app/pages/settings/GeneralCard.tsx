import { FolderOpen, Palette, Power } from "lucide-react";
import { api, type AppConfig } from "../../../services/api";
import Card from "./Card";
import DefaultBrowserRow from "./DefaultBrowserRow";
import Row from "./Row";
import Switch from "./Switch";
import ThemeSegmented from "./ThemeSegmented";
import type { SettingsUpdater } from "./types";

/** 通用：界面外观、默认浏览器联动，以及登录后的启动与关闭行为。 */
export default function GeneralCard({
  config,
  onUpdate,
}: {
  config: AppConfig;
  onUpdate: SettingsUpdater;
}) {
  return (
    <>
      <Card
        title="外观"
        icon={<Palette />}
        description="主题会立即应用到主窗口、托盘与 HUD，不需要重启。"
      >
        <Row label="界面主题" hint="跟随系统时随 Windows 自动切换">
          <ThemeSegmented />
        </Row>{" "}
        <Row
          label="默认浏览器联动"
          hint="全局代理与 TUN 运行时，外部链接交给受控指纹浏览器"
          wide
        >
          <DefaultBrowserRow />
        </Row>
      </Card>

      <Card
        title="启动与窗口"
        icon={<Power />}
        description="决定登录 Windows 后的启动方式，以及点关闭按钮时的行为。"
      >
        <Row label="开机自启" hint="登录后自动启动云阔">
          <Switch
            v={config.enable_auto_launch}
            label="切换开机自启"
            on={(b) => onUpdate("enable_auto_launch", b)}
          />
        </Row>
        <Row label="静默启动" hint="启动后只保留托盘图标，不弹出主窗口">
          <Switch
            v={config.enable_silent_start}
            label="切换静默启动"
            on={(b) => onUpdate("enable_silent_start", b)}
          />
        </Row>
        <Row
          label="关闭时最小化到托盘"
          hint="点关闭按钮不退出，代理、时区与服务继续运行"
        >
          <Switch
            v={config.close_to_tray}
            label="切换关闭时最小化到托盘"
            on={(b) => onUpdate("close_to_tray", b)}
          />
        </Row>
        <Row label="数据目录" hint="配置、日志、订阅缓存与浏览器资料都在本机">
          <button
            type="button"
            onClick={() => void api.openDataDir()}
            className="settings-action"
          >
            <FolderOpen />
            打开目录
          </button>
        </Row>
      </Card>
    </>
  );
}
