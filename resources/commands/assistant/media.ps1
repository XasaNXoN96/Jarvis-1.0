param([string]$Key = "playpause")

# Sends a media key (like the keyboard media buttons): works with Spotify, browser players, VLC ...
Add-Type -TypeDefinition @'
using System.Runtime.InteropServices;
public class MediaKey {
  [DllImport("user32.dll")] public static extern void keybd_event(byte bVk, byte bScan, int dwFlags, int dwExtraInfo);
}
'@

switch ($Key) {
    "playpause" { $vk = 0xB3 }
    "next"      { $vk = 0xB0 }
    "prev"      { $vk = 0xB1 }
    "stop"      { $vk = 0xB2 }
    default     { exit 1 }
}
[MediaKey]::keybd_event($vk, 0, 0, 0)
[MediaKey]::keybd_event($vk, 0, 2, 0)
