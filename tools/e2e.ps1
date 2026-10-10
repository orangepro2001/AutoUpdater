# e2e.ps1 — ghost-updater 端到端验证
# 构建一次 demo-app + ghost-push,组装 v1/v2 两个 dist,真实走完
# probe → stage(哈希校验) → apply(分离拉起 apply worker) → 等旧进程退出 →
# 备份/覆盖/重启 → 轮询 started_at 确认重启 的全链路。
# 用法: powershell -ExecutionPolicy Bypass -File tools\e2e.ps1
$ErrorActionPreference = "Stop"

$Root   = Split-Path -Parent $PSScriptRoot
$E2E    = Join-Path $Root "target\e2e"
$Port   = 7601
$Token  = "e2e-token"
$Target = "127.0.0.1:$Port"

function Fail($msg) { Write-Host "[FAIL] $msg" -ForegroundColor Red; exit 1 }
function Ok($msg)   { Write-Host "[OK]   $msg" -ForegroundColor Green }

# 0. 清场(先杀残留进程,再删工作目录)
Get-Process ghost-demo -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Get-Process demo-gui  -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 300
if (Test-Path $E2E) { Remove-Item -Recurse -Force $E2E }
New-Item -ItemType Directory -Force -Path "$E2E\dist-v1", "$E2E\dist-v2", "$E2E\install", "$E2E\dist-v3", "$E2E\dist-v3-big", "$E2E\dist-v4" | Out-Null

# 1. 构建(demo-app 与 ghost-push 一次构建)
Write-Host "== cargo build =="
Push-Location $Root
cmd /c "cargo build --workspace >nul 2>&1"
if ($LASTEXITCODE -ne 0) { Pop-Location; Fail "cargo build" }
Pop-Location
$Demo  = Join-Path $Root "target\debug\ghost-demo.exe"
$Push  = Join-Path $Root "target\debug\ghost-push.exe"
Ok "构建完成"

# 2. 组装 dist:v1(装机)与 v2(新版)
Copy-Item $Demo "$E2E\dist-v1\ghost-demo.exe"
Set-Content -Encoding Ascii "$E2E\dist-v1\VERSION" "1.0.0"
Set-Content -Encoding Ascii "$E2E\dist-v1\NOTES.txt" "v1 notes"
Copy-Item $Demo "$E2E\dist-v2\ghost-demo.exe"
Set-Content -Encoding Ascii "$E2E\dist-v2\VERSION" "2.0.0"
Set-Content -Encoding Ascii "$E2E\dist-v2\NOTES.txt" "v2 notes"
# 安装目录 = v1 dist + 宿主自定义配置(config.json 不推,必须保留)
Copy-Item -Recurse -Force "$E2E\dist-v1\*" "$E2E\install\"
Set-Content -Encoding Ascii "$E2E\install\config.json" "{""keep"":true}"
Ok "dist 就绪 v1=1.0.0 v2=2.0.0"

# 3. 起 v1(带 Token)
$v1 = Start-Process -FilePath "$E2E\install\ghost-demo.exe" `
    -ArgumentList "--addr","$Target","--token",$Token `
    -WorkingDirectory $E2E -PassThru -WindowStyle Hidden
Start-Sleep -Milliseconds 1500
if ($v1.HasExited) { Fail "v1 启动失败" }

# 4. probe v1
$probe1 = Invoke-RestMethod "http://$Target/update/probe"
if ($probe1.version -ne "1.0.0") { Fail "probe v1 版本非 1.0.0: $($probe1.version)" }
Write-Host "       v1 probe: version=$($probe1.version) started_at=$($probe1.started_at) os=$($probe1.os) arch=$($probe1.arch)"
$oldStarted = $probe1.started_at

# 5. 无 token 的 stage 必须被拒(安全边界)
try {
    Invoke-RestMethod -Method Post "http://$Target/update/stage?sha256=00&main=x.exe" -InFile "$E2E\dist-v2\NOTES.txt" | Out-Null
    Fail "未带 token 的 stage 竟然成功"
} catch { Ok "未带 token 的 stage 被拒($($_.Exception.Response.StatusCode.value__))" }

# 6. ghost-push 推 v2(全链路)
Write-Host "== ghost-push v2 → $Target =="
& $Push push --target $Target --dist "$E2E\dist-v2" --token $Token --timeout-secs 60
if ($LASTEXITCODE -ne 0) { Fail "ghost-push 退出码 $LASTEXITCODE" }
Ok "ghost-push 成功"

