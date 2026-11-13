# Hebnix one-line installer
#
#   irm https://raw.githubusercontent.com/Hebbins/Hebnix-Setup/main/install.ps1 | iex
#   irm https://raw.githubusercontent.com/Hebbins/Hebnix-Setup/main/install-lite.ps1 | iex
#   irm https://raw.githubusercontent.com/Hebbins/Hebnix-Setup/main/install-portable.ps1 | iex
#
# Downloads the latest Hebnix (or Hebnix Lite), extracts it, and adds Desktop and Start Menu
# shortcuts. Hebnix updates and uninstalls itself from inside the app, so no separate
# setup program is installed.
#
# -Portable instead extracts into the current folder (or -Destination) and adds
# portable.txt, which makes Hebnix keep its
# data next to the exe (hebnix-sdk paths::base_dir). It creates no shortcuts, writes no .inst
# and installs nothing else, so nothing outside that folder is touched.
#
# This runs inside the caller's session through iex, so it never calls `exit` and keeps
# everything inside Install-Hebnix to avoid leaking variables.

param(
    [ValidateSet('Hebnix', 'Lite')]
    [string]$Edition = 'Hebnix',
    [switch]$Portable,
    [string]$Destination
)

function Install-Hebnix {
    param([string]$Edition, [switch]$Portable, [string]$Destination)

    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue'
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    Add-Type -AssemblyName System.Net.Http
    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem

    $apiBaseUrl = 'https://api.hebnix.com'
    $browserUserAgent = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36'
    # HEBNIX_INSTALL_DIR is only for testing; normal installs always use %APPDATA%\Hebnix
    $installDirectory = if ($env:HEBNIX_INSTALL_DIR) { $env:HEBNIX_INSTALL_DIR } else { Join-Path $env:APPDATA 'Hebnix' }
    if ($Portable) {
        if ($Destination) {
            $installDirectory = [System.IO.Path]::GetFullPath($Destination)
        } else {
            # iex runs in the caller's session, so this is wherever they cd'd to
            $location = Get-Location
            if ($location.Provider.Name -ne 'FileSystem') {
                throw "cd into the folder you want Hebnix in first (the current location $location is not a folder)."
            }
            $installDirectory = $location.ProviderPath
        }
        # an admin PowerShell starts in System32 and a normal one in the home folder. Hebnix
        # fills its folder with data folders, so make people pick one instead of spilling there
        $trimmed = $installDirectory.TrimEnd('\')
        $refused = @($env:USERPROFILE, [Environment]::GetFolderPath('DesktopDirectory'), [System.IO.Path]::GetPathRoot($installDirectory)) |
            Where-Object { $_ -and $trimmed -ieq $_.TrimEnd('\') }
        foreach ($systemFolder in @($env:windir, $env:ProgramFiles, ${env:ProgramFiles(x86)})) {
            if ($systemFolder -and ($trimmed + '\').StartsWith($systemFolder.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) { $refused = $true }
        }
        if ($refused) {
            throw "Not installing into $installDirectory. Make a folder for Hebnix and cd into it first, e.g.  mkdir ~\Hebnix; cd ~\Hebnix"
        }
    }
    $installStatePath = Join-Path $installDirectory '.inst'
    $liteExecutableNames = @('Hebnix Lite.exe', 'Hebnite Lite.exe')

    if ($Edition -eq 'Lite') {
        $editionName = 'Hebnix Lite'
        $endpoint = '/download-hebnix-lite'
    } else {
        $editionName = 'Hebnix'
        $endpoint = '/download-hebnix'
    }

    # no bar when output is piped or logged, the redraws would just be noise
    $showProgress = -not [Console]::IsOutputRedirected

    function Format-Size([double]$Bytes) {
        if ($Bytes -ge 1GB) { return '{0:N1} GB' -f ($Bytes / 1GB) }
        if ($Bytes -ge 1MB) { return '{0:N1} MB' -f ($Bytes / 1MB) }
        if ($Bytes -ge 1KB) { return '{0:N0} KB' -f ($Bytes / 1KB) }
        return "$Bytes B"
    }

    # one line redrawn in place, like winget: "  ===============  12.3 MB / 45.6 MB  3.2 MB/s"
    # the bar character is built from its code point so the script stays plain ASCII
    function Write-ProgressBar([double]$Fraction, [string]$Text) {
        $width = 30
        $filled = [Math]::Min($width, [int][Math]::Floor($Fraction * $width))
        $bar = [string][char]0x2501
        Write-Host "`r  " -NoNewline
        Write-Host ($bar * $filled) -NoNewline -ForegroundColor Cyan
        Write-Host ($bar * ($width - $filled)) -NoNewline -ForegroundColor DarkGray
        Write-Host "  $Text    " -NoNewline
    }

    function Write-DownloadProgress([long]$Done, $Total, [double]$Seconds) {
        # the first chunk lands almost instantly, so an early speed reading is nonsense
        $speed = if ($Seconds -ge 0.5) { "$(Format-Size ($Done / $Seconds))/s" } else { '' }
        if ($Total) {
            Write-ProgressBar ($Done / $Total) "$(Format-Size $Done) / $(Format-Size $Total)  $speed"
        } else {
            # no Content-Length, so there is nothing to fill a bar against
            Write-Host "`r  $(Format-Size $Done)  $speed    " -NoNewline
        }
    }

    $stallTimeoutSeconds = 30

    function Invoke-Download([string]$Url, [string]$Destination) {
        # HttpClient rather than Invoke-WebRequest: PowerShell 5.1 refuses to set Referer
        $client = New-Object System.Net.Http.HttpClient
        $client.Timeout = [TimeSpan]::FromMinutes(10)
        try {
            $request = New-Object System.Net.Http.HttpRequestMessage([System.Net.Http.HttpMethod]::Get, $Url)
            $request.Headers.UserAgent.ParseAdd($browserUserAgent)
            $request.Headers.Referrer = [Uri]"$apiBaseUrl/"
            $response = $client.SendAsync($request, [System.Net.Http.HttpCompletionOption]::ResponseHeadersRead).GetAwaiter().GetResult()
            try {
                $response.EnsureSuccessStatusCode() | Out-Null
                $expectedLength = $response.Content.Headers.ContentLength
                $source = $response.Content.ReadAsStreamAsync().GetAwaiter().GetResult()
                $target = [System.IO.File]::Create($Destination)
                $buffer = New-Object byte[] 81920
                $downloaded = 0L
                $timer = [Diagnostics.Stopwatch]::StartNew()
                $lastDraw = -1000L
                $drew = $false
                try {
                    while ($true) {
                        # HttpClient.Timeout stops at the headers, so a server that stops
                        # sending without closing the connection would hang a plain Read forever.
                        # Give up on the connection instead, the retry gets a fresh one
                        $readTask = $source.ReadAsync($buffer, 0, $buffer.Length)
                        if (-not $readTask.Wait([TimeSpan]::FromSeconds($stallTimeoutSeconds))) {
                            throw "download stalled (no data for $stallTimeoutSeconds seconds)"
                        }
                        $read = $readTask.GetAwaiter().GetResult()
                        if ($read -le 0) { break }
                        $target.Write($buffer, 0, $read)
                        $downloaded += $read
                        # redraw at most 10 times a second, console writes are slow
                        if ($showProgress -and $timer.ElapsedMilliseconds - $lastDraw -ge 100) {
                            $lastDraw = $timer.ElapsedMilliseconds
                            Write-DownloadProgress $downloaded $expectedLength $timer.Elapsed.TotalSeconds
                            $drew = $true
                        }
                    }
                    if ($showProgress) {
                        Write-DownloadProgress $downloaded $expectedLength $timer.Elapsed.TotalSeconds
                        $drew = $true
                    }
                } finally {
                    $target.Dispose(); $source.Dispose()
                    # end the bar's line, also when the download fails and a retry message follows
                    if ($drew) { Write-Host '' }
                }
                if (-not $showProgress) {
                    Write-Host "  Downloaded $(Format-Size $downloaded) in $([Math]::Round($timer.Elapsed.TotalSeconds, 1))s"
                }
                # the server sometimes drops the connection mid-file without an error
                $actualLength = (Get-Item $Destination).Length
                if ($expectedLength -and $actualLength -ne $expectedLength) {
                    throw "download was cut off ($actualLength of $expectedLength bytes)"
                }
            } finally {
                $response.Dispose()
            }
        } finally {
            $client.Dispose()
        }
    }

    function Invoke-DownloadWithRetry([string]$Url, [string]$Destination) {
        $attempts = 3
        for ($attempt = 1; $attempt -le $attempts; $attempt++) {
            try {
                Invoke-Download $Url $Destination
                return
            } catch {
                if ($attempt -eq $attempts) { throw }
                Write-Host "  Download failed ($($_.Exception.Message)), retrying..." -ForegroundColor Yellow
                Start-Sleep -Seconds (3 * $attempt)
            }
        }
    }

    # mirrors MainForm.ExtractAsync, including the check that no entry escapes the folder
    function Expand-ZipSafely([string]$ZipPath, [string]$Destination) {
        New-Item -ItemType Directory -Force -Path $Destination | Out-Null
        $root = [System.IO.Path]::GetFullPath($Destination.TrimEnd('\') + '\')
        $archive = [System.IO.Compression.ZipFile]::OpenRead($ZipPath)
        $total = $archive.Entries.Count
        $done = 0
        $timer = [Diagnostics.Stopwatch]::StartNew()
        $lastDraw = -1000L
        try {
            foreach ($entry in $archive.Entries) {
                $done++
                if ($showProgress -and ($timer.ElapsedMilliseconds - $lastDraw -ge 100 -or $done -eq $total)) {
                    $lastDraw = $timer.ElapsedMilliseconds
                    Write-ProgressBar ($done / $total) "$done / $total files"
                }
                $target = [System.IO.Path]::GetFullPath((Join-Path $Destination $entry.FullName))
                if (-not $target.StartsWith($root, [StringComparison]::OrdinalIgnoreCase)) {
                    throw "The download contains an unsafe path: $($entry.FullName)"
                }
                if ([string]::IsNullOrEmpty($entry.Name)) {
                    New-Item -ItemType Directory -Force -Path $target | Out-Null
                } else {
                    New-Item -ItemType Directory -Force -Path (Split-Path $target) | Out-Null
                    [System.IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $target, $true)
                }
            }
        } finally {
            $archive.Dispose()
            if ($showProgress -and $total) { Write-Host '' }
        }
    }

    function New-Shortcut([string]$ShortcutPath, [string]$TargetPath, [string]$WorkingDirectory) {
        New-Item -ItemType Directory -Force -Path (Split-Path $ShortcutPath) | Out-Null
        $shell = New-Object -ComObject WScript.Shell
        $shortcut = $shell.CreateShortcut($ShortcutPath)
        $shortcut.TargetPath = $TargetPath
        $shortcut.WorkingDirectory = $WorkingDirectory
        $shortcut.Save()
    }

    # same Edition=version format and ordering as MainForm.SaveInstalledVersions
    function Set-InstalledVersion([string]$Edition, [string]$Version) {
        $versions = @{}
        if (Test-Path $installStatePath) {
            foreach ($line in Get-Content $installStatePath) {
                $separator = $line.IndexOf('=')
                if ($separator -le 0 -or $separator -eq $line.Length - 1) { continue }
                $versions[$line.Substring(0, $separator).Trim()] = $line.Substring($separator + 1).Trim()
            }
        }
        $versions[$Edition] = $Version
        $lines = foreach ($name in @('Hebnix', 'Hebnix Lite')) {
            if ($versions[$name]) { "$name=$($versions[$name])" }
        }
        [System.IO.File]::WriteAllLines($installStatePath, [string[]]@($lines))
    }

    if ($Portable) {
        Write-Host "Installing $editionName (portable) to $installDirectory" -ForegroundColor Cyan
    } else {
        Write-Host "Installing $editionName" -ForegroundColor Cyan
    }

    Write-Host '  Checking latest version...'
    $info = Invoke-RestMethod -Uri "$apiBaseUrl/info" -UserAgent 'Hebnix-Updater' -TimeoutSec 15 -UseBasicParsing
    $latestVersion = "$($info.latest_version)".Trim()
    if (-not $latestVersion) { throw 'The update API did not return a version.' }
    Write-Host "  Latest version: $latestVersion"

    New-Item -ItemType Directory -Force -Path $installDirectory | Out-Null

    $folderPrefix = $installDirectory.TrimEnd('\') + '\'
    foreach ($processName in @('Hebnix', 'Hebnix Lite', 'Hebnite Lite')) {
        Get-Process -Name $processName -ErrorAction SilentlyContinue | Where-Object {
            # a portable copy only replaces its own exe, so leave other copies running.
            # Path is empty for processes we cannot inspect; skip those in portable mode
            -not $Portable -or ($_.Path -and $_.Path.StartsWith($folderPrefix, [StringComparison]::OrdinalIgnoreCase))
        } | ForEach-Object {
            Write-Host "  Closing $($_.ProcessName)..."
            $_.Kill()
            $_.WaitForExit(3000) | Out-Null
        }
    }

    $temporaryZip = Join-Path ([System.IO.Path]::GetTempPath()) ("Hebnix-" + [Guid]::NewGuid().ToString('N') + '.zip')
    try {
        Write-Host "  Downloading $editionName $latestVersion..."
        Invoke-DownloadWithRetry "$apiBaseUrl$endpoint" $temporaryZip
        Write-Host '  Extracting...'
        Expand-ZipSafely $temporaryZip $installDirectory
    } finally {
        Remove-Item -Force -ErrorAction SilentlyContinue $temporaryZip
    }

    if ($Edition -eq 'Lite') {
        $executableName = $liteExecutableNames | Where-Object { Test-Path (Join-Path $installDirectory $_) } | Select-Object -First 1
    } else {
        $executableName = if (Test-Path (Join-Path $installDirectory 'Hebnix.exe')) { 'Hebnix.exe' }
    }
    if (-not $executableName) { throw "The download did not contain the $editionName executable." }
    $executablePath = Join-Path $installDirectory $executableName

    if ($Portable) {
        # its presence is all Hebnix checks; the text is for whoever opens the folder
        $marker = Join-Path $installDirectory 'portable.txt'
        if (-not (Test-Path $marker)) {
            [System.IO.File]::WriteAllText($marker, "Hebnix keeps its settings and data in this folder while this file is here.`r`n")
        }
        Write-Host ''
        Write-Host "$editionName $latestVersion (portable) installed to $installDirectory" -ForegroundColor Green
        Write-Host "  Run $executableName from that folder. Settings and data stay in it."
        Write-Host '  To update, run this command again in that folder. To remove it, delete the folder.'
        return
    }

    if ($env:HEBNIX_INSTALL_DIR) {
        # test installs keep their shortcuts out of the real Desktop and Start Menu
        $desktop = Join-Path $installDirectory '_test-shortcuts\Desktop'
        $startMenu = Join-Path $installDirectory '_test-shortcuts\Programs\Hebnix'
    } else {
        $desktop = [Environment]::GetFolderPath('DesktopDirectory')
        $startMenu = Join-Path ([Environment]::GetFolderPath('Programs')) 'Hebnix'
    }
    New-Shortcut (Join-Path $desktop "$editionName.lnk") $executablePath $installDirectory
    New-Shortcut (Join-Path $startMenu "$editionName.lnk") $executablePath $installDirectory

    Set-InstalledVersion $editionName $latestVersion

    Write-Host ''
    Write-Host "$editionName $latestVersion installed to $installDirectory" -ForegroundColor Green
    Write-Host "  Launch it from the Desktop or Start Menu shortcut '$editionName'."
    Write-Host '  Hebnix updates and uninstalls itself from its settings. To update from here, close Hebnix and run this command again.'
}

Install-Hebnix -Edition $Edition -Portable:$Portable -Destination $Destination
