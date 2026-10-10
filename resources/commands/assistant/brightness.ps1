param(
    [string]$Action = "get",
    [int]$Value = 0
)

# Screen brightness through WMI. Works on laptops / built-in displays.
# External monitors usually do not support this: exit code 2.
try {
    $levels = Get-CimInstance -Namespace root/WMI -ClassName WmiMonitorBrightness -ErrorAction Stop
    $methods = Get-CimInstance -Namespace root/WMI -ClassName WmiMonitorBrightnessMethods -ErrorAction Stop
} catch {
    exit 2
}
if (-not $levels -or -not $methods) { exit 2 }

$current = [int]($levels | Select-Object -First 1).CurrentBrightness
function Clamp([int]$v) { [Math]::Max(0, [Math]::Min(100, $v)) }

switch ($Action) {
    "set"  { $target = Clamp $Value }
    "up"   { $target = Clamp ($current + $Value) }
    "down" { $target = Clamp ($current - $Value) }
    default { $target = $current }
}

$methods | ForEach-Object {
    Invoke-CimMethod -InputObject $_ -MethodName WmiSetBrightness -Arguments @{ Timeout = 1; Brightness = $target } | Out-Null
}
Write-Output $target
