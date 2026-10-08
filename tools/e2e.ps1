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
Start-Sleep -Milliseconds 300
if (Test-Path $E2E) { Remove-Item -Recurse -Force $E2E }
New-Item -ItemType Directory -Force -Path "$E2E\dist-v1", "$E2E\dist-v2", "$E2E\install" | Out-Null

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

# 8. 清理
Get-Process ghost-demo -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Write-Host ""
Write-Host "E2E 全绿" -ForegroundColor Green
