# PDFJiff installer for Windows (PowerShell 5.1+ or PowerShell 7+).
#
#   irm https://github.com/pdfjiff/pdfjiff/releases/latest/download/install.ps1 | iex
#
# Settings (environment variables, because `irm | iex` cannot take parameters):
#   $env:PDFJIFF_VERSION      = '0.1.0'      # default: latest
#   $env:PDFJIFF_INSTALL_DIR  = 'C:\tools'   # default: %LOCALAPPDATA%\Programs\pdfjiff\bin
#   $env:PDFJIFF_NO_MODIFY_PATH = '1'        # do not add the directory to your user PATH
#   $env:PDFJIFF_UNINSTALL    = '1'          # remove pdfjiff and its PATH entry
#   $env:PDFJIFF_DOWNLOAD_BASE = 'https://mirror.example/pdfjiff'  # archives + SHA256SUMS
#
# The script downloads one zip for your CPU, verifies its SHA-256 checksum and
# copies pdfjiff.exe into place. It never needs administrator rights.

& {
    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue' # Invoke-WebRequest is very slow with a progress bar
    Set-StrictMode -Version 2.0

    function Write-Step([string] $Message) { Write-Host "pdfjiff-install: $Message" }

    $repo = if ($env:PDFJIFF_REPO) { $env:PDFJIFF_REPO } else { 'pdfjiff/pdfjiff' }
    $version = if ($env:PDFJIFF_VERSION) { $env:PDFJIFF_VERSION } else { 'latest' }
    $installDir = if ($env:PDFJIFF_INSTALL_DIR) {
        $env:PDFJIFF_INSTALL_DIR
    } else {
        Join-Path $env:LOCALAPPDATA 'Programs\pdfjiff\bin'
    }
    $exe = Join-Path $installDir 'pdfjiff.exe'

    function Get-UserPath {
        $value = [Environment]::GetEnvironmentVariable('Path', 'User')
        if ($value) { @($value -split ';' | Where-Object { $_ }) } else { @() }
    }

    if ($env:PDFJIFF_UNINSTALL -eq '1') {
        if (Test-Path -LiteralPath $exe) {
            Remove-Item -LiteralPath $exe -Force
            Write-Step "removed $exe"
        } else {
            Write-Step "nothing to remove at $exe"
        }
        $entries = Get-UserPath
        if ($entries -contains $installDir) {
            $kept = $entries | Where-Object { $_ -ne $installDir }
            [Environment]::SetEnvironmentVariable('Path', ($kept -join ';'), 'User')
            Write-Step "removed $installDir from your user PATH"
        }
        return
    }

    # --- Detect the release target ------------------------------------------------
    $cpu = $null
    try {
        $cpu = [string][System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
    } catch {
        $cpu = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
    }
    $arch = switch -Regex ($cpu) {
        '^(X64|AMD64)$' { 'x86_64'; break }
        '^(Arm64|ARM64)$' { 'aarch64'; break }
        default { throw "pdfjiff-install: unsupported CPU architecture '$cpu'. Build from source: cargo install pdfjiff-cli --locked" }
    }
    $target = "$arch-pc-windows-msvc"
    $archive = "pdfjiff-$target.zip"

    $base = if ($env:PDFJIFF_DOWNLOAD_BASE) {
        $env:PDFJIFF_DOWNLOAD_BASE.TrimEnd('/')
    } elseif ($version -eq 'latest') {
        "https://github.com/$repo/releases/latest/download"
    } elseif ($version.StartsWith('v')) {
        "https://github.com/$repo/releases/download/$version"
    } else {
        "https://github.com/$repo/releases/download/v$version"
    }

    # Windows PowerShell 5.1 may default to TLS 1.0; GitHub requires TLS 1.2+.
    try {
        [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    } catch {
        Write-Verbose 'Could not change TLS settings; continuing with the defaults.'
    }

    $tmp = Join-Path ([IO.Path]::GetTempPath()) ("pdfjiff-" + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $tmp | Out-Null
    try {
        # --- Download and verify ------------------------------------------------------
        Write-Step "downloading $archive ($version)"
        $zip = Join-Path $tmp $archive
        $sums = Join-Path $tmp 'SHA256SUMS'
        try {
            Invoke-WebRequest -UseBasicParsing -Uri "$base/$archive" -OutFile $zip
            Invoke-WebRequest -UseBasicParsing -Uri "$base/SHA256SUMS" -OutFile $sums
        } catch {
            throw "pdfjiff-install: download failed from $base ($($_.Exception.Message)). Check the version exists: https://github.com/$repo/releases"
        }

        $expected = $null
        foreach ($line in Get-Content -LiteralPath $sums) {
            $parts = $line.Trim() -split '\s+', 2
            if ($parts.Count -eq 2 -and $parts[1].TrimStart('*') -eq $archive) { $expected = $parts[0].ToLowerInvariant() }
        }
        if (-not $expected) { throw "pdfjiff-install: SHA256SUMS has no entry for $archive" }
        $actual = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actual -ne $expected) {
            throw "pdfjiff-install: checksum mismatch for $archive (expected $expected, got $actual). Nothing was installed."
        }
        Write-Step 'checksum verified'

        Expand-Archive -LiteralPath $zip -DestinationPath $tmp -Force
        $source = Join-Path $tmp "pdfjiff-$target\pdfjiff.exe"
        if (-not (Test-Path -LiteralPath $source)) { throw "pdfjiff-install: archive does not contain pdfjiff-$target\pdfjiff.exe" }

        # --- Install ------------------------------------------------------------------
        New-Item -ItemType Directory -Path $installDir -Force | Out-Null
        try {
            Copy-Item -LiteralPath $source -Destination $exe -Force
        } catch {
            throw "pdfjiff-install: could not write $exe. Close any running pdfjiff and try again. ($($_.Exception.Message))"
        }
        $installed = & $exe --version
        if ($LASTEXITCODE -ne 0) { throw "pdfjiff-install: installed $exe, but it does not run on this system" }
        Write-Step "installed $installed to $exe"
    } finally {
        Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
    }

    if ($env:PDFJIFF_NO_MODIFY_PATH -ne '1') {
        $entries = Get-UserPath
        if ($entries -notcontains $installDir) {
            [Environment]::SetEnvironmentVariable('Path', (@($entries) + $installDir) -join ';', 'User')
            Write-Step "added $installDir to your user PATH (open a new terminal to use it)"
        }
        if (($env:Path -split ';') -notcontains $installDir) { $env:Path = "$env:Path;$installDir" }
    }
    Write-Step 'next: pdfjiff --help    (tab completion: pdfjiff completions powershell >> $PROFILE)'
}
