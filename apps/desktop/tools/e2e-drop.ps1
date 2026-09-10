# End-to-end drop verification for the OpenConvert desktop shell.
#
# Drags a real PNG out of an Explorer window and releases it over the running
# app with injected mouse input, then presses Convert and asserts an output
# file appears. Every participant is production: the shell drag source, wry's
# registered IDropTarget, tauri's drag event, DropZone, the IPC commands, the
# confined worker, the receipt.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File tools\e2e-drop.ps1
#
# Optional -Mode control runs the same drag between two Explorer windows, to
# separate "the harness works" from "the app accepts the drop" if something
# fails.
#
# Requirements (all checked where possible):
#   - An UNLOCKED interactive session. While LogonUI is running, injected
#     input lands on the lock screen and nothing else can work. The script
#     aborts if it detects one.
#   - The primary display must be 2880x1800 physical, or $PrimaryW/H below
#     must be edited to match. Absolute injected moves are normalized against
#     these numbers; wrong values put the cursor in the wrong place.
#   - Nothing must cover the app or the Explorer windows while it runs.
#
# Lessons already paid for, encoded here:
#   - Absolute injected moves are deterministic; RELATIVE ones go through the
#     pointer-speed curve and will miss. Do not "simplify" Move-Verified.
#   - Every move is verified against GetCursorPos. A silent miss must be a
#     loud failure.
#   - SetForegroundWindow from a background process fails silently. Focus is
#     taken by clicking the target's title bar instead.
#   - Explorer names items without their extension when "hide extensions" is
#     on, so the fixture matches "photo" as well as "photo.png".
#   - Explorer windows are closed with WM_CLOSE. Never Stop-Process
#     explorer.exe: that kills the shell, not the window.
param(
    [string]$ExePath,
    [string]$TestDir,
    [ValidateSet("app", "control")]
    [string]$Mode = "app",
    [int]$PrimaryW = 2880,
    [int]$PrimaryH = 1800
)

$ErrorActionPreference = "Stop"

Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class Input {
    [DllImport("user32.dll")] private static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")]
    private static extern void mouse_event(uint f, int dx, int dy, uint d, UIntPtr extra);
    [DllImport("user32.dll")] public static extern bool GetCursorPos(out P p);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr a, int X, int Y, int cx, int cy, uint f);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
    public struct P { public int X; public int Y; }
    public static void InitDPI() { SetProcessDPIAware(); }
    public static void LeftDown() { mouse_event(0x0002,0,0,0,UIntPtr.Zero); }
    public static void LeftUp()   { mouse_event(0x0004,0,0,0,UIntPtr.Zero); }
    public static void MoveTo(int x, int y) {
        mouse_event(0x0001 | 0x8000, (int)((long)x*65536/PrimaryW), (int)((long)y*65536/PrimaryH), 0, UIntPtr.Zero);
    }
    public static bool Pos(ref int x, ref int y) {
        P p; if (!GetCursorPos(out p)) return false; x = p.X; y = p.Y; return true;
    }
    public static bool CloseWindow(IntPtr h) { return PostMessage(h, 0x0010, IntPtr.Zero, IntPtr.Zero); }
    public static int PrimaryW;
    public static int PrimaryH;
}
"@
[Input]::PrimaryW = $PrimaryW
[Input]::PrimaryH = $PrimaryH
[Input]::InitDPI()

# --- Session guard ---------------------------------------------------------
Get-Process LogonUI -ErrorAction SilentlyContinue | ForEach-Object {
    Write-Output "ABORT: the workstation is locked (LogonUI is running)."
    Write-Output "Injected input reaches the lock screen, not the desktop."
    Write-Output "Unlock the session and run this again."
    exit 10
}

if (-not $ExePath) { $ExePath = Join-Path $PSScriptRoot "..\src-tauri\target\release\openconvert-desktop.exe" }
if (-not (Test-Path $ExePath)) {
    Write-Output "ABORT: $ExePath not found. Build it first:"
    Write-Output "  cd apps/desktop && npm run build"
    Write-Output "  cd src-tauri && cargo build --release --features custom-protocol"
    exit 10
}
if (-not $TestDir) { $TestDir = Join-Path $env:TEMP "openconvert-e2e-drop" }

Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes

# --- Fixture ---------------------------------------------------------------
Remove-Item -Recurse -Force $TestDir -ErrorAction SilentlyContinue | Out-Null
New-Item -ItemType Directory -Force $TestDir | Out-Null
# A real 8x8 red PNG: small enough to inline, real enough to decode+encode.
$pngBase64 = "iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAFklEQVR4nGP8z8Dwn4GBgYGJgYEBAA3bAgMORn9OAAAAAElFTkSuQmCC"
$pngPath = Join-Path $TestDir "photo.png"
[IO.File]::WriteAllBytes($pngPath, [Convert]::FromBase64String($pngBase64))
Write-Output "fixture: $pngPath"

