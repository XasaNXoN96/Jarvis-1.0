# Word / Excel through COM automation (the program must be installed). Text arrives as Base64 (UTF-8).
param(
    [string]$App,
    [string]$Action,
    [string]$Arg1 = "",
    [string]$Arg2 = "",
    [string]$Arg3 = "",
    [switch]$Strict,          # "work mode": never start a program or create a document, only use the one that is open
    [string]$Expect = ""      # Base64 of the document name the work mode is bound to
)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

function Decode([string]$s) {
    if ([string]::IsNullOrEmpty($s)) { return '' }
    [System.Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($s))
}

# "@Documents\name.docx" -> the real folder (Documents, Desktop or Downloads of this user)
function Expand-Place([string]$p) {
    if ($p -match '^@(Documents|Desktop|Downloads)\\(.*)$') {
        $dir = switch ($Matches[1]) {
            'Documents' { [Environment]::GetFolderPath('MyDocuments') }
            'Desktop'   { [Environment]::GetFolderPath('Desktop') }
            'Downloads' { Join-Path $env:USERPROFILE 'Downloads' }
        }
        return (Join-Path $dir $Matches[2])
    }
    return $p
}

# the running instance, or a new visible one. In strict mode only a running instance is used:
# a program that was closed is reported (exit 4), it is not started again behind the user's back.
function Get-OfficeApp([string]$prog) {
    try { return [System.Runtime.InteropServices.Marshal]::GetActiveObject("$prog.Application") }
    catch {
        if ($Strict) { [Console]::Error.WriteLine("NOAPP"); exit 4 }
        try { $a = New-Object -ComObject "$prog.Application"; $a.Visible = $true; return $a }
        catch { [Console]::Error.WriteLine("NOTINSTALLED"); exit 2 }
    }
}

# strict mode: the active document must be the one the work mode started with
function Assert-Doc([string]$name, [string]$actual) {
    if (-not $Strict) { return }
    if ([string]::IsNullOrEmpty($actual)) { [Console]::Error.WriteLine("NODOC"); exit 5 }
    $want = Decode $Expect
    if ($want -ne '' -and $want -ne $actual) { [Console]::Error.WriteLine("DOCCHANGED:" + $actual); exit 6 }
}

# the text of a document the way Jarvis compares it: paragraphs are "`n", no trailing whitespace
function Norm-Text([string]$s) {
    return ($s -replace "`r`n", "`n" -replace "`r", "`n").TrimEnd()
}
function Text-Hash([string]$s) {
    $sha = [System.Security.Cryptography.SHA256]::Create()
    $bytes = $sha.ComputeHash([System.Text.Encoding]::UTF8.GetBytes((Norm-Text $s)))
    return (($bytes | ForEach-Object { $_.ToString('x2') }) -join '')
}

function Word-Doc($w) {
    if ($w.Documents.Count -eq 0) { return $null }
    return $w.ActiveDocument
}
function Excel-Book($x) {
    if ($x.Workbooks.Count -eq 0) { return $null }
    return $x.ActiveWorkbook
}

# the text window of Notepad that is open (new Notepad can keep several windows: the first with a title)
function Get-Notepad {
    Get-Process -Name notepad -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
}

function Send-KeysTo($proc, [string]$keys) {
    $sh = New-Object -ComObject WScript.Shell
    [void]$sh.AppActivate($proc.Id)
    Start-Sleep -Milliseconds 300
    $sh.SendKeys($keys)
}