# 7. 断言:版本变化 + started_at 变化 + 配置保留 + NOTES 换新 + 无残留备份
$probe2 = Invoke-RestMethod "http://$Target/update/probe"
if ($probe2.version -ne "2.0.0") { Fail "更新后版本非 2.0.0: $($probe2.version)" }
if ($probe2.started_at -eq $oldStarted) { Fail "started_at 未变化,疑似没重启" }
$notes = Get-Content "$E2E\install\NOTES.txt" -Raw
if ($notes.Trim() -ne "v2 notes") { Fail "NOTES.txt 未换新: $notes" }
$cfg = Get-Content "$E2E\install\config.json" -Raw
if ($cfg -notmatch '"keep"') { Fail "config.json 被误覆盖: $cfg" }
# 备份清理发生在 apply worker 健康检查之后(比 pusher 的重启确认晚 ~1.5s),轮询等它
$cleaned = $false
for ($i = 0; $i -lt 15; $i++) {
    if (-not (Test-Path "$E2E\install\.update_backup\backup-*")) { $cleaned = $true; break }
    Start-Sleep -Seconds 1
}
if (-not $cleaned) { Fail "成功更新后备份目录未清理" }
if (-not (Test-Path "$E2E\install\.update_backup\apply.log")) { Fail "缺 apply.log" }
$result = Get-Content "$E2E\install\.update_backup\result.json" -Raw | ConvertFrom-Json
if (-not $result.ok) { Fail "result.json ok=false: $($result.error)" }
Write-Host "       v2 probe: version=$($probe2.version) started_at=$($probe2.started_at)"
Ok "版本 1.0.0 → 2.0.0,config 保留,备份已清,result.json ok"

# 8. REQ-2:锁住安装目录文件逼停 swap → 自动回滚 + 锁释放,重推可成功
Copy-Item $Demo "$E2E\dist-v3\ghost-demo.exe"
Set-Content -Encoding Ascii "$E2E\dist-v3\VERSION" "3.0.0"
Set-Content -Encoding Ascii "$E2E\dist-v3\NOTES.txt" "v3 notes"
# 锁"写"不锁"读"(FileShare.Read):worker 备份读得动,覆盖写必败(os error 32)
$lock = [System.IO.File]::Open("$E2E\install\NOTES.txt", 'Open', 'Read', 'Read')
Write-Host "== REQ-2: 锁 NOTES.txt 后推 v3,期望失败并自动回滚 =="
$out = cmd /c "$Push push --target $Target --dist $E2E\dist-v3 --token $Token --timeout-secs 15 2>&1" | Out-String
$rc = $LASTEXITCODE
$lock.Close()
if ($rc -eq 0) { Fail "锁文件状态下 v3 竟推送成功: $($out.Trim())" }
$result = Get-Content -Raw -Encoding UTF8 "$E2E\install\.update_backup\result.json" | ConvertFrom-Json
if ($result.ok) { Fail "REQ-2 result.json ok=true: $($result.error)" }
if ($result.error -notmatch "已回滚") { Fail "REQ-2 result 未标注已回滚: $($result.error)" }
if ((Get-Content "$E2E\install\VERSION" -Raw).Trim() -ne "2.0.0") { Fail "REQ-2 回滚后 VERSION 非 2.0.0" }
$staleLock = Get-ChildItem "$E2E\.ghost-update" -Directory | Where-Object { Test-Path (Join-Path $_.FullName ".apply-lock") }
if ($staleLock) { Fail "REQ-2 失败后 .apply-lock 泄漏: $($staleLock.Name -join ',')" }
# 回滚后 worker 重拉旧版是无参启动,落默认端口 7600 —— 顺带验证服务已恢复
$rb = $null
for ($i = 0; $i -lt 10; $i++) {
    try { $rb = Invoke-RestMethod "http://127.0.0.1:7600/update/probe"; break } catch { Start-Sleep -Milliseconds 500 }
}
if (-not $rb -or $rb.version -ne "2.0.0") { Fail "REQ-2 回滚重拉不可达或版本非 2.0.0" }
Ok "REQ-2 swap 失败 → result 标注已回滚、无锁泄漏、旧版已重拉"