# --- Helpers ---------------------------------------------------------------
function Find-FolderWindow([string]$namePart) {
    $root = [System.Windows.Automation.AutomationElement]::RootElement
    $clsCond = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ClassNameProperty, "CabinetWClass")
    foreach ($w in $root.FindAll([System.Windows.Automation.TreeScope]::Children, $clsCond)) {
        if ($w.Current.Name -like "*$namePart*") { return $w }
    }
    return $null
}

function Get-AppWindow([int]$procId) {
    $root = [System.Windows.Automation.AutomationElement]::RootElement
    $cond = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ProcessIdProperty, $procId)
    return $root.FindFirst([System.Windows.Automation.TreeScope]::Children, $cond)
}

function Get-NamedElements($win) {
    $all = $win.FindAll(
        [System.Windows.Automation.TreeScope]::Descendants,
        [System.Windows.Automation.Condition]::TrueCondition)
    $list = @()
    foreach ($el in $all) { if ($el.Current.Name) { $list += $el } }
    return ,$list
}

function Move-Verified([int]$tx, [int]$ty) {
    for ($i = 0; $i -lt 6; $i++) {
        [Input]::MoveTo($tx, $ty)
        Start-Sleep -Milliseconds 60
        $cx = 0; $cy = 0
        [Input]::Pos([ref]$cx, [ref]$cy) | Out-Null
        if ([Math]::Abs($cx - $tx) -le 3 -and [Math]::Abs($cy - $ty) -le 3) { return $true }
    }
    return $false
}

function Find-Item($window, [string]$stem) {
    $listCond = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::ListItem)
    foreach ($attempt in 1..5) {
        foreach ($it in $window.FindAll([System.Windows.Automation.TreeScope]::Descendants, $listCond)) {
            # Explorer hides known extensions by default.
            if ($it.Current.Name -eq $stem -or $it.Current.Name -eq "$stem.png") { return $it }
        }
        Start-Sleep -Seconds 2
    }
    return $null
}

function Invoke-ShellDrag($srcWindow, [int]$sx, [int]$sy, [int]$tx, [int]$ty) {
    # Focus by clicking the title bar: SetForegroundWindow fails silently
    # from a background process.
    $ok = Move-Verified ($sx - 100) ($sy - 60)
    if (-not $ok) { return "could not reach the source window" }
    [Input]::LeftDown(); Start-Sleep -Milliseconds 80; [Input]::LeftUp()
    Start-Sleep -Milliseconds 600

    $ok = Move-Verified $sx $sy
    if (-not $ok) { return "could not reach the item" }
    Start-Sleep -Milliseconds 300

    [Input]::LeftDown()
    Start-Sleep -Milliseconds 350

    # Cross the drag threshold with one deterministic absolute step.
    $ok = Move-Verified ($sx + 5) ($sy + 4)
    if (-not $ok) { return "threshold move failed" }
    Start-Sleep -Milliseconds 250

    $steps = 10
    for ($i = 1; $i -le $steps; $i++) {
        $x = [int](($sx + 5) + ($tx - $sx - 5) * $i / $steps)
        $y = [int](($sy + 4) + ($ty - $sy - 4) * $i / $steps)
        $ok = Move-Verified $x $y
        if (-not $ok) { return "travel step $i failed at $x,$y" }
        Start-Sleep -Milliseconds 20
    }
    Start-Sleep -Milliseconds 700
    [Input]::LeftUp()
    return $null
}

# --- Windows ---------------------------------------------------------------
if ($Mode -eq "app") {
    # The webview builds its accessibility tree lazily; force it on so UIA
    # can read the content (the same tree a screen reader would get).
    $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--force-renderer-accessibility"
    $p = Start-Process -FilePath $ExePath -PassThru
    Start-Sleep -Seconds 6
    if ($p.HasExited) { Write-Output "FAIL: app exited immediately ($($p.ExitCode))"; exit 1 }
    Write-Output "app running pid=$($p.Id)"

    $appWin = Get-AppWindow $p.Id
    if (-not $appWin) { Write-Output "FAIL: no app window"; Stop-Process -Id $p.Id -Force; exit 1 }
    [Input]::SetWindowPos([IntPtr]$appWin.Current.NativeWindowHandle, [IntPtr]::Zero, 900, 150, 880, 640, 0) | Out-Null
    $appHwnd = [IntPtr]$appWin.Current.NativeWindowHandle
}
else {
    $p = $null
}