# SendKeys treats + ^ % ~ ( ) { } [ ] as commands: escape them in a typed path
function Escape-Keys([string]$s) {
    return [regex]::Replace($s, '[+^%~(){}\[\]]', { param($m) '{' + $m.Value + '}' })
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
            if ($Strict) { $d = Word-Doc $w; Assert-Doc "Word" $(if ($d) { $d.Name } else { '' }) }
            elseif ($w.Documents.Count -eq 0) { [void]$w.Documents.Add() }
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
            if ($Strict) { $b = Excel-Book $x; Assert-Doc "Excel" $(if ($b) { $b.Name } else { '' }) }
            elseif ($x.Workbooks.Count -eq 0) { [void]$x.Workbooks.Add() }
            $v = Decode $Arg2
            if ($Arg3 -eq 'n') { $x.Range($Arg1).Value2 = [double]::Parse($v, [System.Globalization.CultureInfo]::InvariantCulture) }
            else { $x.Range($Arg1).Value2 = $v }
        }
        "excel/formula" {
            $x = Get-OfficeApp "Excel"
            if ($Strict) { $b = Excel-Book $x; Assert-Doc "Excel" $(if ($b) { $b.Name } else { '' }) }
            elseif ($x.Workbooks.Count -eq 0) { [void]$x.Workbooks.Add() }
            $x.Range($Arg1).Formula = (Decode $Arg2)
            Write-Output $x.Range($Arg1).Text
        }
        "excel/get" {
            $x = Get-OfficeApp "Excel"
            Write-Output $x.Range($Arg1).Text
        }
        # ---------- work mode: the document that is open ----------
        # info: "count|name|full path|saved(1/0)" of the active document ("0" when none is open)
        "word/info" {
            $w = Get-OfficeApp "Word"
            $d = Word-Doc $w
            if ($d) { Write-Output ("1|" + $d.Name + "|" + $d.FullName + "|" + $(if ($d.Saved) { 1 } else { 0 }) + "|" + $(if ($d.Path) { 1 } else { 0 })) } else { Write-Output "0" }
        }
        "excel/info" {
            $x = Get-OfficeApp "Excel"
            $b = Excel-Book $x
            if ($b) { Write-Output ("1|" + $b.Name + "|" + $b.FullName + "|" + $(if ($b.Saved) { 1 } else { 0 }) + "|" + $(if ($b.Path) { 1 } else { 0 })) } else { Write-Output "0" }
        }
        "notepad/info" {
            $p = Get-Notepad
            if ($p) { Write-Output ("1|" + $p.MainWindowTitle + "||" + $(if ($p.MainWindowTitle.StartsWith('*')) { 0 } else { 1 }) + "|0") } else { Write-Output "0" }
        }
        "word/open" {
            $w = Get-OfficeApp "Word"
            [void]$w.Documents.Open((Decode $Arg1))
        }
        "excel/open" {
            $x = Get-OfficeApp "Excel"
            [void]$x.Workbooks.Open((Decode $Arg1))
        }
        # the active cell (Excel): write the value and move down, like typing in a column
        "excel/active" {
            $x = Get-OfficeApp "Excel"
            $b = Excel-Book $x
            Assert-Doc "Excel" $(if ($b) { $b.Name } else { '' })
            if (-not $b) { [void]$x.Workbooks.Add() }
            $v = Decode $Arg1
            $cell = $x.ActiveCell
            if ($Arg2 -eq 'n') { $cell.Value2 = [double]::Parse($v, [System.Globalization.CultureInfo]::InvariantCulture) } else { $cell.Value2 = $v }
            Write-Output $cell.Address($false, $false)
            [void]$cell.Offset(1, 0).Select()
        }
        # formatting (Word): bold_on/off, italic_on/off, underline_on/off, left/center/right/justify, h1, h2, normal, bullets, numbers, undo, size
        "word/format" {
            $w = Get-OfficeApp "Word"
            $d = Word-Doc $w
            Assert-Doc "Word" $(if ($d) { $d.Name } else { '' })
            $s = $w.Selection
            switch ($Arg1) {
                "bold_on"      { $s.Font.Bold = 1 }
                "bold_off"     { $s.Font.Bold = 0 }
                "italic_on"    { $s.Font.Italic = 1 }
                "italic_off"   { $s.Font.Italic = 0 }
                "underline_on" { $s.Font.Underline = 1 }
                "underline_off"{ $s.Font.Underline = 0 }
                "left"         { $s.ParagraphFormat.Alignment = 0 }
                "center"       { $s.ParagraphFormat.Alignment = 1 }
                "right"        { $s.ParagraphFormat.Alignment = 2 }
                "justify"      { $s.ParagraphFormat.Alignment = 3 }
                "h1"           { $s.Style = $d.Styles.Item(-2) }
                "h2"           { $s.Style = $d.Styles.Item(-3) }
                "normal"       { $s.Style = $d.Styles.Item(-1) }
                "bullets"      { $s.Range.ListFormat.ApplyBulletDefault() }
                "numbers"      { $s.Range.ListFormat.ApplyNumberDefault() }
                "undo"         { [void]$d.Undo(1) }
                "size"         { $s.Font.Size = [double]$Arg2 }
                default        { [Console]::Error.WriteLine("unknown format $Arg1"); exit 3 }
            }
        }
        # the text of the active document (Base64 of UTF-8, paragraphs "`n"): for the AI
        "word/read" {
            $w = Get-OfficeApp "Word"
            $d = Word-Doc $w
            if (-not $d) { [Console]::Error.WriteLine("NODOC"); exit 5 }
            Assert-Doc "Word" $d.Name
            Write-Output ([Convert]::ToBase64String([System.Text.Encoding]::UTF8.GetBytes((Norm-Text $d.Content.Text))))
        }
        # replace the whole text of the active document; Arg1 = new text (Base64), Arg2 = hash of the text it was made from.
        # A document that changed meanwhile is not touched (exit 10). Prints the hash of the text after the replacement.
        "word/replace" {
            $w = Get-OfficeApp "Word"
            $d = Word-Doc $w
            if (-not $d) { [Console]::Error.WriteLine("NODOC"); exit 5 }
            Assert-Doc "Word" $d.Name
            if ((Text-Hash $d.Content.Text) -ne $Arg2) { [Console]::Error.WriteLine("TEXTCHANGED"); exit 10 }
            $new = ((Decode $Arg1) -replace "`r`n", "`n") -replace "`n", "`r"
            $d.Content.Text = $new
            Write-Output (Text-Hash $d.Content.Text)
        }
        # save: a document that already has a file is saved in place; a new one needs a name (exit 7 = NEEDNAME)
        "word/save" {
            $w = Get-OfficeApp "Word"
            $d = Word-Doc $w
            Assert-Doc "Word" $(if ($d) { $d.Name } else { '' })
            if (-not $d.Path) { [Console]::Error.WriteLine("NEEDNAME"); exit 7 }
            $d.Save()
            Write-Output $d.FullName
        }
        "excel/save" {
            $x = Get-OfficeApp "Excel"
            $b = Excel-Book $x
            Assert-Doc "Excel" $(if ($b) { $b.Name } else { '' })
            if (-not $b.Path) { [Console]::Error.WriteLine("NEEDNAME"); exit 7 }
            $b.Save()
            Write-Output $b.FullName
        }
        # save as: the full path arrives in Arg1 (Base64); an existing file is never replaced here (exit 8 = EXISTS)
        "word/saveas" {
            $w = Get-OfficeApp "Word"
            $d = Word-Doc $w
            Assert-Doc "Word" $(if ($d) { $d.Name } else { '' })
            $path = Expand-Place (Decode $Arg1)
            if ((Test-Path -LiteralPath $path) -and $Arg2 -ne 'overwrite') { [Console]::Error.WriteLine("EXISTS"); exit 8 }
            $dir = Split-Path -Parent $path
            if (-not (Test-Path -LiteralPath $dir)) { [void](New-Item -ItemType Directory -Force $dir) }
            try { $d.SaveAs2($path) } catch { $d.SaveAs($path) }
            Write-Output $d.FullName
        }
        "excel/saveas" {
            $x = Get-OfficeApp "Excel"
            $b = Excel-Book $x
            Assert-Doc "Excel" $(if ($b) { $b.Name } else { '' })
            $path = Expand-Place (Decode $Arg1)
            if ((Test-Path -LiteralPath $path) -and $Arg2 -ne 'overwrite') { [Console]::Error.WriteLine("EXISTS"); exit 8 }
            $dir = Split-Path -Parent $path
            if (-not (Test-Path -LiteralPath $dir)) { [void](New-Item -ItemType Directory -Force $dir) }
            $x.DisplayAlerts = $false
            try { $b.SaveAs($path, 51) } finally { $x.DisplayAlerts = $true }
            Write-Output $b.FullName
        }
        # Notepad has no automation: Ctrl+S (and the path typed into the "Save as" window when the file is new)
        "notepad/save" {
            $p = Get-Notepad
            if (-not $p) { [Console]::Error.WriteLine("NOAPP"); exit 4 }
            $path = Expand-Place (Decode $Arg1)
            if ($path -ne '' -and (Test-Path -LiteralPath $path) -and $Arg2 -ne 'overwrite') { [Console]::Error.WriteLine("EXISTS"); exit 8 }
            Send-KeysTo $p '^s'
            if ($path -ne '') {
                Start-Sleep -Milliseconds 900
                Send-KeysTo $p ((Escape-Keys $path) + '{ENTER}')
                Start-Sleep -Milliseconds 700
                if (-not (Test-Path -LiteralPath $path)) { [Console]::Error.WriteLine("NOTSAVED"); exit 9 }
            }
            Write-Output $path
        }
        # close the document WITHOUT saving; prints the file path when the document was stored on the disk
        "word/discard" {
            $w = Get-OfficeApp "Word"
            $d = Word-Doc $w
            Assert-Doc "Word" $(if ($d) { $d.Name } else { '' })
            $full = if ($d.Path) { $d.FullName } else { '' }
            $d.Close(0)
            Write-Output $full
        }
        "excel/discard" {
            $x = Get-OfficeApp "Excel"
            $b = Excel-Book $x
            Assert-Doc "Excel" $(if ($b) { $b.Name } else { '' })
            $full = if ($b.Path) { $b.FullName } else { '' }
            $x.DisplayAlerts = $false
            try { $b.Close($false) } finally { $x.DisplayAlerts = $true }
            Write-Output $full
        }
        # Notepad: close the window and answer its "save?" question with "Don't save" (Russian and English keys)
        "notepad/discard" {
            $p = Get-Notepad
            if (-not $p) { [Console]::Error.WriteLine("NOAPP"); exit 4 }
            [void]$p.CloseMainWindow()
            Start-Sleep -Milliseconds 700
            if (-not $p.HasExited) {
                Send-KeysTo $p 'н'
                Start-Sleep -Milliseconds 400
                if (-not $p.HasExited) { Send-KeysTo $p 'n' }
                Start-Sleep -Milliseconds 400
            }
            Write-Output $(if ($p.HasExited) { 'closed' } else { 'asking' })
        }
        # recycle bin (never a permanent delete)
        "file/recycle" {
            $path = Expand-Place (Decode $Arg1)
            if (-not (Test-Path -LiteralPath $path)) { Write-Output 'absent'; break }
            Add-Type -AssemblyName Microsoft.VisualBasic
            [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteFile($path, 'OnlyErrorDialogs', 'SendToRecycleBin')
            Write-Output 'recycled'
        }
        # a file by a spoken name in the usual places; Arg1 = Base64 words, Arg2 = extensions "docx,doc"
        "file/find" {
            $words = (Decode $Arg1).ToLower().Replace('ё', 'е') -split '\s+' | Where-Object { $_ -ne '' }
            $exts = $Arg2 -split ','
            $roots = @([Environment]::GetFolderPath('MyDocuments'), [Environment]::GetFolderPath('Desktop'), (Join-Path $env:USERPROFILE 'Downloads'))
            $best = $null; $bestScore = 0
            foreach ($r in $roots) {
                if (-not (Test-Path -LiteralPath $r)) { continue }
                Get-ChildItem -LiteralPath $r -Recurse -Depth 3 -File -ErrorAction SilentlyContinue |
                    Where-Object { $exts -contains $_.Extension.TrimStart('.').ToLower() } |
                    ForEach-Object {
                        $n = [System.IO.Path]::GetFileNameWithoutExtension($_.Name).ToLower().Replace('ё', 'е')
                        $hit = 0; foreach ($w in $words) { if ($n.Contains($w)) { $hit++ } }
                        if ($hit -eq $words.Count -and $hit -gt 0) {
                            $score = $hit * 1000 - $n.Length
                            if ($score -gt $bestScore) { $bestScore = $score; $script:best = $_.FullName }
                        }
                    }
            }
            if ($script:best) { Write-Output $script:best } else { Write-Output '' }
        }
        default { [Console]::Error.WriteLine("unknown action $App/$Action"); exit 3 }
    }
}
catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 1
}
