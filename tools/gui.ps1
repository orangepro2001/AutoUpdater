# gui.ps1 — ghost-updater 推送台(Tauri GUI)构建 / 运行 / 打包统一入口
#
# 为什么需要它:debug 构建下 Tauri 会忽略已构建的 webdist,改让窗口加载
# tauri.conf.json 的 devUrl(http://localhost:5173),而那个 Vite 开发服务器
# 只有 `tauri dev` 才会自动拉起。直接 `cargo run` 必然报 ERR_CONNECTION_REFUSED。
# 所以想跑窗口,要么走 dev(要热更新),要么走 run(用已构建前端,官方路径)。
#
# 用法: powershell -ExecutionPolicy Bypass -File tools\gui.ps1 <命令>
#       或直接用薄包装: tools\gui.cmd <命令>
$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $PSScriptRoot              # 仓库根
$Gui  = Join-Path $Root "crates\ghost-updater-tauri"  # 前端工程(vite/npm 在这里)
$App  = Join-Path $Gui "src-tauri"                    # Rust 工程
$Exe  = Join-Path $App "target\debug\ghost-updater-tauri.exe"

function Fail($msg) { Write-Host "[FAIL] $msg" -ForegroundColor Red; exit 1 }
function Ok($msg)   { Write-Host "[OK]   $msg" -ForegroundColor Green }

function Need-Npm {
    if (-not (Test-Path (Join-Path $Gui "node_modules"))) {
        Fail "缺少 node_modules,先执行: cd crates\ghost-updater-tauri; npm install"
    }
}

# npm/cargo 的 stderr 在 $ErrorActionPreference=Stop 下可能变 terminating error,
# 统一经 cmd 调用只看退出码
function Invoke-Cmd([string]$workDir, [string]$argLine) {
    Write-Host "> $argLine" -ForegroundColor DarkGray
    Push-Location $workDir
    try { cmd /c "$argLine 2>&1"; $code = $LASTEXITCODE } finally { Pop-Location }
    if ($code -ne 0) { Fail "$argLine 退出码 $code" }
}

function Build-Frontend { Need-Npm; Invoke-Cmd $Gui "npm run build" }

# 上一次运行的推送台实例会锁住目标 exe,导致 cargo 覆盖时报 os error 5(拒绝访问)
function Stop-Running {
    $p = Get-Process ghost-updater-tauri -ErrorAction SilentlyContinue
    if ($p) {
        Write-Host "先关闭仍在运行的推送台实例(PID $($p.Id -join ','))" -ForegroundColor Yellow
        $p | Stop-Process -Force -ErrorAction SilentlyContinue
        Start-Sleep -Milliseconds 500
    }
}

# 开发模式:CLI 会先跑 beforeDevCommand(npm run dev)起 Vite,再开窗口,改前端热更新
function Cmd-Dev {
    Need-Npm
    Stop-Running
    Invoke-Cmd $Gui "npm run tauri -- dev"
}

# 本地跑一次"打完包的界面":Rust 用 debug 编译,前端用已构建的 webdist(不走 devUrl)
function Cmd-Run {
    Need-Npm
    Stop-Running
    Invoke-Cmd $Gui "npm run tauri -- build --debug --no-bundle"
    if (-not (Test-Path $Exe)) { Fail "未找到 $Exe" }
    Ok "启动 $Exe"
    & $Exe
}

# 只重新构建前端 → webdist
function Cmd-Build {
    Build-Frontend
    Ok "webdist 就绪: $(Join-Path $Gui 'webdist')"
}

# 前端构建 + GUI crate 类型检查(generate_context! 编译期要读 webdist,故不能省前端这步)
function Cmd-Check {
    Build-Frontend
    Invoke-Cmd $App "cargo check"
    Ok "GUI crate cargo check 通过"
}

# 正式安装包(NSIS)
function Cmd-Installer {
    Need-Npm
    Stop-Running
    Invoke-Cmd $Gui "npm run tauri -- build"
    $dir = Join-Path $App "target\release\bundle\nsis"
    Ok "安装包输出目录: $dir"
    Get-ChildItem $dir -Filter *.exe -ErrorAction SilentlyContinue | ForEach-Object { Write-Host "       $($_.Name)" }
}

# 主 workspace 全量核验 + GUI check
function Cmd-Verify {
    Invoke-Cmd $Root "cargo build --workspace"
    Invoke-Cmd $Root "cargo test --workspace"
    Cmd-Check
    Ok "workspace build/test + GUI check 全绿"
}

function Usage {
    Write-Host "ghost-updater 推送台(GUI)入口脚本"
    Write-Host ""
    Write-Host "  dev        开发模式:起 Vite + 开窗口,前端热更新"
    Write-Host "  run        跑打包版界面:debug 编译 + 内嵌 webdist(绕过 devUrl)"
    Write-Host "  build      只构建前端 → webdist"
    Write-Host "  check      构建前端 + GUI crate cargo check"
    Write-Host "  installer  正式打包 → NSIS 安装包"
    Write-Host "  verify     主 workspace build/test + GUI check"
    Write-Host ""
    Write-Host "不要直接 cargo run:debug 会去连 devUrl,无人起 Vite 必报连接被拒。"
}

switch ($args[0]) {
    "dev"       { Cmd-Dev }
    "run"       { Cmd-Run }
    "build"     { Cmd-Build }
    "check"     { Cmd-Check }
    "installer" { Cmd-Installer }
    "verify"    { Cmd-Verify }
    default     { Usage; if ($args[0]) { exit 1 } }
}
