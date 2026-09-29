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

function Out-Line {
    param([string]$Text)
    [Console]::Out.WriteLine($Text)
    [Console]::Out.Flush()
}

function Get-StrayPid {
    param([int[]]$Before)
    Start-Sleep -Milliseconds 500
    $after = Get-Process WINWORD -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Id
    return ($after | Where-Object { $Before -notcontains $_ } | Select-Object -First 1)
}

try {
    $beforePids = Get-Process WINWORD -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Id

    $word = New-Object -ComObject Word.Application
    $newPid = Get-StrayPid -Before $beforePids
    if ($newPid) { Out-Line "PID $newPid" }

    $word.Visible = $false
    $word.DisplayAlerts = 0        # wdAlertsNone
    $word.AutomationSecurity = 3   # msoAutomationSecurityForceDisable (belt & suspenders, no macros)

    Out-Line "STAGE converting"
    $inPath = (Resolve-Path $In).Path

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
    if (-not (Test-Path $outDir)) { New-Item -ItemType Directory -Path $outDir | Out-Null }

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
        $stray = Get-Process -Id $newPid -ErrorAction SilentlyContinue
        if ($stray) {
            try { Stop-Process -Id $newPid -Force -ErrorAction SilentlyContinue } catch {}
        }
    }
}
