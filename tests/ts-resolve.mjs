/** 在测试进程里注册 TS 解析钩子；只影响 `pnpm run test:frontend`。 */
import { register } from "node:module";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
register(pathToFileURL(path.join(here, "ts-resolve-hooks.mjs")).href);
