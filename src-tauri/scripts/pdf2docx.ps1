param(
    [Parameter(Mandatory=$true)][string]$In,
    [Parameter(Mandatory=$true)][string]$Out,
    [string]$Password = $null
)

$ErrorActionPreference = 'Stop'
$sw = [System.Diagnostics.Stopwatch]::StartNew()
$word = $null
$doc = $null
$newPid = $null
$scriptStart = Get-Date

function Out-Line {
    param([string]$Text)
    [Console]::Out.WriteLine($Text)
    [Console]::Out.Flush()
}

function Get-OwnPid {
    # Our Office = not in the before-snapshot, COM-launched (/Automation), started after this script did.
    param([int[]]$Before, [string]$Image)
    for ($i = 0; $i -lt 40; $i++) {
        $p = Get-CimInstance Win32_Process -Filter "Name='$Image'" -ErrorAction SilentlyContinue |
            Where-Object { $Before -notcontains [int]$_.ProcessId -and $_.CommandLine -like '*/Automation*' -and $_.CreationDate -ge $scriptStart } |
            Select-Object -First 1
        if ($p) { return [int]$p.ProcessId }
        Start-Sleep -Milliseconds 250
    }
    return $null
}

function Test-OwnPid {
    param([int]$ProcId, [string]$Image)
    $p = Get-CimInstance Win32_Process -Filter "ProcessId=$ProcId" -ErrorAction SilentlyContinue
    return [bool]($p -and $p.Name -eq $Image -and $p.CommandLine -like '*/Automation*' -and $p.CreationDate -ge $scriptStart)
}

try {
    $beforePids = @(Get-Process WINWORD -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Id)

    $word = New-Object -ComObject Word.Application
    $newPid = Get-OwnPid -Before $beforePids -Image 'WINWORD.EXE'
    if ($newPid) { Out-Line "PID $newPid" }

    $word.Visible = $false
    $word.DisplayAlerts = 0        # wdAlertsNone
    $word.AutomationSecurity = 3   # msoAutomationSecurityForceDisable (belt & suspenders, no macros)

    Out-Line "STAGE converting"
    $inPath = (Resolve-Path -LiteralPath $In).Path

    # Documents.Open(FileName, ConfirmConversions, ReadOnly, AddToRecentFiles, PasswordDocument)
    # NOTE: passing the full ~16-arg COM signature with trailing $null values makes
    # PowerShell's late-bound COM dispatch throw a bogus NullReferenceException.
    # Only pass as many positional args as actually needed.
    if ($Password) {
        $doc = $word.Documents.Open($inPath, $false, $true, $false, $Password)
    } else {
        $doc = $word.Documents.Open($inPath, $false, $true, $false)
    }

    # NOTE: use PowerShell's own location (not [IO.Path]::GetFullPath, which
    # resolves against the process's Win32 CWD - that silently diverges from
    # $PWD/Set-Location in some hosts and writes the file to the wrong folder).
    $outPath = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Out)
    $outDir = Split-Path $outPath -Parent
    if (-not (Test-Path -LiteralPath $outDir)) { [void][IO.Directory]::CreateDirectory($outDir) }

    Out-Line "STAGE saving"
    # wdFormatXMLDocument = 16
    $doc.SaveAs2($outPath, 16)

    $doc.Close($false)
    $doc = $null

    $word.Quit()
    $sw.Stop()
    Write-Output ("OK seconds=$([math]::Round($sw.Elapsed.TotalSeconds,2)) out=$outPath")
    exit 0
}
catch {
    $sw.Stop()
    $oneLine = $_.Exception.Message -replace "[\r\n]+", ' '
    Write-Output ("ERROR seconds=$([math]::Round($sw.Elapsed.TotalSeconds,2)) msg=$oneLine")
    exit 1
}
finally {
    try { if ($doc) { $doc.Close($false) } } catch {}
    try { if ($doc) { [Runtime.InteropServices.Marshal]::ReleaseComObject($doc) | Out-Null } } catch {}
    try { if ($word) { $word.Quit() } } catch {}
    try { if ($word) { [Runtime.InteropServices.Marshal]::ReleaseComObject($word) | Out-Null } } catch {}
    [GC]::Collect()
    [GC]::WaitForPendingFinalizers()
    [GC]::Collect()
    if ($newPid) {
        Start-Sleep -Milliseconds 500
        # only ever our own COM-launched instance; a user's Word (or a reused pid) fails the check
        if (Test-OwnPid -ProcId $newPid -Image 'WINWORD.EXE') {
            try { Stop-Process -Id $newPid -Force -ErrorAction SilentlyContinue } catch {}
        }
    }
}
