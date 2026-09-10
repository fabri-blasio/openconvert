<#
.SYNOPSIS
    Removes OpenConvert and, if you ask it to, everything OpenConvert wrote.

.DESCRIPTION
    The NSIS uninstaller the installer registers already does the right thing:
    it removes the program, and its "delete application data" checkbox is wired
    (through installer/hooks.nsh) to the directory this app actually writes to.
    This script exists for the cases that checkbox cannot reach:

      * The program was removed some other way -- the folder was deleted, an
        image was reset, the machine was migrated -- and the per-user data was
        left behind with nothing left to offer to remove it.
      * Several Windows accounts used one per-machine install. The uninstaller
        clears the profile of whoever runs it and cannot touch the others;
        each of them can run this.
      * Somebody wants to SEE what would be removed before anything is, and
        keep the output.

    It removes exactly two things, both named as literal paths:

      1. The install, by invoking the uninstaller Windows has registered for it.
      2. %LOCALAPPDATA%\OpenConvert -- the settings, history, receipts database,
         saved recipes, signature library and downloaded models. Only with
         -RemoveData, and it says what is in there first.

    NOTHING ELSE. It composes no path from anything it reads, deletes nothing
    outside those two, and touches no file you converted: outputs are written
    where you chose, they are your files, and an uninstaller that went looking
    for them would be a bug with a very long tail.

.PARAMETER RemoveData
    Also delete %LOCALAPPDATA%\OpenConvert. Off by default -- reinstalling keeps
    your settings and history, which is the behaviour people expect and the one
    that is not recoverable if it is wrong.

.PARAMETER KeepProgram
    Leave the installed program alone and only act on the data. Useful for
    "reset it to how it shipped" and for a second user account on a machine
    where the program itself is already gone.

.PARAMETER WhatIf
    Show what would happen. Deletes nothing, runs no uninstaller.

.EXAMPLE
    .\uninstall.ps1 -WhatIf
    Lists what is installed and what is on disk. Changes nothing.

.EXAMPLE
    .\uninstall.ps1 -RemoveData
    Removes the program and everything it wrote for the current user.

.NOTES
    Windows only. On macOS delete OpenConvert.app and
    ~/Library/Application Support/OpenConvert; on Linux remove the package and
    ${XDG_STATE_HOME:-~/.local/state}/openconvert. Both are printed by
    -WhatIf when run on those platforms under PowerShell 7.
#>

