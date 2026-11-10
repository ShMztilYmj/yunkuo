import PageTabs from "../../../shared/ui/PageTabs";
import { PROXY_SUB } from "../navigation";

/** 代理子页导航：交给共用页签渲染，样式与位置由页头统一决定。 */
export default function ProxyTabs() {
  return <PageTabs label="代理页面" tabs={PROXY_SUB} />;
}
