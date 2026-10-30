# Run with Windows PowerShell. Uses fake applications and isolated temp folders.
$ErrorActionPreference = 'Stop'
$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('hebnix-maintenance-tests-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $testRoot | Out-Null
$source = Get-Content -LiteralPath (Join-Path $PSScriptRoot '../crates/hebnix-app/src/maintenance.ps1') -Raw
$psExe = Join-Path $env:SystemRoot 'System32/WindowsPowerShell/v1.0/powershell.exe'
function Assert($condition, [string]$message) { if (!$condition) { throw $message } }
function Run-Helper($plan, [string]$name) {
    $helper = Join-Path $testRoot $name
    New-Item -ItemType Directory -Path $helper | Out-Null
    $script = Join-Path $helper 'maintenance.ps1'
    # Suppress Explorer windows for deliberate failure cases only. All actual
    # file operations and application launches use the production implementation.
    $mock = @'
function Start-Process {
    [CmdletBinding()]
    param([string]$FilePath, [string[]]$ArgumentList, [string]$WorkingDirectory, [string]$WindowStyle, [switch]$PassThru)
    if ($FilePath -eq 'explorer.exe') { return }
    Microsoft.PowerShell.Management\Start-Process @PSBoundParameters
}
'@
    $scriptText = $source.Insert($source.IndexOf("`n") + 1, $mock + "`n")
    Set-Content -LiteralPath $script -Value $scriptText -Encoding UTF8
    $planPath = Join-Path $helper 'plan.json'
    $plan | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $planPath -Encoding UTF8
    & $psExe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $script -PlanPath $planPath
    return $LASTEXITCODE
}
try {
    $app = Join-Path $testRoot "Portable app's folder"
    New-Item -ItemType Directory -Path $app | Out-Null
    $old = Join-Path $app 'Hebnix.exe'
    $new = Join-Path $app '.hebnix-update-test.exe'
    $backup = Join-Path $app 'Hebnix.old-test.exe'
    $marker = Join-Path $app 'portable.txt'
    Set-Content -LiteralPath $marker -Value 'portable=true'
    # Both stubs are real PE files. The old version can be relaunched after a
    # rollback; the new one confirms reaching its startup handshake.
    Add-Type -TypeDefinition @'
public static class OldApp {
    public static void Main() { System.Threading.Thread.Sleep(1500); }
}
'@ -OutputAssembly $old -OutputType WindowsApplication
    Add-Type -TypeDefinition @'
public static class NewApp {
    public static void Main() {
        var path = System.Environment.GetEnvironmentVariable("HEBNIX_UPDATE_ACK");
        if (!System.String.IsNullOrEmpty(path)) System.IO.File.WriteAllText(path, "started");
    }
}
'@ -OutputAssembly $new -OutputType WindowsApplication
    $oldHash = (Get-FileHash -LiteralPath $old).Hash
    $newHash = (Get-FileHash -LiteralPath $new).Hash
    $parent = Start-Process -FilePath $old -PassThru -WindowStyle Hidden
    $waitStart = [DateTime]::UtcNow
    $plan = @{mode='update'; pid=$parent.Id; exe=$old; staged=$new; backup=$backup; base=$app}
    $code = Run-Helper $plan 'update-success'
    Assert ($code -eq 0) "Update failed ($code)"
    Assert (([DateTime]::UtcNow - $waitStart).TotalSeconds -ge 1) 'Helper did not wait for the old process'
    Assert ((Get-FileHash -LiteralPath $old).Hash -eq $newHash) 'Update did not replace the executable'
    Assert (!(Test-Path -LiteralPath $backup)) 'Old executable was not cleaned up'
    Assert (Test-Path -LiteralPath $marker) 'Portable marker was removed'
    Assert (!(Test-Path -LiteralPath (Join-Path $testRoot 'update-success'))) 'Successful helper left files behind'

    # An application that exits before its first frame must roll back.
    $bad = Join-Path $app '.hebnix-update-bad.exe'
    Add-Type -TypeDefinition @'
public static class BrokenApp {
    public static void Main() { System.Environment.Exit(1); }
}
'@ -OutputAssembly $bad -OutputType WindowsApplication
    $plan.staged = $bad
    $plan.pid = 2147483647
    $code = Run-Helper $plan 'update-rollback'
    Assert ($code -eq 1) 'Startup failure did not report an error'
    Assert ((Get-FileHash -LiteralPath $old).Hash -eq $newHash) 'Startup failure lost the working version'

    $data = Join-Path $app 'plugins'
    New-Item -ItemType Directory -Path $data | Out-Null
    Set-Content -LiteralPath (Join-Path $data 'plugin.lua') -Value 'test'
    $unrelated = Join-Path $app 'my unrelated file.txt'
    Set-Content -LiteralPath $unrelated -Value 'keep'
    $plan = @{mode='uninstall'; pid=2147483647; exe=$old; paths=@($data,$old); remove_app=$false; base=$app}
    $code = Run-Helper $plan 'uninstall-success'
    Assert ($code -eq 0) 'Uninstall helper failed'
    Assert (!(Test-Path -LiteralPath $data)) 'Selected plugins were not removed'
    Assert (!(Test-Path -LiteralPath $old)) 'Executable was not removed'
    Assert (Test-Path -LiteralPath $unrelated) 'Uninstall removed unrelated files'
    Assert (Test-Path -LiteralPath $marker) 'Unselected portable marker was removed'

    # A selected junction must not let deletion escape into unrelated data.
    $outside = Join-Path $testRoot 'unrelated-data'
    New-Item -ItemType Directory -Path $outside | Out-Null
    $keep = Join-Path $outside 'keep.txt'
    Set-Content -LiteralPath $keep -Value 'keep'
    $junction = Join-Path $app 'plugins'
    New-Item -ItemType Junction -Path $junction -Target $outside | Out-Null
    $plan.paths = @($junction)
    $code = Run-Helper $plan 'uninstall-link-refused'
    Assert ($code -eq 1) 'Unsafe directory link was not reported'
    Assert (Test-Path -LiteralPath $keep) 'Uninstall traversed a directory junction'
    Assert (Test-Path -LiteralPath (Join-Path $testRoot 'uninstall-link-refused/Cleanup needed.txt')) 'Manual cleanup report missing'
    [IO.Directory]::Delete($junction)
    Write-Output 'PASS: parent wait, replacement, startup rollback, helper cleanup, selective uninstall, portable/unrelated-file preservation, junction protection and cleanup report'
} finally {
    # The generated absolute path is constrained to this run's dedicated folder.
    $resolved = [IO.Path]::GetFullPath($testRoot)
    $temp = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
    if (!$resolved.StartsWith($temp, [StringComparison]::OrdinalIgnoreCase) -or !(Split-Path $resolved -Leaf).StartsWith('hebnix-maintenance-tests-')) {
        throw 'Refusing to clean up outside the test directory'
    }
    if ($junction -and (Test-Path -LiteralPath $junction)) { [IO.Directory]::Delete($junction) }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
