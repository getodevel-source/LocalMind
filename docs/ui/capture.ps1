#Requires -Version 5.1
<#
.SYNOPSIS
  Capture the LocalMind desktop window (WebView2) into docs/ui/screens/<state>-<lang>.png
  using PrintWindow (works even when the window is occluded).

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File docs/ui/capture.ps1 -State dashboard-stopped -Lang es
#>
param(
  [string]$State = 'dashboard-stopped',
  [string]$Lang = 'es',
  [string]$OutDir = (Join-Path $PSScriptRoot 'screens')
)

Add-Type @"
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;

public static class WinCap {
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
  [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
  [StructLayout(LayoutKind.Sequential)]
  public struct RECT { public int L, T, R, B; }
}
"@ -ReferencedAssemblies System.Drawing

$procs = Get-Process -Name 'LocalMind' -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 }
if (-not $procs) { Write-Error 'No LocalMind window found (process LocalMind with MainWindowHandle). Is the app running?'; exit 1 }
$hwnd = $procs[0].MainWindowHandle
Write-Host "PID=$($procs[0].Id) HWND=$hwnd Title='$($procs[0].MainWindowTitle)'"

$rect = New-Object WinCap+RECT
[WinCap]::GetWindowRect($hwnd, [ref]$rect) | Out-Null
$w = $rect.R - $rect.L; $h = $rect.B - $rect.T
Write-Host "rect=${w}x${h}"
if ($w -le 0 -or $h -le 0) { Write-Error 'Window rect is empty.'; exit 1 }

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$out = Join-Path $OutDir ("{0}-{1}.png" -f $State, $Lang)

$bmp = New-Object Drawing.Bitmap($w, $h)
$g = [Drawing.Graphics]::FromImage($bmp)
$dc = $g.GetHdc()
try { [WinCap]::PrintWindow($hwnd, $dc, 0) | Out-Null } finally { $g.ReleaseHdc($dc); $g.Dispose() }
$bmp.Save($out, [Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()
$bytes = (Get-Item $out).Length
Write-Host "saved $out ($bytes bytes)"
