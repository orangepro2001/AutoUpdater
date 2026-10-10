<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";

  interface HostEntry { addr: string }
  // 一个项目 = 更新包配置 + 默认端口 + 目标主机列表,整档保存(apps.json)
  interface AppProfile {
    name: string; port: number; hosts: HostEntry[];
    dist_dir: string; main_exe: string; overwrite_config: boolean;
    token: string; sign_key: string; timeout_secs: number;
  }
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

  // ---- 更新包配置直接读写当前项目(apps[activeIdx]),切换项目即随档切换 ----
  let dist: DistInfo | null = null;
  let distError = "";

  // ---- 项目配置(每套 = 更新包设置 + 项目名 + 默认端口 + 机器列表) ----
  function blankProject(name: string, port = 7600): AppProfile {
    return {
      name, port, hosts: [],
      dist_dir: "", main_exe: "", overwrite_config: false,
      token: "", sign_key: "", timeout_secs: 120,
    };
  }
  let apps: AppProfile[] = [blankProject("项目A")];
  let activeIdx = 0;
  $: active = apps[activeIdx];
  // 函数内一律用 currentProject():active 是 $: 派生,同步改 activeIdx 后尚未刷新,
  // switchApp 里紧接着读 active 会拿到上一个项目。
  const currentProject = (): AppProfile | undefined => apps[activeIdx];

  // ---- 当前应用的主机工作列表 ----
  let hosts: HostState[] = [];
  let newIp = "";
  let newPort = 7600;
  let batchOpen = false;
  let batchText = "";
  let importFrom = "";
  let addingApp = false;
  let newName = "";
  let renaming = false;
  let renameText = "";
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

  function pushLog(addr: string, line: string) {
    logs = [...logs.slice(-500), { addr, line, at: now() }];
    if (logEl) logEl.scrollTop = logEl.scrollHeight;
  }

  function freshHost(addr: string): HostState {
    return {
      addr, probing: false, reachable: false, version: "", os: "", arch: "",
      lastError: "", lastResult: "", lastOk: false, updating: false, selected: false,
    };
  }

  // ip:端口 已带端口就用原样;裸 IP 补当前应用默认端口
  function makeAddr(ip: string, port: number | string): string {
    ip = ip.trim();
    if (!ip) return "";
    if (ip.includes(":")) return ip;
    const p = Number(port);
    if (!(p >= 1 && p <= 65535)) return "";
    return `${ip}:${p}`;
  }

  // ---- 项目配置持久化 ----
  async function loadApps() {
    apps = await invoke<AppProfile[]>("load_apps");
    if (apps.length === 0) apps = [blankProject("项目A")];
    await switchApp(0, false);
  }

  // 主机列表是独立工作态,保存前同步回当前项目
  async function persistApps() {
    const p = currentProject();
    if (p) p.hosts = hosts.map((h) => ({ addr: h.addr }));
    apps = apps; // 触发下拉框选项文本(台数/端口)重渲染
    await invoke("save_apps", { apps });
  }

  // 更新包输入框用:改动防抖落盘
  let saveTimer: ReturnType<typeof setTimeout> | undefined;
  function persistProject() {
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => { persistApps().catch(() => {}); }, 300);
  }

  async function switchApp(i: number, autoprobe = true) {
    activeIdx = i;
    hosts = apps[i].hosts.map((e) => freshHost(e.addr));
    newPort = apps[i].port;
    importFrom = "";
    dist = null;
    distError = "";
    if (apps[i].dist_dir?.trim()) refreshDist();
    if (autoprobe) hosts.forEach((h) => probeOne(h.addr));
  }

  function uniqueName(base: string): string {
    if (!apps.some((a) => a.name === base)) return base;
    for (let n = 2; ; n++) {
      const name = `${base}-${n}`;
      if (!apps.some((a) => a.name === name)) return name;
    }
  }

  async function createApp() {
    const name = uniqueName(newName.trim() || `项目${apps.length + 1}`);
    apps = [...apps, blankProject(name, newPort || 7600)];
    newName = "";
    addingApp = false;
    await switchApp(apps.length - 1, false);
    await persistApps();
  }

  async function renameApp() {
    const n = renameText.trim();
    if (n && active) {
      active.name = n;
      apps = apps;
      await persistApps();
    }
    renaming = false;
  }

  async function deleteApp() {
    apps = apps.filter((_, i) => i !== activeIdx);
    if (apps.length === 0) apps = [blankProject("项目A")];
    await switchApp(Math.max(0, Math.min(activeIdx, apps.length - 1)), false);
    await persistApps();
  }

  async function changePort() {
    if (!active) return;
    let p = Number(String(active.port).replace(/\D/g, ""));
    if (!(p >= 1 && p <= 65535)) p = 7600;
    active.port = p;
    newPort = p;
    apps = apps;
    await persistApps();
  }

  // ---- 主机增删 ----
  async function addHost() {
    const a = makeAddr(newIp, newPort);
    if (!a || hosts.some((h) => h.addr === a)) return;
    hosts = [...hosts, freshHost(a)];
    newIp = "";
    await persistApps();
    probeOne(a);
  }

  async function removeHost(addr: string) {
    hosts = hosts.filter((h) => h.addr !== addr);
    await persistApps();
  }

  // 批量添加:一行/一条一个 IP 或 ip:端口,逗号空格分隔也行
  async function batchAdd() {
    const tokens = batchText.split(/[\s,;，；]+/).filter(Boolean);
    const fresh: HostState[] = [];
    for (const t of tokens) {
      const a = makeAddr(t, newPort);
      if (a && !hosts.some((h) => h.addr === a) && !fresh.some((h) => h.addr === a))
        fresh.push(freshHost(a));
    }
    if (fresh.length === 0) return;
    hosts = [...hosts, ...fresh];
    batchText = "";
    await persistApps();
    pushLog("批量添加", `新增 ${fresh.length} 台,跳过无效/重复 ${tokens.length - fresh.length} 条`);
    fresh.forEach((h) => probeOne(h.addr));
  }

  // 把另一个应用配置好的机器整批导入(端口随源配置逐台保留)
  async function importHosts() {
    const src = apps.find((a) => a.name === importFrom);
    if (!src) return;
    const fresh: HostState[] = [];
    for (const e of src.hosts) {
      if (!hosts.some((h) => h.addr === e.addr) && !fresh.some((h) => h.addr === e.addr))
        fresh.push(freshHost(e.addr));
    }
    if (fresh.length === 0) return;
    hosts = [...hosts, ...fresh];
    importFrom = "";
    await persistApps();
    pushLog("导入机器", `从「${src.name}」导入 ${fresh.length} 台`);
    fresh.forEach((h) => probeOne(h.addr));
  }

  // ---- 探测 ----
  async function probeOne(addr: string) {
    const h = findHost(addr);
    if (!h || h.probing) return;
    h.probing = true;
    hosts = hosts;
    try {
      const r = await invoke<ProbeResult>("cmd_probe", { addr, token: currentProject()?.token || null });
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
    const p = currentProject();
    const dir = p?.dist_dir?.trim() || "";
    if (!dir) return;
    try {
      const r = await invoke<DistInfo>("cmd_dist_info", {
        distDir: dir,
        main: p?.main_exe?.trim() || null,
        overwriteConfig: !!p?.overwrite_config,
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
    const p = currentProject();
    const jobs = targets.map(async (addr) => {
      try {
        const r = await invoke<any>("cmd_update_host", {
          addr,
          distDir: p?.dist_dir?.trim() || "",
          main: p?.main_exe?.trim() || null,
          token: p?.token || null,
          signKey: p?.sign_key?.trim() || null,
          overwriteConfig: !!p?.overwrite_config,
          timeoutSecs: Number(p?.timeout_secs) || 120,
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
    await loadApps();
    await listen<LogLine>("update-log", (e) => {
      const { addr, line } = e.payload;
      pushLog(addr, line);
    });
  });
</script>

<div class="panel">
  <h2>更新包 <span class="meta">· 随项目「{active?.name || ""}」保存</span></h2>
  <div class="row">
    <input type="text" class="grow" placeholder="新版 dist 目录,如 D:\dist" bind:value={active.dist_dir} on:change={() => { refreshDist(); persistProject(); }} />
    <input type="text" style="width: 130px" placeholder="主程序(可选)" bind:value={active.main_exe} on:change={() => { refreshDist(); persistProject(); }} />
    <label class="hint"><input type="checkbox" bind:checked={active.overwrite_config} on:change={() => { refreshDist(); persistProject(); }} /> 覆盖 config.json</label>
    <button on:click={refreshDist}>重新计算</button>
  </div>
  <div class="row" style="margin-top: 8px">
    <input type="password" style="width: 200px" placeholder="Token(可选)" bind:value={active.token} on:change={persistProject} />
    <input type="password" style="width: 200px" placeholder="签名私钥 hex(可选)" bind:value={active.sign_key} on:change={persistProject} />
    <span class="hint">轮询超时</span>
    <input type="text" style="width: 60px" bind:value={active.timeout_secs} on:change={persistProject} />
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
    {#if renaming}
      <h2 class="grow">项目</h2>
      <input type="text" style="width: 220px" bind:value={renameText}
        on:keydown={(e) => { if (e.key === "Enter") renameApp(); if (e.key === "Escape") renaming = false; }} />
      <button class="primary" on:click={renameApp}>确定</button>
      <button on:click={() => (renaming = false)}>取消</button>
    {:else if addingApp}
      <h2 class="grow">项目</h2>
      <input type="text" style="width: 220px" placeholder="新项目名,如 项目C" bind:value={newName}
        on:keydown={(e) => { if (e.key === "Enter") createApp(); if (e.key === "Escape") (addingApp = false); }} />
      <button class="primary" on:click={createApp}>创建</button>
      <button on:click={() => (addingApp = false)}>取消</button>
    {:else}
      <h2 class="grow">项目(推送对象)</h2>
      <select bind:value={activeIdx} on:change={() => switchApp(activeIdx)}>
        {#each apps as a, i}
          <option value={i}>{a.name} · {a.hosts.length}台 · 端口{a.port}</option>
        {/each}
      </select>
      <button on:click={() => { renaming = true; renameText = active?.name || ""; }}>改名</button>
      <button on:click={() => { addingApp = true; newName = ""; }}>新增项目</button>
      <button class="danger" on:click={deleteApp}>删除项目</button>
      <span class="hint">默认端口</span>
      <input type="text" style="width: 60px" bind:value={active.port} on:change={changePort} />
    {/if}
  </div>

  {#if batchOpen}
    <div class="row" style="margin-top: 8px">
      <textarea class="grow" rows="3" bind:value={batchText}
        placeholder="粘贴机器列表一次添加:每行一个 IP(端口用上方默认端口),也可写 ip:端口 覆盖;空格/逗号分隔均可&#10;192.168.1.60&#10;192.168.1.61:7601"></textarea>
    </div>
    <div class="row" style="margin-top: 6px">
      <button class="primary" on:click={batchAdd}>批量添加</button>
      <select bind:value={importFrom}>
        <option value="" disabled>从其他项目整批导入已配好的机器…</option>
        {#each apps as a, i}
          {#if i !== activeIdx}
            <option value={a.name}>{a.name}({a.hosts.length}台)</option>
          {/if}
        {/each}
      </select>
      <button on:click={importHosts} disabled={!importFrom}>导入</button>
      <span class="grow"></span>
      <button on:click={() => (batchOpen = false)}>收起</button>
    </div>
  {/if}

  <div class="row" style="margin-top: 8px">
    <h2 class="grow">{active?.name || ""} 主机({hosts.length})</h2>
    <input type="text" style="width: 200px" placeholder="IP,如 192.168.1.60" bind:value={newIp}
      on:keydown={(e) => e.key === "Enter" && addHost()} />
    <span class="hint">:</span>
    <input type="text" style="width: 64px" bind:value={newPort}
      on:keydown={(e) => e.key === "Enter" && addHost()} />
    <button on:click={addHost}>添加</button>
    <button on:click={() => (batchOpen = !batchOpen)}>{batchOpen ? "收起批量" : "批量添加"}</button>
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
        <tr><td colspan="7" class="meta" style="text-align: center; padding: 18px">
          「{active?.name}」还没有机器:填 IP 添加,或用「批量添加」粘一排 IP 一次全加
        </td></tr>
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
