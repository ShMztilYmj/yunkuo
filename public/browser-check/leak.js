// 只采集当前浏览器原始证据；通过/风险/未知由后端判定。
export const ICE_SERVERS = [
  "local",
  "stun:stun.cloudflare.com:3478",
  "stun:stun.l.google.com:19302",
];

export async function probeWebRtc(
  Peer = globalThis.RTCPeerConnection,
  timeoutMs = 5000,
) {
  if (!Peer) return { supported: false, probes: [] };
  const probes = await Promise.all(
    ICE_SERVERS.map(
      (server) =>
        new Promise((resolve) => {
          let peer,
            channel,
            timer,
            done = false;
          const candidates = [];
          const finish = (outcome, error = null) => {
            if (done) return;
            done = true;
            clearTimeout(timer);
            try {
              channel?.close();
              peer?.close();
            } catch {}
            resolve({
              server,
              outcome,
              candidates,
              error: error ? String(error).slice(0, 180) : null,
            });
          };
          timer = setTimeout(
            () => finish("timeout", "ICE 收集超时"),
            timeoutMs,
          );
          try {
            peer = new Peer({
              iceServers: server === "local" ? [] : [{ urls: server }],
              iceCandidatePoolSize: 0,
            });
            peer.onicecandidate = (event) => {
              if (!event.candidate) {
                finish("complete");
                return;
              }
              const candidate = event.candidate;
              const parts = (candidate.candidate || "").trim().split(/\s+/);
              const address = candidate.address || parts[4];
              const kind = candidate.type || parts[parts.indexOf("typ") + 1];
              const protocol = (
                candidate.protocol ||
                parts[2] ||
                ""
              ).toLowerCase();
              const relatedAddress =
                candidate.relatedAddress ||
                (parts.includes("raddr")
                  ? parts[parts.indexOf("raddr") + 1]
                  : null);
              if (candidates.length >= 32) {
                finish("error", "ICE 候选超出数量限制");
                return;
              }
              if (
                address &&
                address.length <= 100 &&
                ["host", "srflx", "prflx", "relay"].includes(kind) &&
                ["udp", "tcp"].includes(protocol) &&
                candidates.length < 32 &&
                !candidates.some(
                  (c) =>
                    c.address === address &&
                    c.kind === kind &&
                    c.relatedAddress === relatedAddress,
                )
              ) {
                candidates.push({ address, kind, protocol, relatedAddress });
              }
            };
            channel = peer.createDataChannel("leak-probe");
            Promise.resolve(peer.createOffer())
              .then((offer) => {
                if (!done) return peer.setLocalDescription(offer);
              })
              .catch(() => finish("error", "ICE 探测失败"));
          } catch {
            finish("error", "WebRTC 初始化失败");
          }
        }),
    ),
  );
  return { supported: true, probes };
}

export async function probeDns(
  request,
  runId,
  fetcher = globalThis.fetch,
  wait = (ms) => new Promise((r) => setTimeout(r, ms)),
) {
  const challenge = await request(`dns/start/${runId}`, { method: "POST" });
  const hosts = challenge.hostnames;
  if (
    !Array.isArray(hosts) ||
    hosts.length < 4 ||
    hosts.length > 24 ||
    new Set(hosts).size !== hosts.length ||
    hosts.some(
      (host) =>
        typeof host !== "string" ||
        !/^[a-z0-9]{6,64}-[aq]\d{2}\.t\.dnsleak\.dev$/.test(host),
    ) ||
    !hosts.some((h) => /-a\d{2}\./.test(h)) ||
    !hosts.some((h) => /-q\d{2}\./.test(h))
  )
    throw new Error("DNS 测试域名不合法");
  // TLS 失败也可能已经完成 DNS 查询；是否真的到达权威端只看后端观察结果。
  for (let offset = 0; offset < hosts.length; offset += 6) {
    await Promise.allSettled(
      hosts.slice(offset, offset + 6).map((host) =>
        fetcher(`https://${host}/`, {
          method: "GET",
          mode: "cors",
          credentials: "omit",
          cache: "no-store",
          redirect: "error",
          referrerPolicy: "no-referrer",
          signal: AbortSignal.timeout(4000),
        }),
      ),
    );
  }
  let result;
  for (let attempt = 0; attempt < 3; attempt++) {
    await wait(1000);
    result = await request(`dns/result/${runId}`, { method: "POST" });
    if (result.observed >= hosts.length || result.state === "risk") break;
  }
  return result;
}
