# run_godot_verify.ps1
# 统一 Godot 自动化验证外壳启动器（第一防线：进程树硬超时与双进程强杀保护）
# 用法示例：
#   powershell -ExecutionPolicy Bypass -File U:\anti\projects\pure_connect_gal2\tools\run_godot_verify.ps1 -Script U:\anti\scratchpad\261122\verify_review_fixes.gd
#   powershell -ExecutionPolicy Bypass -File U:\anti\projects\pure_connect_gal2\tools\run_godot_verify.ps1 -Script U:\anti\projects\pure_connect_gal2\tools\verify_stage5.gd -Headless -TimeoutSec 30
#   powershell -ExecutionPolicy Bypass -File U:\anti\projects\pure_connect_gal2\tools\run_godot_verify.ps1 -AllGates

[CmdletBinding()]
param(
    [string]$Script = "",
    [int]$TimeoutSec = 60,
    [switch]$Headless,
    [switch]$AllGates,
    [switch]$SummaryOnly,
    [string]$ProjectDir = "U:\anti\projects\pure_connect_gal2\project",
    [string]$GodotExe = "U:\anti\projects\pure_connect_gal2\tools\godot\Godot_v4.3-stable_win64_console.exe"
)

$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

if (-not (Test-Path -LiteralPath $GodotExe)) {
    [Console]::Error.WriteLine("[ERROR] Godot 可执行文件不存在: $GodotExe")
    exit 2
}
if (-not (Test-Path -LiteralPath $ProjectDir)) {
    [Console]::Error.WriteLine("[ERROR] Godot 工程目录不存在: $ProjectDir")
    exit 2
}
if ($TimeoutSec -lt 1) {
    $TimeoutSec = 1
}

$dayStamp = Get-Date -Format "yyMMdd"
$logDir = "U:\anti\scratchpad\$dayStamp\godot_verify_logs"
if (-not (Test-Path -LiteralPath $logDir)) {
    New-Item -ItemType Directory -Path $logDir -Force | Out-Null
}

function Write-SummaryOutput {
    param(
        [string]$TargetScript,
        [string]$OutLog,
        [string]$ErrLog,
        [int]$ExitCode,
        [bool]$OneLineGate = $false
    )

    $baseName = [System.IO.Path]::GetFileNameWithoutExtension($TargetScript)
    $savedLog = Join-Path $logDir "last_${baseName}.log"
    $allLines = New-Object System.Collections.Generic.List[string]
    if (Test-Path -LiteralPath $OutLog) {
        foreach ($line in (Get-Content -LiteralPath $OutLog -Encoding UTF8)) {
            [void]$allLines.Add($line)
        }
    }
    if (Test-Path -LiteralPath $ErrLog) {
        foreach ($line in (Get-Content -LiteralPath $ErrLog -Encoding UTF8)) {
            [void]$allLines.Add($line)
        }
    }
    Set-Content -LiteralPath $savedLog -Value $allLines -Encoding UTF8

    if ($ExitCode -ne 0) {
        [Console]::Error.WriteLine("[FAIL] $baseName (exit=$ExitCode), 完整日志: $savedLog")
        $start = [Math]::Max(0, $allLines.Count - 40)
        for ($i = $start; $i -lt $allLines.Count; $i++) {
            [Console]::Error.WriteLine($allLines[$i])
        }
        return
    }

    if ($OneLineGate) {
        [Console]::Out.WriteLine("[GATE PASS] $baseName (exit=0, log=$savedLog)")
        return
    }

    $foldedNoise = 0
    $important = New-Object System.Collections.Generic.List[string]
    foreach ($line in $allLines) {
        if ($line -match "CompressedBg::decode failed for candidate 'arev0" -or
            $line -match "resolve_image 未命中或全部解码失败: original='arev0") {
            $foldedNoise++
            continue
        }
        if ($line -match '^\s*(\[Check|\[OK\]|\[ALL |\[验收|\[专项|ERROR:|SCRIPT ERROR|FAILED|panic)') {
            [void]$important.Add($line)
        }
    }

    foreach ($line in $important) {
        [Console]::Out.WriteLine($line)
    }
    if ($foldedNoise -gt 0) {
        [Console]::Out.WriteLine("[INFO] 已折叠 $foldedNoise 条已知历史 arev0* 解码提示 (完整日志: $savedLog)")
    }
    [Console]::Out.WriteLine("[PASS] $baseName (exit=0, log=$savedLog)")
}

