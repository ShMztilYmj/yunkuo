import {
  FONTS_CN_VENDOR,
  FONTS_SC,
  FONTS_TC,
  emojiFlagProbe,
  isFontAvailable,
} from "../signals/web";

/**
 * 生成在指纹浏览器里执行的采集脚本。
 *
 * 主窗口的 WebView2 与指纹浏览器是两个进程，只能把脚本塞进本地页面下发。
 * 字体探针与 Emoji 探针直接序列化 `signals/web.ts` 的函数体，保证两边跑的
 * 是同一段逻辑；计分仍留在主程序里（`selfcheck/report.ts`），不重复一份规则。
 *
 * 脚本里只用 ES2017 语法、不出现反引号与 `</script`，避免把宿主 HTML 截断。
 */
export function buildSelfcheckScript(): string {
  return `(function () {
  var FONTS_SC = ${JSON.stringify(FONTS_SC)};
  var FONTS_TC = ${JSON.stringify(FONTS_TC)};
  var FONTS_CN_VENDOR = ${JSON.stringify(FONTS_CN_VENDOR)};
  var isFontAvailable = ${String(isFontAvailable)};
  var emojiFlagProbe = ${String(emojiFlagProbe)};

  function text(value) { return typeof value === "string" ? value : ""; }
  function int(value) { return typeof value === "number" && isFinite(value) ? value : null; }

  var status = document.getElementById("selfcheck-status");
  var note = document.getElementById("selfcheck-note");
  function show(title, detail) {
    if (status) status.textContent = title;
    if (note) note.textContent = detail;
  }

  var report;
  try {
    var canvas = document.createElement("canvas");
    var ctx = canvas.getContext("2d");
    function probe(list) {
      if (!ctx) return [];
      return list.filter(function (font) { return isFontAvailable(font, ctx); });
    }
    var resolved = {};
    try { resolved = Intl.DateTimeFormat().resolvedOptions() || {}; } catch (error) { resolved = {}; }
    var brands = "";
    try {
      var data = navigator.userAgentData;
      brands = ((data && data.brands) || []).map(function (item) { return item.brand; }).join(" ");
    } catch (error) { brands = ""; }
    var webgl = "";
    try {
      var gl = document.createElement("canvas").getContext("webgl");
      var ext = gl && gl.getExtension("WEBGL_debug_renderer_info");
      webgl = ext ? text(gl.getParameter(ext.UNMASKED_RENDERER_WEBGL)) : "";
    } catch (error) { webgl = ""; }

    report = {
      at: Date.now(),
      ua: text(navigator.userAgent),
      brands: brands,
      platform: text(navigator.platform),
      languages: Array.prototype.slice.call(navigator.languages || [navigator.language || ""]),
      timezone: text(resolved.timeZone),
      locale: text(resolved.locale),
      offset: new Date().getTimezoneOffset(),
      hardwareConcurrency: int(navigator.hardwareConcurrency),
      deviceMemory: int(navigator.deviceMemory),
      screen: [screen.width, screen.height, window.devicePixelRatio || 1],
      fonts: {
        sc: probe(FONTS_SC),
        tc: probe(FONTS_TC),
        vendor: probe(FONTS_CN_VENDOR)
      },
      emojiFlag: emojiFlagProbe(),
      webgl: webgl,
      webdriver: navigator.webdriver === true
    };
  } catch (error) {
    show("采集失败", String((error && error.message) || error));
    return;
  }

  var ticket = new URLSearchParams(location.search).get("t") || "";
  show("正在回传…", report.languages.join(", ") + " · " + report.timezone);
  fetch("/report?t=" + encodeURIComponent(ticket), {
    method: "POST",
    headers: { "Content-Type": "text/plain" },
    body: JSON.stringify(report)
  })
    .then(function (response) {
      if (!response.ok) throw new Error("HTTP " + response.status);
      show("自检完成", "结果已回传，可以关闭此页面。");
      setTimeout(function () {
        try { window.close(); } catch (error) { /* 命令行打开的标签页不允许脚本关闭 */ }
      }, 900);
    })
    .catch(function (error) {
      show("回传失败", String((error && error.message) || error));
    });
})();`;
}
