<#
.SYNOPSIS
    Non-blocking wrapper around scripts/check.ps1 (the gate).

.DESCRIPTION
    The gate itself lives in scripts/check.ps1, which is a line-for-line twin of
    scripts/check.sh. This wrapper does not duplicate or modify it - it shells out
    to it, so there is exactly one definition of what the gate runs.

    A cold gate run takes minutes, which hangs an agent-driven terminal. Run it
    detached instead and inspect it on a later turn:

        ./scripts/check-bg.ps1            # start detached, print PID + logs, return at once
        ./scripts/check-bg.ps1 -Status    # report RUNNING / FINISHED - PASS / FINISHED - FAIL

    Never waits. See .clinerules/long-running-commands.md.
#>
[CmdletBinding(DefaultParameterSetName = 'Start')]
param(
    # Report on a detached run (tails the logs, says whether it finished). Never waits.
    [Parameter(ParameterSetName = 'Status')]
    [switch] $Status
)

$ErrorActionPreference = "Stop"

$CheckScript = Join-Path $PSScriptRoot 'check.ps1'
$RepoRoot    = Split-Path -Parent $PSScriptRoot

$LogOut     = Join-Path $env:TEMP 'elya-check.out.log'
$LogErr     = Join-Path $env:TEMP 'elya-check.err.log'
$DoneFile   = Join-Path $env:TEMP 'elya-check.done'
$PidFile    = Join-Path $env:TEMP 'elya-check.pid'
$RunnerFile = Join-Path $env:TEMP 'elya-check-runner.ps1'

function Get-BackgroundCheckProcess {
    if (-not (Test-Path -LiteralPath $PidFile)) { return $null }
    $recorded = (Get-Content -LiteralPath $PidFile -ErrorAction SilentlyContinue | Select-Object -First 1)
    if (-not $recorded) { return $null }
    $proc = Get-Process -Id ([int]$recorded) -ErrorAction SilentlyContinue
    if (-not $proc) { return $null }
    # Guard against PID reuse by an unrelated process.
    if ($proc.ProcessName -notmatch '^(powershell|pwsh)$') { return $null }
    return $proc
}

if ($Status) {
    if (-not (Test-Path -LiteralPath $PidFile)) {
        Write-Host "No background check recorded. Start one with: ./scripts/check-bg.ps1"
        exit 0
    }

    $recorded = (Get-Content -LiteralPath $PidFile -ErrorAction SilentlyContinue | Select-Object -First 1)
    $running  = Get-BackgroundCheckProcess

    if (Test-Path -LiteralPath $DoneFile) {
        $code = (Get-Content -LiteralPath $DoneFile -ErrorAction SilentlyContinue | Select-Object -First 1)
        if ("$code" -eq '0') {
            Write-Host "FINISHED - PASS (exit 0, PID $recorded)"
        }
        else {
            Write-Host "FINISHED - FAIL (exit $code, PID $recorded)"
        }
    }
    elseif ($running) {
        Write-Host "RUNNING (PID $($running.Id)) - not finished yet."
    }
    else {
        Write-Host "STOPPED (PID $recorded) with no result marker - killed, crashed, or never started."
    }

    Write-Host ""
    Write-Host "--- last 20 lines of $LogOut ---"
    Get-Content -LiteralPath $LogOut -Tail 20 -ErrorAction SilentlyContinue
    Write-Host ""
    Write-Host "--- last 20 lines of $LogErr ---"
    Get-Content -LiteralPath $LogErr -Tail 20 -ErrorAction SilentlyContinue
    exit 0
}

# ---------------------------------------------------------------------------
# Start: launch scripts/check.ps1 detached and return immediately.
# ---------------------------------------------------------------------------

$running = Get-BackgroundCheckProcess
if ($running) {
    Write-Host "A background check is already running (PID $($running.Id))."
    Write-Host "  stdout: $LogOut"
    Write-Host "  stderr: $LogErr"
    Write-Host "Use './scripts/check-bg.ps1 -Status' to check on it. Not starting a second copy."
    exit 0
}

Remove-Item -LiteralPath $DoneFile -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath $PidFile  -Force -ErrorAction SilentlyContinue

$psExe = (Get-Process -Id $PID).Path
if (-not $psExe) { $psExe = 'powershell.exe' }

# The detached child runs check.ps1 unmodified and records its exit code.
# check.ps1's `exit N` returns control here and lands in $LASTEXITCODE; a
# terminating error leaves the pre-seeded failure code, so the marker is
# always written.
$runnerBody = @"
`$ErrorActionPreference = 'Continue'
`$env:CARGO_INCREMENTAL = '0'
`$code = 1
try {
    & '$CheckScript'
    `$code = `$LASTEXITCODE
}
finally {
    Set-Content -LiteralPath '$DoneFile' -Value `$code -Encoding ascii
}
exit `$code
"@
Set-Content -LiteralPath $RunnerFile -Value $runnerBody -Encoding ascii

# -ArgumentList joins with spaces and does not quote, so pre-quote the path.
$proc = Start-Process -NoNewWindow -PassThru `
    -FilePath $psExe `
    -ArgumentList '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', "`"$RunnerFile`"" `
    -WorkingDirectory $RepoRoot `
    -RedirectStandardOutput $LogOut `
    -RedirectStandardError  $LogErr

Set-Content -LiteralPath $PidFile -Value $proc.Id -Encoding ascii

Write-Host "Started fmt + clippy + test detached (scripts/check.ps1)."
Write-Host "  PID   : $($proc.Id)"
Write-Host "  stdout: $LogOut"
Write-Host "  stderr: $LogErr"
Write-Host "Check on it later with: ./scripts/check-bg.ps1 -Status"
exit 0
