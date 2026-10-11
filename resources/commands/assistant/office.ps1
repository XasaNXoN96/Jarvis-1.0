# Word / Excel through COM automation (the program must be installed). Text arrives as Base64 (UTF-8).
param(
    [string]$App,
    [string]$Action,
    [string]$Arg1 = "",
    [string]$Arg2 = "",
    [string]$Arg3 = ""
)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

function Decode([string]$s) {
    if ([string]::IsNullOrEmpty($s)) { return '' }
    [System.Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($s))
}

# the running instance, or a new visible one
function Get-OfficeApp([string]$prog) {
    try { return [System.Runtime.InteropServices.Marshal]::GetActiveObject("$prog.Application") }
    catch {
        try { $a = New-Object -ComObject "$prog.Application"; $a.Visible = $true; return $a }
        catch { [Console]::Error.WriteLine("NOTINSTALLED"); exit 2 }
    }
}

try {
    switch ("$App/$Action") {
        "word/ensure" {
            $w = Get-OfficeApp "Word"
            if ($w.Documents.Count -eq 0) { [void]$w.Documents.Add() }
        }
        "word/new" {
            $w = Get-OfficeApp "Word"
            [void]$w.Documents.Add()
        }
        "word/text" {
            $w = Get-OfficeApp "Word"
            if ($w.Documents.Count -eq 0) { [void]$w.Documents.Add() }
            $parts = (Decode $Arg1) -split "`n"
            for ($i = 0; $i -lt $parts.Count; $i++) {
                if ($parts[$i] -ne '') { $w.Selection.TypeText($parts[$i]) }
                if ($i -lt $parts.Count - 1) { $w.Selection.TypeParagraph() }
            }
        }
        "excel/ensure" {
            $x = Get-OfficeApp "Excel"
            if ($x.Workbooks.Count -eq 0) { [void]$x.Workbooks.Add() }
        }
        "excel/new" {
            $x = Get-OfficeApp "Excel"
            [void]$x.Workbooks.Add()
        }
        "excel/value" {
            $x = Get-OfficeApp "Excel"
            if ($x.Workbooks.Count -eq 0) { [void]$x.Workbooks.Add() }
            $v = Decode $Arg2
            if ($Arg3 -eq 'n') { $x.Range($Arg1).Value2 = [double]::Parse($v, [System.Globalization.CultureInfo]::InvariantCulture) }
            else { $x.Range($Arg1).Value2 = $v }
        }
        "excel/formula" {
            $x = Get-OfficeApp "Excel"
            if ($x.Workbooks.Count -eq 0) { [void]$x.Workbooks.Add() }
            $x.Range($Arg1).Formula = (Decode $Arg2)
            Write-Output $x.Range($Arg1).Text
        }
        "excel/get" {
            $x = Get-OfficeApp "Excel"
            Write-Output $x.Range($Arg1).Text
        }
        default { [Console]::Error.WriteLine("unknown action $App/$Action"); exit 3 }
    }
}
catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 1
}
