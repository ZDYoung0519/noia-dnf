# Build with: .\scripts\build-installer.ps1 [-MakeNsis C:\Tools\NSIS\makensis.exe]
[CmdletBinding()]
param([string]$MakeNsis)

$ErrorActionPreference = 'Stop'
$project = Split-Path -Parent $PSScriptRoot
if ($MakeNsis) {
    $compiler = (Get-Command $MakeNsis -CommandType Application -ErrorAction Stop).Source
} else {
    $compiler = (Get-Command makensis.exe -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1).Source
    if (-not $compiler) {
        $candidates = @(
            "${env:ProgramFiles(x86)}\NSIS\makensis.exe"
            "$env:ProgramFiles\NSIS\makensis.exe"
        )
        foreach ($key in 'HKLM:\SOFTWARE\WOW6432Node\NSIS', 'HKLM:\SOFTWARE\NSIS', 'HKCU:\SOFTWARE\NSIS') {
            if (Test-Path -LiteralPath $key) {
                $nsisDir = (Get-Item -LiteralPath $key).GetValue('')
                if ($nsisDir) { $candidates += Join-Path $nsisDir 'makensis.exe' }
            }
        }
        $candidates += @(Get-Item "$project\target\packaging-tools\nsis-*\makensis.exe" -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending | ForEach-Object FullName)
        $compiler = $candidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
    }
    if (-not $compiler) { throw 'Install NSIS 3 (winget install --id NSIS.NSIS -e), or pass -MakeNsis with the compiler path.' }
}

Push-Location -LiteralPath $project
try {
    $metadata = cargo metadata --no-deps --format-version 1 --locked | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed.' }
    $version = ($metadata.packages | Where-Object name -eq 'noia-dnf').version
    $releases = $env:NOIA_RELEASE_URL
    if ($null -eq $releases) {
        $line = Get-Content -Encoding UTF8 -LiteralPath '.env' | Where-Object { $_.StartsWith('NOIA_RELEASE_URL=') } | Select-Object -First 1
        $releases = $line.Substring('NOIA_RELEASE_URL='.Length).Trim()
    }
    $releases = $releases.TrimEnd('/')
    $notes = & (Join-Path $PSScriptRoot 'read-changelog.ps1') -Version $version
    $exe = $null
    cargo build --release --locked --bin noia-dnf --message-format=json-render-diagnostics | ForEach-Object {
        $artifact = $_ | ConvertFrom-Json
        if ($artifact.reason -eq 'compiler-artifact' -and $artifact.target.name -eq 'noia-dnf' -and $artifact.executable) {
            $exe = $artifact.executable
        }
    }
    if ($LASTEXITCODE -ne 0 -or -not $exe) { throw 'Release build failed.' }

    # Check the actual PE architecture before labeling the installer x64.
    $reader = [IO.BinaryReader]::new([IO.File]::OpenRead($exe))
    try {
        $reader.BaseStream.Position = 0x3c
        $reader.BaseStream.Position = $reader.ReadUInt32()
        if ($reader.ReadUInt32() -ne 0x4550 -or $reader.ReadUInt16() -ne 0x8664) {
            throw 'An x64 Windows release is required. Set CARGO_BUILD_TARGET=x86_64-pc-windows-msvc.'
        }
    } finally { $reader.Dispose() }

    $outDir = New-Item -ItemType Directory -Force -Path (Join-Path $metadata.target_directory 'installer')
    $output = Join-Path $outDir.FullName "Noia-DNF-$version-windows-x64-setup.exe"
    & $compiler /V2 /WX "/DAPP_VERSION=$version" "/DAPP_EXE=$exe" "/DOUTPUT_FILE=$output" (Join-Path $PSScriptRoot 'noia-dnf.nsi')
    if ($LASTEXITCODE -ne 0) { throw 'NSIS compilation failed.' }
    $installer = Get-Item -LiteralPath $output
    $manifest = [ordered]@{
        schema_version = 1
        version = $version
        published_at = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ssZ')
        changelog = $notes
        assets = @{ 'windows-x86_64' = [ordered]@{
            url = "$releases/download/v$version/$($installer.Name)"
            size = $installer.Length
            sha256 = (Get-FileHash -LiteralPath $output -Algorithm SHA256).Hash.ToLowerInvariant()
        }}
    }
    $manifestPath = Join-Path $outDir.FullName 'latest.json'
    [IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
    $zip = Join-Path $metadata.target_directory 'Noia-DNF-windows-x64.zip'
    Compress-Archive -LiteralPath $exe -DestinationPath $zip -Force
    Write-Output $output
    Write-Output $manifestPath
    Write-Output $zip
} finally {
    Pop-Location
}
