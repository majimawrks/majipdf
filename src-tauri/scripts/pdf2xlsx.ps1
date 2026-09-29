param(
    [Parameter(Mandatory=$true)][string]$In,
    [Parameter(Mandatory=$true)][string]$Out
)

$ErrorActionPreference = 'Stop'
$sw = [System.Diagnostics.Stopwatch]::StartNew()
$excel = $null
$wb = $null
$newPid = $null

function Out-Line {
    param([string]$Text)
    [Console]::Out.WriteLine($Text)
    [Console]::Out.Flush()
}

function Get-StrayPid {
    param([int[]]$Before)
    Start-Sleep -Milliseconds 500
    $after = Get-Process EXCEL -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Id
    return ($after | Where-Object { $Before -notcontains $_ } | Select-Object -First 1)
}

# Parses an Indonesian-formatted number string ("1.204.500", "73,5", "0,00",
# "(12.000)" negative, "100,00%" percent). Returns $null if the string has no
# thousands/decimal separator or percent sign - i.e. it is NOT a formatted
# number and must stay text (this also keeps plain digit codes like "054" or
# a bare "500" as text, since they carry no separator to disambiguate them
# from a numeric amount - lazy but matches every example in the spec).
function ConvertFrom-IndoNumber {
    param([string]$s)
    $t = $s.Trim()
    if ($t.Length -eq 0) { return $null }
    $neg = $false
    if ($t -match '^\((.*)\)$') { $neg = $true; $t = $Matches[1].Trim() }
    $isPercent = $false
    if ($t.EndsWith('%')) { $isPercent = $true; $t = $t.Substring(0, $t.Length - 1).Trim() }
    if ($t -notmatch '[.,]') { return $null }
    if ($t -notmatch '^\d{1,3}(\.\d{3})*(,\d+)?$' -and $t -notmatch '^\d+,\d+$') { return $null }
    $norm = ($t -replace '\.', '') -replace ',', '.'
    $val = 0.0
    if (-not [double]::TryParse($norm, [Globalization.NumberStyles]::Float, [Globalization.CultureInfo]::InvariantCulture, [ref]$val)) { return $null }
    if ($neg) { $val = -$val }
    if ($isPercent) { $val = $val / 100.0 }
    return [PSCustomObject]@{ Value = $val; IsPercent = $isPercent }
}

