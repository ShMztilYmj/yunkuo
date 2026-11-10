/**
 * 测试用解析钩子：把 `./foo` 补成 `./foo.ts`。
 *
 * 源码里统一用无扩展名的相对导入（Vite 负责解析），但 Node 的 ESM 加载器
 * 只认带扩展名的路径。这里只在测试进程里补一次，源码保持原样。
 */
export async function resolve(specifier, context, next) {
  if (specifier.startsWith(".") && !/\.[cm]?[jt]sx?$/i.test(specifier)) {
    try {
      return await next(specifier, context);
    } catch {
      return next(`${specifier}.ts`, context);
    }
  }
  return next(specifier, context);
}
