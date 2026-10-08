<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";

  interface HostEntry { addr: string }
  interface ProbeResult {
    addr: string; reachable: boolean; version: string; started_at: number;
    os: string; arch: string; pubkey_fingerprint: string; error: string;
  }
  interface FileEntry { name: string; size: number }
  interface DistInfo {
    ok: boolean; version: string; main: string; files: FileEntry[];
    total_bytes: number; package_sha256: string; error: string;
  }
  interface HostState {
    addr: string;
    probing: boolean;
    reachable: boolean;
    version: string;
    os: string;
    arch: string;
    lastError: string;
    lastResult: string;   // 最近一次更新结果描述
    lastOk: boolean;
    updating: boolean;
    selected: boolean;
  }
  interface LogLine { addr: string; line: string }

  // ---- 更新包配置 ----
  let distDir = "";
  let mainExe = "";
  let overwriteConfig = false;
  let token = "";
  let signKey = "";
  let timeoutSecs = 120;
  let dist: DistInfo | null = null;
  let distError = "";

  // ---- 主机 ----
  let hosts: HostState[] = [];
  let newAddr = "";
  let logs: { addr: string; line: string; at: string }[] = [];
  let pushing = false;
  let logEl: HTMLElement;

  function fmtSize(n: number): string {
    if (n > 1024 * 1024) return (n / 1024 / 1024).toFixed(1) + " MB";
    if (n > 1024) return (n / 1024).toFixed(1) + " KB";
    return n + " B";
  }

  function now(): string {
    return new Date().toLocaleTimeString("zh-CN", { hour12: false });
  }

  function addrColor(addr: string): string {
    let h = 0;
    for (const c of addr) h = (h * 31 + c.charCodeAt(0)) % 360;
    return `hsl(${h}, 65%, 70%)`;
  }

  function findHost(addr: string): HostState | undefined {
    return hosts.find((h) => h.addr === addr);
  }

  // ---- 主机列表持久化 ----
  async function loadHosts() {
    const saved = await invoke<HostEntry[]>("load_hosts");
    hosts = saved.map((e) => freshHost(e.addr));
  }

  function freshHost(addr: string): HostState {
    return {
      addr, probing: false, reachable: false, version: "", os: "", arch: "",
      lastError: "", lastResult: "", lastOk: false, updating: false, selected: false,
    };
  }

  async function persistHosts() {
    const entries: HostEntry[] = hosts.map((h) => ({ addr: h.addr }));
    await invoke("save_hosts", { hosts: entries });
  }

  async function addHost() {
    const a = newAddr.trim();
    if (!a || hosts.some((h) => h.addr === a)) return;
    hosts = [...hosts, freshHost(a)];
    newAddr = "";
    await persistHosts();
    probeOne(a);
  }

  async function removeHost(addr: string) {
    hosts = hosts.filter((h) => h.addr !== addr);
    await persistHosts();
  }

  // ---- 探测 ----
  async function probeOne(addr: string) {
    const h = findHost(addr);
    if (!h || h.probing) return;
    h.probing = true;
    hosts = hosts;
    try {
      const r = await invoke<ProbeResult>("cmd_probe", { addr, token: token || null });
      h.reachable = r.reachable;
      h.version = r.version;
      h.os = r.os;
      h.arch = r.arch;
      h.lastError = r.reachable ? "" : r.error;
    } catch (e) {
      h.reachable = false;
      h.lastError = String(e);
    }
    h.probing = false;
    hosts = hosts;
  }

  async function probeAll() {
    await Promise.all(hosts.map((h) => probeOne(h.addr)));
  }

  // ---- dist 预览 ----
  async function refreshDist() {
    distError = "";
    dist = null;
    if (!distDir.trim()) return;
    try {
      const r = await invoke<DistInfo>("cmd_dist_info", {
        distDir: distDir.trim(),
        main: mainExe.trim() || null,
        overwriteConfig,
      });
      if (r.ok) dist = r;
      else distError = r.error;
    } catch (e) {
      distError = String(e);
    }
  }

  // ---- 更新 ----
  async function updateHosts(targets: string[]) {
    if (pushing || targets.length === 0 || !dist?.ok) return;
    pushing = true;
    hosts = hosts.map((h) => (targets.includes(h.addr) ? { ...h, updating: true } : h));
    const jobs = targets.map(async (addr) => {
      try {
        const r = await invoke<any>("cmd_update_host", {
          addr,
          distDir: distDir.trim(),
          main: mainExe.trim() || null,
          token: token || null,
          signKey: signKey.trim() || null,
          overwriteConfig,
          timeoutSecs,
        });
        const h = findHost(addr);
        if (h) {
          h.updating = false;
          if (r.ok) {
            h.lastOk = true;
            h.lastResult = `${r.old_version} → ${r.new_version}`;
            h.version = r.new_version;
            probeOne(addr);
          } else {
            h.lastOk = false;
            h.lastResult = r.detail;
          }
        }
      } catch (e) {
        const h = findHost(addr);
        if (h) {
          h.updating = false;
          h.lastOk = false;
          h.lastResult = String(e);
        }
      }
      hosts = hosts;
    });
    await Promise.all(jobs);
    pushing = false;
    hosts = hosts;
  }

  const updateSelected = () => updateHosts(hosts.filter((h) => h.selected).map((h) => h.addr));
  const updateAll = () => updateHosts(hosts.map((h) => h.addr));
  const retryFailed = () =>
    updateHosts(hosts.filter((h) => h.lastResult && !h.lastOk).map((h) => h.addr));

  onMount(async () => {
    await loadHosts();
    await listen<LogLine>("update-log", (e) => {
      const { addr, line } = e.payload;
      logs = [...logs.slice(-500), { addr, line, at: now() }];
      if (logEl) logEl.scrollTop = logEl.scrollHeight;
    });
    hosts.forEach((h) => probeOne(h.addr));
  });
