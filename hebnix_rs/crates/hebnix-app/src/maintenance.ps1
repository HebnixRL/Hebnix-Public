param([Parameter(Mandatory=$true)][string]$PlanPath)
$ErrorActionPreference = 'Stop'
$plan = Get-Content -LiteralPath $PlanPath -Raw -Encoding UTF8 | ConvertFrom-Json
$helperDir = Split-Path -LiteralPath $PlanPath
function Retry([scriptblock]$operation) {
    for ($attempt = 0; $attempt -lt 100; $attempt++) {
        try { & $operation; return } catch {
            if ($attempt -eq 99) { throw }
            Start-Sleep -Milliseconds 200
        }
    }
}
try {
    # Wait for this process object, so a reused PID cannot block maintenance.
    $parent = Get-Process -Id $plan.pid -ErrorAction SilentlyContinue
    if ($null -ne $parent) { $parent.WaitForExit() }
    if ($plan.mode -eq 'update') {
        $movedOld = $false
        $installed = $false
        try {
            Retry { Move-Item -LiteralPath $plan.exe -Destination $plan.backup }
            $movedOld = $true
            Retry { Move-Item -LiteralPath $plan.staged -Destination $plan.exe }
            $installed = $true
            if ($plan.base) { [IO.File]::WriteAllText((Join-Path $plan.base '.first'), 'updated') }
            $env:HEBNIX_UPDATE_ACK = Join-Path $helperDir 'started'
            $updated = Start-Process -FilePath $plan.exe -WorkingDirectory ([IO.Path]::GetDirectoryName($plan.exe)) -WindowStyle Hidden -PassThru -ErrorAction Stop
            $deadline = [DateTime]::UtcNow.AddMinutes(3)
            while (!(Test-Path -LiteralPath $env:HEBNIX_UPDATE_ACK)) {
                if ($updated.HasExited) { throw 'The updated application exited before opening its window.' }
                if ([DateTime]::UtcNow -gt $deadline) { throw 'The updated application did not finish starting.' }
                Start-Sleep -Milliseconds 200
            }
            Remove-Item -LiteralPath $env:HEBNIX_UPDATE_ACK -Force
            Remove-Item Env:\HEBNIX_UPDATE_ACK
        } catch {
            Remove-Item Env:\HEBNIX_UPDATE_ACK -ErrorAction SilentlyContinue
            if ($null -ne $updated -and !$updated.HasExited) { $updated.Kill(); $updated.WaitForExit() }
            if ($installed) { Retry { Remove-Item -LiteralPath $plan.exe -Force } }
            if ($movedOld) { Retry { Move-Item -LiteralPath $plan.backup -Destination $plan.exe } }
            Start-Process -FilePath $plan.exe -WorkingDirectory ([IO.Path]::GetDirectoryName($plan.exe)) -WindowStyle Hidden
            throw
        }
        Retry { Remove-Item -LiteralPath $plan.backup -Force }
    } elseif ($plan.mode -eq 'uninstall') {
        $remaining = [System.Collections.Generic.List[string]]::new()
        foreach ($path in $plan.paths) {
            try {
                if (Test-Path -LiteralPath $path) {
                    # Plans contain individual, absolute Hebnix-owned entries.
                    # Never follow a junction when deleting an owned directory.
                    $item = Get-Item -LiteralPath $path -Force
                    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                        throw "Refusing to traverse link: $path"
                    }
                    if ($item.PSIsContainer) {
                        $links = @(Get-ChildItem -LiteralPath $path -Recurse -Force | Where-Object { ($_.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 })
                        if ($links.Count -gt 0) { throw "Directory contains links: $path" }
                    }
                    Retry { Remove-Item -LiteralPath $path -Recurse -Force }
                }
            } catch { $remaining.Add($path) }
        }
        if ($plan.remove_app) {
            if ($plan.shortcut -and (Test-Path -LiteralPath $plan.shortcut)) {
                $shortcut = (New-Object -ComObject WScript.Shell).CreateShortcut($plan.shortcut)
                if ($shortcut.TargetPath -eq $plan.exe) {
                    try { Remove-Item -LiteralPath $plan.shortcut -Force } catch { $remaining.Add($plan.shortcut) }
                }
            }
            $key = 'HKCU:\Software\Classes\hebnix'
            $command = (Get-ItemProperty -LiteralPath "$key\shell\open\command" -ErrorAction SilentlyContinue).'(default)'
            if ($command -and $command.StartsWith('"' + $plan.exe + '"', [StringComparison]::OrdinalIgnoreCase)) {
                try { Remove-Item -LiteralPath $key -Recurse -Force } catch { $remaining.Add($key) }
            }
        }
        if ($remaining.Count -gt 0) {
            $report = Join-Path $helperDir 'Cleanup needed.txt'
            $remaining | Set-Content -LiteralPath $report
            Start-Process -FilePath explorer.exe -ArgumentList @('/select,', ('"' + $report + '"')) -WindowStyle Normal
            foreach ($folder in @($remaining | Where-Object { [IO.Path]::IsPathRooted($_) } | ForEach-Object { [IO.Path]::GetDirectoryName($_) } | Select-Object -Unique)) {
                if (Test-Path -LiteralPath $folder) { Start-Process -FilePath explorer.exe -ArgumentList ('"' + $folder + '"') -WindowStyle Normal }
            }
            exit 1
        }
    }
    Remove-Item -LiteralPath $PlanPath -Force
    Remove-Item -LiteralPath $PSCommandPath -Force
    # Only remove this dedicated helper folder when empty.
    if (@(Get-ChildItem -LiteralPath $helperDir -Force).Count -eq 0) { [IO.Directory]::Delete($helperDir) }
} catch {
    $_ | Out-String | Set-Content -LiteralPath (Join-Path $helperDir 'Maintenance failed.txt')
    Start-Process -FilePath explorer.exe -ArgumentList ('"' + $helperDir + '"') -WindowStyle Normal
    exit 1
}
