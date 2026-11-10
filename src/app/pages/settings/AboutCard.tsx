import { ArrowUpRight, Info, ShieldCheck } from "lucide-react";
import { Link } from "react-router-dom";
import Card from "./Card";
import Note from "./Note";
import Row from "./Row";

/** 「关于」分类：版本、维护入口与授权说明。 */
export default function AboutCard({ appVersion }: { appVersion: string }) {
  return (
    <Card
      title="关于云阔"
      icon={<Info />}
      description="内置 mihomo、按出口 IP 同步时区，并在环境合格后才允许启动 Claude。"
      badge={`v${appVersion}`}
      badgeTone="idle"
    >
      <Row
        label="维护与迁移"
        hint="首次使用清单、环境诊断、配置导入导出与版本管理"
      >
        <Link className="settings-action" to="/maintenance">
          <ShieldCheck />
          打开维护
          <ArrowUpRight />
        </Link>
      </Row>
      <Row label="界面数据" hint="身份、配置与日志均保存在本机数据目录">
        <span className="settings-value">仅本地存储</span>
      </Row>
      <Row label="开源与致谢" wide>
        <dl className="settings-credits">
          <div>
            <dt>代理内核</dt>
            <dd>
              mihomo 以独立进程运行，由本软件管理配置、系统代理与 TUN 服务
            </dd>
          </div>
          <div>
            <dt>指纹浏览器</dt>
            <dd>Fingerprint Chromium，Chromium 及指纹分支许可随安装包提供</dd>
          </div>
          <div>
            <dt>身份能力</dt>
            <dd>参考 CCBaga（MIT）；环境与身份功能遵循对应项目的许可</dd>
          </div>
        </dl>
      </Row>
      <Note>
        本软件遵循
        GPL-3.0。网络检测会查询公网出口服务；时区修改需要系统权限，安装安全服务后
        TUN 开关免 UAC。相关许可全文随安装包提供。
      </Note>
    </Card>
  );
}