function Invoke-OneGodotScript {
    param(
        [string]$TargetScript,
        [bool]$UseHeadless,
        [int]$LimitSec,
        [bool]$UseSummaryOnly,
        [bool]$OneLineGate = $false
    )

    if (-not (Test-Path -LiteralPath $TargetScript)) {
        [Console]::Error.WriteLine("[ERROR] 验证脚本不存在: $TargetScript")
        return 2
    }

    $stamp = "{0}_{1}" -f (Get-Date -Format "yyyyMMdd_HHmmss_fff"), $PID
    $outLog = Join-Path $logDir "out_$stamp.log"
    $errLog = Join-Path $logDir "err_$stamp.log"

    $godotArgs = @()
    if ($UseHeadless) {
        $godotArgs += "--headless"
    }
    $godotArgs += @("--path", $ProjectDir, "--script", $TargetScript)

    $proc = Start-Process -FilePath $GodotExe -ArgumentList $godotArgs `
        -PassThru -NoNewWindow `
        -RedirectStandardOutput $outLog `
        -RedirectStandardError $errLog
    # 预先访问 Handle 确保 .NET 在进程退出后能可靠读取 ExitCode
    $null = $proc.Handle

    try {
        $finished = $proc.WaitForExit($LimitSec * 1000)
        if (-not $finished) {
            [Console]::Error.WriteLine("[TIMEOUT] 脚本运行超过 ${LimitSec}s，正在强杀进程树 (PID=$($proc.Id)): $TargetScript")
            & taskkill.exe /F /T /PID $proc.Id 2>$null | Out-Null
            Start-Sleep -Milliseconds 300
            if (Test-Path -LiteralPath $outLog) {
                Get-Content -LiteralPath $outLog -Encoding UTF8 -Tail 30 | ForEach-Object { [Console]::Out.WriteLine($_) }
            }
            if (Test-Path -LiteralPath $errLog) {
                Get-Content -LiteralPath $errLog -Encoding UTF8 -Tail 30 | ForEach-Object { [Console]::Error.WriteLine($_) }
            }
            return 124
        }

        # 无参 WaitForExit 确保异步输出流刷新完毕且 ExitCode 已填充
        $proc.WaitForExit()
        $exitCode = if ($null -eq $proc.ExitCode) { 1 } else { [int]$proc.ExitCode }

        if ($UseSummaryOnly) {
            Write-SummaryOutput -TargetScript $TargetScript -OutLog $outLog -ErrLog $errLog -ExitCode $exitCode -OneLineGate $OneLineGate
        } else {
            if (Test-Path -LiteralPath $outLog) {
                Get-Content -LiteralPath $outLog -Encoding UTF8 | ForEach-Object { [Console]::Out.WriteLine($_) }
            }
            if (Test-Path -LiteralPath $errLog) {
                Get-Content -LiteralPath $errLog -Encoding UTF8 | ForEach-Object { [Console]::Error.WriteLine($_) }
            }
        }
        return $exitCode
    }
    finally {
        Remove-Item -LiteralPath $outLog, $errLog -Force -ErrorAction SilentlyContinue
    }
}

if ($AllGates) {
    $toolsDir = Split-Path -Parent $PSCommandPath
    $gateScripts = @(
        (Join-Path $toolsDir "verify_stage5.gd"),
        (Join-Path $toolsDir "verify_microchecks.gd"),
        (Join-Path $toolsDir "verify_pc_match.gd"),
        (Join-Path $toolsDir "verify_sprint6a_transitions.gd"),
        (Join-Path $toolsDir "verify_all_4_fixes.gd")
    )
    $jobs = New-Object System.Collections.Generic.List[hashtable]
    $idx = 0
    foreach ($gate in $gateScripts) {
        $idx++
        if (-not (Test-Path -LiteralPath $gate)) {
            [Console]::Error.WriteLine("[ERROR] 验证脚本不存在: $gate")
            exit 2
        }
        $stamp = "{0}_{1}_{2}" -f (Get-Date -Format "yyyyMMdd_HHmmss_fff"), $PID, $idx
        $outLog = Join-Path $logDir "out_$stamp.log"
        $errLog = Join-Path $logDir "err_$stamp.log"
        $godotArgs = @("--headless", "--path", $ProjectDir, "--script", $gate)
        $proc = Start-Process -FilePath $GodotExe -ArgumentList $godotArgs `
            -PassThru -NoNewWindow `
            -RedirectStandardOutput $outLog `
            -RedirectStandardError $errLog
        $null = $proc.Handle
        [void]$jobs.Add(@{
            Gate   = $gate
            Proc   = $proc
            OutLog = $outLog
            ErrLog = $errLog
        })
    }

    $failCode = 0
    try {
        foreach ($job in $jobs) {
            $gate = $job.Gate
            $proc = $job.Proc
            $outLog = $job.OutLog
            $errLog = $job.ErrLog
            if (-not $SummaryOnly.IsPresent) {
                Write-Output ">>> Running Gate (Headless, timeout=${TimeoutSec}s): $gate"
            }
            $finished = $proc.WaitForExit($TimeoutSec * 1000)
            if (-not $finished) {
                [Console]::Error.WriteLine("[TIMEOUT] 脚本运行超过 ${TimeoutSec}s，正在强杀进程树 (PID=$($proc.Id)): $gate")
                & taskkill.exe /F /T /PID $proc.Id 2>$null | Out-Null
                Start-Sleep -Milliseconds 200
                $rc = 124
            } else {
                $proc.WaitForExit()
                $rc = if ($null -eq $proc.ExitCode) { 1 } else { [int]$proc.ExitCode }
            }

            if ($SummaryOnly.IsPresent) {
                Write-SummaryOutput -TargetScript $gate -OutLog $outLog -ErrLog $errLog -ExitCode $rc -OneLineGate $true
            } else {
                if (Test-Path -LiteralPath $outLog) {
                    Get-Content -LiteralPath $outLog -Encoding UTF8 | ForEach-Object { [Console]::Out.WriteLine($_) }
                }
                if (Test-Path -LiteralPath $errLog) {
                    Get-Content -LiteralPath $errLog -Encoding UTF8 | ForEach-Object { [Console]::Error.WriteLine($_) }
                }
            }

            if ($rc -ne 0 -and $failCode -eq 0) {
                [Console]::Error.WriteLine("[GATE FAILED] $gate exited with code $rc")
                $failCode = $rc
            }
        }
    }
    finally {
        foreach ($job in $jobs) {
            if (-not $job.Proc.HasExited) {
                & taskkill.exe /F /T /PID $job.Proc.Id 2>$null | Out-Null
            }
            Remove-Item -LiteralPath $job.OutLog, $job.ErrLog -Force -ErrorAction SilentlyContinue
        }
    }

    if ($failCode -ne 0) {
        exit $failCode
    }
    Write-Output "[ALL GATES PASSED] 全部 5 项 Headless 门禁验证通过。"
    exit 0
}

if ([string]::IsNullOrWhiteSpace($Script)) {
    [Console]::Error.WriteLine("[ERROR] 请指定 -Script <path> 或 -AllGates")
    exit 2
}

$code = Invoke-OneGodotScript -TargetScript $Script -UseHeadless $Headless.IsPresent -LimitSec $TimeoutSec -UseSummaryOnly $SummaryOnly.IsPresent
exit $code
