/** 「节点」页的组视图：内核固定生成 GLOBAL 选择器，名字不能改，界面按产品名展示并置顶。 */
export const GLOBAL_ID = "GLOBAL";
export const GLOBAL_LABEL = "全局节点";
export const GROUP_TYPES = [
  "Selector",
  "URLTest",
  "Fallback",
  "LoadBalance",
  "Relay",
];

/** 只取视图需要的字段，控制器响应里多出来的字段不影响。 */
export interface NodeView {
  protocol: string;
  selected: string | null;
  choices: string[];
}

export interface ProxyGroupView {
  /** 控制器里的真实组名，选择与测速必须用它。 */
  id: string;
  /** 界面展示名，只有 GLOBAL 会换名。 */
  label: string;
  type: string;
  now: string;
  /** 从该组往下的选择链，不含自身；GLOBAL 常指向另一个分组。 */
  chain: string[];
  nodes: string[];
  global: boolean;
}

/** 顺着 selected 找到真正出口，遇到环或缺失就停，避免死循环。 */
export function resolveChain(
  nodes: Record<string, NodeView>,
  start: string,
): string[] {
  const chain: string[] = [];
  const seen = new Set<string>();
  let current: string | null = start;
  while (current && !seen.has(current)) {
    seen.add(current);
    const entry: NodeView | undefined = nodes[current];
    if (!entry) break;
    chain.push(current);
    current = entry.selected;
  }
  return chain;
}

/** 只保留可切换的分组；GLOBAL 固定第一位，其余保持控制器顺序。 */
export function buildGroups(nodes: Record<string, NodeView>): ProxyGroupView[] {
  return Object.entries(nodes)
    .filter(
      ([, node]) =>
        GROUP_TYPES.includes(node.protocol) && node.choices.length > 0,
    )
    .map(([id, node]) => ({
      id,
      label: id === GLOBAL_ID ? GLOBAL_LABEL : id,
      type: node.protocol,
      now: node.selected ?? "",
      chain: resolveChain(nodes, id).slice(1),
      nodes: node.choices,
      global: id === GLOBAL_ID,
    }))
    .sort((a, b) => Number(b.global) - Number(a.global));
}