Start-Process explorer.exe -ArgumentList "`"$TestDir`"" | Out-Null
Start-Sleep -Seconds 4
$srcWin = Find-FolderWindow "openconvert-e2e-drop"
if (-not $srcWin) { Write-Output "FAIL: Explorer window for $TestDir"; if ($p) { Stop-Process -Id $p.Id -Force }; exit 1 }
[Input]::SetWindowPos([IntPtr]$srcWin.Current.NativeWindowHandle, [IntPtr]::Zero, 100, 150, 700, 560, 0) | Out-Null
Start-Sleep -Milliseconds 500

$dstDir = $TestDir
$dstWin = $null
if ($Mode -eq "control") {
    $dstDir = Join-Path $env:TEMP "openconvert-e2e-dest"
    Remove-Item -Recurse -Force $dstDir -ErrorAction SilentlyContinue | Out-Null
    New-Item -ItemType Directory -Force $dstDir | Out-Null
    Start-Process explorer.exe -ArgumentList "`"$dstDir`"" | Out-Null
    Start-Sleep -Seconds 4
    $dstWin = Find-FolderWindow "openconvert-e2e-dest"
    if (-not $dstWin) { Write-Output "FAIL: destination Explorer window"; if ($p) { Stop-Process -Id $p.Id -Force }; exit 1 }
    [Input]::SetWindowPos([IntPtr]$dstWin.Current.NativeWindowHandle, [IntPtr]::Zero, 1500, 150, 700, 560, 0) | Out-Null
    Start-Sleep -Milliseconds 500
}

# --- Drop point ------------------------------------------------------------
$item = Find-Item $srcWin "photo"
if (-not $item) {
    Write-Output "FAIL: the photo item is not visible in Explorer"
    if ($p) { Stop-Process -Id $p.Id -Force }
    [Input]::CloseWindow([IntPtr]$srcWin.Current.NativeWindowHandle) | Out-Null
    exit 1
}
$b = $item.Current.BoundingRectangle
$sx = [int]($b.X + $b.Width / 2)
$sy = [int]($b.Y + $b.Height / 2)
Write-Output ("item centre: {0},{1}" -f $sx, $sy)

if ($Mode -eq "app") {
    Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class WR {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
}
"@
    $rr = New-Object WR+RECT
    [WR]::GetWindowRect($appHwnd, [ref]$rr) | Out-Null
    $tx = [int](($rr.L + $rr.R) / 2)
    $ty = [int](($rr.T + $rr.B) / 2)
}
else {
    $rr = New-Object WR+RECT
    [WR]::GetWindowRect([IntPtr]$dstWin.Current.NativeWindowHandle, [ref]$rr) | Out-Null
    $tx = [int](($rr.L + $rr.R) / 2)
    $ty = [int](($rr.T + 160 + $rr.B) / 2)
}
Write-Output ("release point: {0},{1}" -f $tx, $ty)

# --- Drag ------------------------------------------------------------------
$err = Invoke-ShellDrag $srcWin $sx $sy $tx $ty
if ($err) {
    Write-Output "FAIL: $err"
    if ($p) { Stop-Process -Id $p.Id -Force }
    [Input]::CloseWindow([IntPtr]$srcWin.Current.NativeWindowHandle) | Out-Null
    if ($dstWin) { [Input]::CloseWindow([IntPtr]$dstWin.Current.NativeWindowHandle) | Out-Null }
    exit 2
}
Write-Output "released"
Start-Sleep -Seconds 3

# --- Verify ----------------------------------------------------------------
$failed = $false
if ($Mode -eq "control") {
    $received = @(Get-ChildItem $dstDir)
    Write-Output ("destination holds: {0} item(s)" -f $received.Count)
    if ($received.Count -eq 0) { $failed = $true }
}
else {
    $win2 = Get-AppWindow $p.Id
    $named = Get-NamedElements $win2
    Write-Output "--- UI tree after drop ---"
    foreach ($el in $named) {
        Write-Output ("{0}: {1}" -f $el.Current.ControlType.ProgrammaticName, $el.Current.Name)
    }

    $convert = $named | Where-Object { $_.Current.Name -like "Convert*" } | Select-Object -First 1
    if (-not $convert) {
        Write-Output "FAIL: no Convert button -- the drop did not produce a preview"
        $failed = $true
    }
    else {
        [Input]::MoveTo($tx, ($ty + 200)) | Out-Null
        [Input]::LeftDown(); Start-Sleep -Milliseconds 80; [Input]::LeftUp()
        Start-Sleep -Milliseconds 400
        $convert.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
        Write-Output "convert invoked"
        Start-Sleep -Seconds 5

        $after = @(Get-ChildItem $TestDir)
        $newFiles = @($after | Where-Object { $_.Name -ne "photo.png" })
        Write-Output "--- files after conversion ---"
        foreach ($f in $after) { Write-Output ("{0}  {1} bytes" -f $f.Name, $f.Length) }

        $win3 = Get-AppWindow $p.Id
        Write-Output "--- UI tree after convert ---"
        foreach ($el in (Get-NamedElements $win3)) {
            Write-Output ("{0}: {1}" -f $el.Current.ControlType.ProgrammaticName, $el.Current.Name)
        }
        if ($newFiles.Count -eq 0) { $failed = $true }
    }
    Stop-Process -Id $p.Id -Force | Out-Null
}

[Input]::CloseWindow([IntPtr]$srcWin.Current.NativeWindowHandle) | Out-Null
if ($dstWin) { [Input]::CloseWindow([IntPtr]$dstWin.Current.NativeWindowHandle) | Out-Null }

if ($failed) { Write-Output "FAIL"; exit 3 }
Write-Output "PASS: the full drop-to-output path works"
exit 0
