// 前端错误归一化：Rust 命令的 { code, message } 必须既保留旧文案展示，又能按 code 分支。
import assert from "node:assert/strict";
import test from "node:test";

const { ApiError, errorCode, errorMessage, normalizeError } = await import(
  "../src/services/api/invoke.ts"
);

test("对象错误转成 ApiError，String() 仍然只得到文案", () => {
  const error = normalizeError({ code: "core.restart_failed", message: "内核未响应" });
  assert.ok(error instanceof ApiError);
  assert.equal(error.code, "core.restart_failed");
  assert.equal(String(error), "内核未响应");
  assert.equal(errorMessage(error), "内核未响应");
  assert.equal(errorCode(error), "core.restart_failed");
});

test("字符串错误保持原样，旧界面的展示不变", () => {
  const error = normalizeError("服务未安装");
  assert.equal(typeof error, "string");
  assert.equal(errorMessage(error), "服务未安装");
  assert.equal(errorCode(error), null);
});

test("缺少 code 的对象只归类为 unknown，不丢文案", () => {
  const error = normalizeError({ message: "未知失败" });
  assert.equal(errorCode(error), "unknown");
  assert.equal(String(error), "未知失败");
});