try {
    $beforePids = Get-Process EXCEL -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Id

    $excel = New-Object -ComObject Excel.Application
    $newPid = Get-StrayPid -Before $beforePids
    if ($newPid) { Out-Line "PID $newPid" }

    $excel.Visible = $false
    $excel.DisplayAlerts = $false
    $excel.ScreenUpdating = $false
    $excel.AskToUpdateLinks = $false
    $excel.AutomationSecurity = 3   # msoAutomationSecurityForceDisable

    $inPath = (Resolve-Path $In).Path
    $mPath = $inPath -replace '\\', '\\'

    $wb = $excel.Workbooks.Add()

    # Single query: parse the PDF ONCE, flatten every "Page" item's Data table
    # into a long (PageIndex, RowIndex, ColIndex, Value) shape client-side in
    # M. This avoids re-parsing the whole PDF per page (the old script's cost)
    # and sidesteps ragged/varying column counts across pages (can't align
    # them into one wide table). Value is explicitly typed `text` so the
    # Mashup OLEDB provider reports a text column to Excel - Excel then loads
    # "054" etc. verbatim instead of auto-detecting it as a number on refresh.
    Out-Line "STAGE reading"
    $queryName = 'MajiPdfLong'
    $formula = @"
let
    Source = Pdf.Tables(File.Contents("$mPath"), [Implementation="1.3"]),
    Pages = Table.SelectRows(Source, each [Kind] = "Page"),
    WithIdx = Table.AddIndexColumn(Pages, "PageIndex", 1, 1, Int64.Type),
    ToLong = Table.AddColumn(WithIdx, "LongRows", each
        let
            pidx = [PageIndex],
            rows = Table.ToRows([Data]),
            recs = List.Combine(List.Transform(List.Positions(rows), (r) =>
                List.Transform(List.Positions(rows{r}), (c) =>
                    [PageIndex = pidx, RowIndex = r, ColIndex = c,
                     Value = if rows{r}{c} = null then "" else Text.From(rows{r}{c})]
                )
            ))
        in
            Table.FromRecords(recs, {"PageIndex","RowIndex","ColIndex","Value"})
    ),
    Combined = Table.Combine(ToLong[LongRows]),
    Typed = Table.TransformColumnTypes(Combined, {{"PageIndex", Int64.Type}, {"RowIndex", Int64.Type}, {"ColIndex", Int64.Type}, {"Value", type text}})
in
    Typed
"@
    $wb.Queries.Add($queryName, $formula) | Out-Null

    $stagingSheet = $wb.Worksheets.Add()
    $stagingSheet.Name = 'Staging'

    $connStr = "OLEDB;Provider=Microsoft.Mashup.OleDb.1;Data Source=`$Workbook`$;Location=$queryName;Extended Properties=`"`""
    $qt = $stagingSheet.QueryTables.Add($connStr, $stagingSheet.Range("A1"))
    $qt.CommandType = 2  # xlCmdSql
    $qt.CommandText = "SELECT * FROM [$queryName]"
    $qt.BackgroundQuery = $false
    $qt.FieldNames = $false   # no header row - we own the shape entirely (fixes stray "Column1..." row)
    $qt.Refresh() | Out-Null

    $parseSw = $sw.Elapsed.TotalSeconds
    Out-Line "STAGE writing"

    $used = $stagingSheet.UsedRange
    $rowCount = $used.Rows.Count
    $raw = $used.Value2

    # Build one page -> [rows of (RowIndex,ColIndex,Value)] map from the flat
    # staging data, reading the COM array in bulk (no per-cell COM round trips).
    $pageRows = [System.Collections.Generic.Dictionary[int, System.Collections.Generic.List[object]]]::new()
    for ($r = 1; $r -le $rowCount; $r++) {
        if ($rowCount -eq 1) {
            $pi = [int]$raw[1,1]; $rr = [int]$raw[1,2]; $cc = [int]$raw[1,3]; $vv = [string]$raw[1,4]
        } else {
            $pi = [int]$raw[$r,1]; $rr = [int]$raw[$r,2]; $cc = [int]$raw[$r,3]; $vv = [string]$raw[$r,4]
        }
        if (-not $pageRows.ContainsKey($pi)) { $pageRows[$pi] = [System.Collections.Generic.List[object]]::new() }
        $entry = @{ R = $rr; C = $cc; V = $vv }
        $pageRows[$pi].Add($entry)
    }
    $pageCount = $pageRows.Count

    $qt.Delete()
    $stagingSheet.Delete()
    $wb.Queries.Item($queryName).Delete()

    foreach ($pi in ($pageRows.Keys | Sort-Object)) {
        $cells = $pageRows[$pi]
        $maxR = ($cells | ForEach-Object { $_.R } | Measure-Object -Maximum).Maximum
        $maxC = ($cells | ForEach-Object { $_.C } | Measure-Object -Maximum).Maximum

        $vals = New-Object 'object[,]' ($maxR + 1), ($maxC + 1)
        $fmts = New-Object 'object[,]' ($maxR + 1), ($maxC + 1)
        for ($rr = 0; $rr -le $maxR; $rr++) { for ($cc = 0; $cc -le $maxC; $cc++) { $vals[$rr,$cc] = ''; $fmts[$rr,$cc] = '@' } }

        foreach ($cell in $cells) {
            $num = ConvertFrom-IndoNumber -s $cell.V
            if ($null -ne $num) {
                $vals[$cell.R, $cell.C] = $num.Value
                $fmts[$cell.R, $cell.C] = if ($num.IsPercent) { '0.00%' } else { 'General' }
            } else {
                $vals[$cell.R, $cell.C] = $cell.V
                $fmts[$cell.R, $cell.C] = '@'
            }
        }

        # Add() with no args inserts BEFORE the active sheet, which becomes
        # the new active sheet - looping that reverses page order. Insert
        # after the last existing sheet instead to keep Page 1..N in order.
        $sheet = $wb.Worksheets.Add([System.Reflection.Missing]::Value, $wb.Worksheets.Item($wb.Worksheets.Count))
        $sheet.Name = "Page $pi"
        $dest = $sheet.Range($sheet.Cells.Item(1,1), $sheet.Cells.Item($maxR + 1, $maxC + 1))
        $dest.NumberFormat = $fmts   # set formats first so text-format cells store the string verbatim
        $dest.Value2 = $vals
    }

    foreach ($sh in @($wb.Worksheets)) {
        if ($sh.Name -match '^Sheet\d+$' -and $sh.UsedRange.Cells.Count -eq 1 -and [string]::IsNullOrEmpty($sh.Range("A1").Value2)) {
            $sh.Delete()
        }
    }

    foreach ($q in @($wb.Queries)) { $q.Delete() }
    foreach ($c in @($wb.Connections)) { $c.Delete() }

    $outPath = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Out)
    $outDir = Split-Path $outPath -Parent
    if (-not (Test-Path $outDir)) { New-Item -ItemType Directory -Path $outDir | Out-Null }

    Out-Line "STAGE saving"
    $tempPath = Join-Path $env:TEMP ("majipdf_xlsx_" + [Guid]::NewGuid().ToString('N') + ".xlsx")
    $wb.SaveAs($tempPath, 51)  # xlOpenXMLWorkbook
    $wb.Close($false)
    $wb = $null

    if (Test-Path $outPath) { Remove-Item $outPath -Force }
    Move-Item -Path $tempPath -Destination $outPath -Force

    $excel.Quit()
    $sw.Stop()
    Write-Output ("OK seconds=$([math]::Round($sw.Elapsed.TotalSeconds,2)) parseSeconds=$([math]::Round($parseSw,2)) out=$outPath pages=$pageCount")
    exit 0
}
catch {
    $sw.Stop()
    $oneLine = $_.Exception.Message -replace "[\r\n]+", ' '
    Write-Output ("ERROR seconds=$([math]::Round($sw.Elapsed.TotalSeconds,2)) msg=$oneLine")
    if ($env:MAJIPDF_DEBUG) {
        Write-Output ("LINE=" + $_.InvocationInfo.ScriptLineNumber + " STMT=" + $_.InvocationInfo.Line.Trim())
        Write-Output $_.Exception.ToString()
    }
    exit 1
}
finally {
    try { if ($wb) { $wb.Close($false) } } catch {}
    try { if ($wb) { [Runtime.InteropServices.Marshal]::ReleaseComObject($wb) | Out-Null } } catch {}
    try { if ($excel) { $excel.Quit() } } catch {}
    try { if ($excel) { [Runtime.InteropServices.Marshal]::ReleaseComObject($excel) | Out-Null } } catch {}
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
