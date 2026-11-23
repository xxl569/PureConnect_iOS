@echo off
chcp 65001 >nul
cd /d "%~dp0"

echo ======================================================================
echo   PURExCONNECT Galgame 原生移植 [路线 B: Rust + Godot 4.3]
echo   架构: GDExtension + 表现层 + BYOA 外置资产 + 单向 Delta 快照
echo   分辨率: 1280x720  汉化补丁  Windows 1:1 热调试环境
echo ======================================================================
echo.

rem 1. 检查 Rust GDExtension DLL 编译产物
if exist "target\debug\pure_connect_godot.dll" goto :dll_ok
echo [提示] 未发现 target\debug\pure_connect_godot.dll，尝试调用 cargo 自动编译...
if exist "C:\Users\Administrator\.cargo\bin\cargo.exe" (
    "C:\Users\Administrator\.cargo\bin\cargo.exe" build --package pure_connect_godot
) else (
    cargo build --package pure_connect_godot
)
:dll_ok

rem 2. 检查并建立 project\target 与 project\tools 目录联接 Junction
if exist "project\target" goto :target_ok
echo [提示] 正在创建 project\target 目录联接...
mklink /J "project\target" "target" >nul 2>&1
if errorlevel 1 (
    if not exist "project\target\debug" mkdir "project\target\debug"
    copy /y "target\debug\pure_connect_godot.dll" "project\target\debug\" >nul
)
:target_ok

if not exist "project\tools" (
    mklink /J "project\tools" "tools" >nul 2>&1
)

rem 3. 检查 project\.godot\extension_list.cfg 配置
if not exist "project\.godot" mkdir "project\.godot"
if not exist "project\.godot\extension_list.cfg" (
    echo [提示] 正在生成 GDExtension 注册清单...
    echo res://pure_connect.gdextension > "project\.godot\extension_list.cfg"
)

rem 4. 自动探测系统中的 Godot 4.3 可执行程序
set "GODOT_EXE="

if defined GODOT4_BIN if exist "%GODOT4_BIN%" set "GODOT_EXE=%GODOT4_BIN%"
if not defined GODOT_EXE if defined GODOT_BIN if exist "%GODOT_BIN%" set "GODOT_EXE=%GODOT_BIN%"

if not defined GODOT_EXE if exist "tools\godot\Godot_v4.3-stable_win64_console.exe" set "GODOT_EXE=tools\godot\Godot_v4.3-stable_win64_console.exe"
if not defined GODOT_EXE if exist "tools\godot\Godot_v4.3-stable_win64.exe" set "GODOT_EXE=tools\godot\Godot_v4.3-stable_win64.exe"
if not defined GODOT_EXE if exist "..\HenPri_iOS\tools\godot\Godot_v4.3-stable_win64_console.exe" set "GODOT_EXE=..\HenPri_iOS\tools\godot\Godot_v4.3-stable_win64_console.exe"
if not defined GODOT_EXE if exist "..\HenPri_iOS\tools\godot\Godot_v4.3-stable_win64.exe" set "GODOT_EXE=..\HenPri_iOS\tools\godot\Godot_v4.3-stable_win64.exe"
if not defined GODOT_EXE if exist "..\SteinsGate0_iOS\tools\godot\Godot_v4.3-stable_win64_console.exe" set "GODOT_EXE=..\SteinsGate0_iOS\tools\godot\Godot_v4.3-stable_win64_console.exe"
if not defined GODOT_EXE if exist "..\SteinsGate0_iOS\tools\godot\Godot_v4.3-stable_win64.exe" set "GODOT_EXE=..\SteinsGate0_iOS\tools\godot\Godot_v4.3-stable_win64.exe"

if defined GODOT_EXE goto :found_godot

echo [错误] 未在系统或 tools\godot 目录中探测到 Godot 4.3 可执行程序！
echo.
echo 【排障向导】
echo 1. 请下载 Godot 4.3 官方稳定版；
echo 2. 将其放置在 tools\godot\ 目录下；
echo 3. 或在系统环境变量中设置 GODOT4_BIN 指向 Godot 可执行程序。
echo.
pause
exit /b 1

:found_godot
echo [OK] 探测到 Godot 运行时: %GODOT_EXE%
echo [OK] 正在启动 1:1 热调试环境...
echo [参数提示] 可通过附加参数直跳指定剧本，例如:
echo    启动Windows试玩.bat --script 01_prologue1
echo.

rem 5. 参数预处理与启动执行 (将 --script 自动转为 --scenario，避免与 Godot 内部 -s 冲突)
set "RAW_ARGS=%*"
set "FORWARD_ARGS=%RAW_ARGS:--script=--scenario%"
set "IS_HEADLESS="
for %%a in (%*) do (
    if /i "%%a"=="--headless" set "IS_HEADLESS=1"
)

if "%FORWARD_ARGS%"=="" (
    "%GODOT_EXE%" --path "project" res://Main.tscn
) else (
    "%GODOT_EXE%" --path "project" res://Main.tscn %FORWARD_ARGS%
)

if errorlevel 1 (
    echo.
    echo ======================================================================
    echo [异常提示] Godot 引擎异常退出！
    echo 排查建议:
    echo 1. 检查 target\debug\pure_connect_godot.dll 编译产物；
    echo 2. 检查 VFS 路径是否存在；
    echo 3. 可在命令行执行 "%GODOT_EXE% --path project --verbose res://Main.tscn" 查看详细堆栈。
    echo ======================================================================
    if not defined IS_HEADLESS pause
)