# 回滚重拉是无参启动,杀掉后带参重启,供后续场景继续用
Get-Process ghost-demo -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 300
Start-Process -FilePath "$E2E\install\ghost-demo.exe" -ArgumentList "--addr","$Target","--token",$Token `
    -WorkingDirectory $E2E -PassThru -WindowStyle Hidden | Out-Null
Start-Sleep -Milliseconds 1500
Write-Host "== REQ-2: 释放锁后重推 v3,期望成功 =="
& $Push push --target $Target --dist "$E2E\dist-v3" --token $Token --timeout-secs 60
if ($LASTEXITCODE -ne 0) { Fail "REQ-2 重推 v3 失败" }
if ((Invoke-RestMethod "http://$Target/update/probe").version -ne "3.0.0") { Fail "REQ-2 重推后版本非 3.0.0" }
Ok "REQ-2 重推 v3 成功(2.0.0 → 3.0.0)"

# 9. REQ-3:错 token 推 17MB 大包 ×10,每次都必须是 stage 被拒 401,而非传输错误
Copy-Item -Recurse -Force "$E2E\dist-v3\*" "$E2E\dist-v3-big\"
[System.IO.File]::WriteAllBytes("$E2E\dist-v3-big\junk.bin", (New-Object byte[] (17MB)))
Write-Host "== REQ-3: 错 token 推 17MB ×10,期望每次都稳定看到 401 =="
for ($i = 1; $i -le 10; $i++) {
    $out = cmd /c "$Push push --target $Target --dist $E2E\dist-v3-big --token wrong-$i --timeout-secs 5 2>&1" | Out-String
    if ($LASTEXITCODE -eq 0) { Fail "REQ-3 第 $i 次错 token 竟成功: $($out.Trim())" }
    # 断言用 ASCII 锚点(cmd 捕获会按 GBK 解码 ghost-push 的 UTF-8 输出,中文正则不可靠):
    # "HTTP status client error" = error_for_status 路径(401 可见);"error sending request" = 传输错误
    if ($out -notmatch "401") { Fail "REQ-3 第 $i 次未见 401: $($out.Trim())" }
    if ($out -notmatch "HTTP status client error") { Fail "REQ-3 第 $i 次非 error_for_status 路径: $($out.Trim())" }
    if ($out -match "error sending request") { Fail "REQ-3 第 $i 次仍是传输错误(401 不可见): $($out.Trim())" }
}
Ok "REQ-3 错 token ×10 全部稳定 401,0 次传输错误"

# 10. REQ-1:双进程编排——demo-gui.exe 常驻并锁住自身,agent 优雅关闭后全量 swap 成功
Copy-Item $Demo "$E2E\install\demo-gui.exe"
Start-Process -FilePath "$E2E\install\demo-gui.exe" -ArgumentList "--addr","127.0.0.1:7701","--token",$Token `
    -WorkingDirectory $E2E -PassThru -WindowStyle Hidden | Out-Null
Start-Sleep -Milliseconds 1500
if (-not (Get-Process demo-gui -ErrorAction SilentlyContinue)) { Fail "demo-gui 启动失败" }
if ((Invoke-RestMethod "http://127.0.0.1:7701/update/probe").version -ne "3.0.0") { Fail "demo-gui probe 非 3.0.0" }
Get-Process ghost-demo -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 300
Start-Process -FilePath "$E2E\install\ghost-demo.exe" -ArgumentList "--addr","$Target","--token",$Token,"--close-processes","demo-gui.exe" `
    -WorkingDirectory $E2E -PassThru -WindowStyle Hidden | Out-Null
Start-Sleep -Milliseconds 1500
Copy-Item $Demo "$E2E\dist-v4\ghost-demo.exe"
Copy-Item $Demo "$E2E\dist-v4\demo-gui.exe"
Set-Content -Encoding Ascii "$E2E\dist-v4\VERSION" "4.0.0"
Set-Content -Encoding Ascii "$E2E\dist-v4\NOTES.txt" "v4 notes"
Write-Host "== REQ-1: GUI(demo-gui)开着推 v4,期望优雅关闭后 swap 成功 =="
& $Push push --target $Target --dist "$E2E\dist-v4" --main ghost-demo.exe --token $Token --timeout-secs 60
if ($LASTEXITCODE -ne 0) { Fail "REQ-1 推 v4 失败" }
Start-Sleep -Seconds 1
if (Get-Process demo-gui -ErrorAction SilentlyContinue) { Fail "REQ-1 demo-gui 未被关闭" }
if ((Invoke-RestMethod "http://$Target/update/probe").version -ne "4.0.0") { Fail "REQ-1 推送后版本非 4.0.0" }
if ((Get-FileHash "$E2E\install\demo-gui.exe").Hash -ne (Get-FileHash "$E2E\dist-v4\demo-gui.exe").Hash) { Fail "REQ-1 demo-gui.exe 未被换新(锁未释放?)" }
Ok "REQ-1 demo-gui 被关闭、全量 swap 成功、主程序自恢复(3.0.0 → 4.0.0)"

# 11. 清理
Get-Process ghost-demo -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Get-Process demo-gui  -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Write-Host ""
Write-Host "E2E 全绿" -ForegroundColor Green