</script>

<div class="panel">
  <h2>更新包</h2>
  <div class="row">
    <input type="text" class="grow" placeholder="新版 dist 目录,如 D:\dist" bind:value={distDir} on:change={refreshDist} />
    <input type="text" style="width: 130px" placeholder="主程序(可选)" bind:value={mainExe} on:change={refreshDist} />
    <label class="hint"><input type="checkbox" bind:checked={overwriteConfig} on:change={refreshDist} /> 覆盖 config.json</label>
    <button on:click={refreshDist}>重新计算</button>
  </div>
  <div class="row" style="margin-top: 8px">
    <input type="password" style="width: 200px" placeholder="Token(可选)" bind:value={token} />
    <input type="password" style="width: 200px" placeholder="签名私钥 hex(可选)" bind:value={signKey} />
    <span class="hint">轮询超时</span>
    <input type="text" style="width: 60px" bind:value={timeoutSecs} />
    <span class="hint">秒</span>
  </div>
  {#if dist}
    <div class="row" style="margin-top: 8px">
      <span class="badge on">v{dist.version}</span>
      <span class="meta">主程序 {dist.main} · {dist.files.length} 个文件 · {fmtSize(dist.total_bytes)}</span>
      <span class="sha grow">sha256 = {dist.package_sha256}</span>
    </div>
    <div class="meta">{dist.files.map((f) => f.name).join("  ")}</div>
  {:else if distError}
    <div class="meta err" style="margin-top: 8px">{distError}</div>
  {/if}
</div>

<div class="panel">
  <div class="row">
    <h2 class="grow">主机({hosts.length})</h2>
    <input type="text" style="width: 220px" placeholder="IP[:7600]" bind:value={newAddr} on:keydown={(e) => e.key === "Enter" && addHost()} />
    <button on:click={addHost}>添加</button>
    <button on:click={probeAll} disabled={pushing}>探测全部</button>
    <button on:click={updateSelected} disabled={pushing || !dist?.ok}>更新选中</button>
    <button on:click={retryFailed} disabled={pushing || !dist?.ok}>重试失败</button>
    <button class="primary" on:click={updateAll} disabled={pushing || !dist?.ok}>全部更新</button>
  </div>
  <table>
    <thead>
      <tr>
        <th style="width: 30px"></th>
        <th>地址</th>
        <th style="width: 70px">状态</th>
        <th style="width: 90px">版本</th>
        <th style="width: 140px">平台</th>
        <th>最近更新</th>
        <th style="width: 130px">操作</th>
      </tr>
    </thead>
    <tbody>
      {#each hosts as h (h.addr)}
        <tr>
          <td><input type="checkbox" bind:checked={h.selected} disabled={h.updating} /></td>
          <td><b>{h.addr}</b></td>
          <td>
            {#if h.probing || h.updating}
              <span class="badge idle">{h.updating ? "更新中…" : "探测中…"}</span>
            {:else if h.reachable}
              <span class="badge on">在线</span>
            {:else}
              <span class="badge off">离线</span>
            {/if}
          </td>
          <td>{h.version || "—"}</td>
          <td class="meta">{h.os ? `${h.os}/${h.arch}` : "—"}</td>
          <td class={h.lastOk ? "okline" : "err"}>
            {h.lastResult || h.lastError || "—"}
          </td>
          <td>
            <button on:click={() => probeOne(h.addr)} disabled={h.updating}>探测</button>
            <button class="primary" on:click={() => updateHosts([h.addr])} disabled={pushing || h.updating || !dist?.ok}>更新</button>
            <button class="danger" on:click={() => removeHost(h.addr)} disabled={h.updating}>删</button>
          </td>
        </tr>
      {:else}
        <tr><td colspan="7" class="meta" style="text-align: center; padding: 18px">添加目标主机地址开始(目标需已集成 ghost-updater)</td></tr>
      {/each}
    </tbody>
  </table>
</div>

<div class="row hint">
  <span>操作日志(实时)</span>
  <span class="grow"></span>
  <button on:click={() => (logs = [])}>清空</button>
</div>
<div id="log" bind:this={logEl}>
  {#each logs as l}
    <div class="log-line">
      <span class="meta">{l.at}</span>
      <span class="tag" style="color: {addrColor(l.addr)}">[{l.addr}]</span>
      <span class={l.line.includes("失败") || l.line.includes("错误") ? "err" : l.line.includes("成功") ? "okline" : ""}>{l.line}</span>
    </div>
  {:else}
    <div class="log-line meta">等待操作…</div>
  {/each}
</div>