[CmdletBinding(SupportsShouldProcess = $true, ConfirmImpact = 'High')]
param(
    [switch] $RemoveData,
    [switch] $KeepProgram
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# The one name this script knows. Everything below is built from it and from
# environment variables Windows owns -- never from output the script has read.
$ProductName = 'OpenConvert'

function Write-Step { param([string] $Text) Write-Host "  $Text" }
function Write-Head { param([string] $Text) Write-Host ''; Write-Host $Text -ForegroundColor Cyan }

# --- where the data lives ---------------------------------------------------
#
# This has to agree with `openconvert_run::state::paths::state_dir`, which is
# %LOCALAPPDATA%\OpenConvert on Windows -- NOT %APPDATA%\<bundle-id>, which is
# where Tauri's built-in checkbox looks and where this app has never written a
# byte. That mismatch is the exact reason installer/hooks.nsh exists, and
# getting it wrong here would reintroduce the same silent no-op.
$DataDir = Join-Path $env:LOCALAPPDATA $ProductName

# What is under it, so the summary can say what is about to go rather than
# printing a path and hoping the reader knows.
$DataContents = @(
    @{ Path = 'config.toml'; What = 'your settings' }
    @{ Path = 'journal';     What = 'the conversion history' }
    @{ Path = 'receipts';    What = 'the receipts database' }
    @{ Path = 'recipes';     What = 'saved recipes' }
    @{ Path = 'signatures';  What = 'the signature library' }
    @{ Path = 'models';      What = 'downloaded AI models' }
    @{ Path = 'work';        What = 'temporary working files' }
)

function Get-DirectorySize {
    param([string] $Path)
    if (-not (Test-Path -LiteralPath $Path)) { return 0 }
    try {
        $files = Get-ChildItem -LiteralPath $Path -Recurse -File -Force -ErrorAction Stop
        if ($null -eq $files) { return 0 }
        return ($files | Measure-Object -Property Length -Sum).Sum
    } catch {
        # A directory we cannot walk is still a directory we can report. A size
        # is a nicety; refusing to run because one could not be measured is not.
        return -1
    }
}

function Format-Size {
    param([long] $Bytes)
    if ($Bytes -lt 0) { return 'size unknown' }
    if ($Bytes -lt 1024) { return "$Bytes B" }
    $units = @('KB', 'MB', 'GB')
    $value = [double] $Bytes
    foreach ($unit in $units) {
        $value = $value / 1024
        if ($value -lt 1024) { return ('{0:N1} {1}' -f $value, $unit) }
    }
    return ('{0:N1} TB' -f ($value / 1024))
}

# --- what Windows thinks is installed ---------------------------------------
#
# Both registry views and both hives, because the installer offers per-user and
# per-machine and a 64-bit install lands somewhere a 32-bit view cannot see.
function Get-Installations {
    $roots = @(
        'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall'
        'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall'
        'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall'
    )
    $found = @()
    foreach ($root in $roots) {
        if (-not (Test-Path -LiteralPath $root)) { continue }
        $keys = Get-ChildItem -LiteralPath $root -ErrorAction SilentlyContinue
        foreach ($key in $keys) {
            $props = Get-ItemProperty -LiteralPath $key.PSPath -ErrorAction SilentlyContinue
            if ($null -eq $props) { continue }
            $name = if ($props.PSObject.Properties['DisplayName']) { $props.DisplayName } else { $null }
            if ([string]::IsNullOrWhiteSpace($name)) { continue }
            if ($name -notlike "*$ProductName*") { continue }
            $found += [pscustomobject] @{
                Name      = $name
                Version   = if ($props.PSObject.Properties['DisplayVersion']) { $props.DisplayVersion } else { 'unknown' }
                Scope     = if ($root.StartsWith('HKCU')) { 'this user' } else { 'all users' }
                Uninstall = if ($props.PSObject.Properties['QuietUninstallString']) {
                    $props.QuietUninstallString
                } elseif ($props.PSObject.Properties['UninstallString']) {
                    $props.UninstallString
                } else { $null }
            }
        }
    }
    return $found
}

# --- report -----------------------------------------------------------------

Write-Head "$ProductName uninstall"

# `$IsWindows` exists only in PowerShell 6+. Under Windows PowerShell 5.1 it is
# undefined, and `Set-StrictMode` makes reading it a terminating error -- so the
# variable is looked up rather than referenced. 5.1 only ships on Windows, which
# is what its absence means.
$onWindows = $true
$isWindowsVar = Get-Variable -Name IsWindows -ErrorAction SilentlyContinue
if ($null -ne $isWindowsVar) { $onWindows = [bool] $isWindowsVar.Value }

if (-not $onWindows) {
    Write-Host 'This script is for Windows. On this platform, remove:' -ForegroundColor Yellow
    Write-Step 'macOS   : /Applications/OpenConvert.app and ~/Library/Application Support/OpenConvert'
    Write-Step 'Linux   : the package, and ${XDG_STATE_HOME:-~/.local/state}/openconvert'
    exit 0
}

$installs = @(Get-Installations)
if ($installs.Count -eq 0) {
    Write-Step "No installed copy is registered with Windows."
} else {
    foreach ($i in $installs) {
        Write-Step "Installed: $($i.Name) $($i.Version) - for $($i.Scope)"
    }
}

$dataExists = Test-Path -LiteralPath $DataDir
if ($dataExists) {
    $size = Get-DirectorySize -Path $DataDir
    Write-Step "Data     : $DataDir ($(Format-Size $size))"
    foreach ($entry in $DataContents) {
        $path = Join-Path $DataDir $entry.Path
        if (Test-Path -LiteralPath $path) {
            Write-Step "           - $($entry.Path) : $($entry.What)"
        }
    }
} else {
    Write-Step "Data     : nothing at $DataDir"
}

Write-Step "Outputs  : not touched. Converted files are yours and stay where you saved them."

# --- act --------------------------------------------------------------------

$didSomething = $false

if (-not $KeepProgram) {
    foreach ($i in $installs) {
        if ([string]::IsNullOrWhiteSpace($i.Uninstall)) {
            Write-Warning "$($i.Name) has no uninstall command registered; remove it from Settings > Apps."
            continue
        }
        if ($PSCmdlet.ShouldProcess($i.Name, 'Run the registered uninstaller')) {
            Write-Head "Running the uninstaller for $($i.Name)"
            # Handed to the shell exactly as Windows recorded it. The string is
            # the OS's own, and quoting it ourselves is how a path with a space
            # in it becomes two arguments.
            Start-Process -FilePath 'cmd.exe' -ArgumentList '/c', $i.Uninstall -Wait
            $didSomething = $true
        }
    }
}

if ($RemoveData) {
    if (-not $dataExists) {
        Write-Step 'No data directory to remove.'
    } elseif ($PSCmdlet.ShouldProcess($DataDir, 'Delete permanently')) {
        Write-Head 'Removing user data'
        # A FIXED, FULLY-QUALIFIED PATH AND NOTHING DERIVED FROM INPUT. Same
        # rule the NSIS hook and the app's own temp sweep follow: an uninstaller
        # that composes a delete path from a variable is one bad variable away
        # from being a deletion primitive.
        Remove-Item -LiteralPath $DataDir -Recurse -Force
        Write-Step "Deleted $DataDir"
        $didSomething = $true
    }
} elseif ($dataExists) {
    Write-Host ''
    Write-Host 'Settings, history and receipts were KEPT.' -ForegroundColor Yellow
    Write-Step 'Re-run with -RemoveData to delete them as well.'
}

if (-not $didSomething -and -not $WhatIfPreference) {
    Write-Host ''
    Write-Step 'Nothing to do.'
}
