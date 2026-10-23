$ErrorActionPreference = 'Stop'
$heliumRoot = Split-Path $PSScriptRoot -Parent
$downloads = Join-Path $heliumRoot 'build/toolchain-downloads'
$installation = Join-Path $heliumRoot 'build/toolchain'
New-Item -ItemType Directory -Force $downloads, $installation | Out-Null

# Pinned official MSYS2 packages, Arduino AVR tools and AVR8js. No installer,
# registry changes, global PATH changes or package lifecycle scripts are run.
$packages = Get-Content (Join-Path $PSScriptRoot 'toolchain-lock.json') -Raw | ConvertFrom-Json
foreach ($package in $packages) {
    $archive = Join-Path $downloads $package.file
    if (!(Test-Path -LiteralPath $archive)) {
        Write-Host "Downloading $($package.name)"
        Invoke-WebRequest $package.url -OutFile $archive
    }
    if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $package.sha256) {
        throw "SHA256 mismatch: $archive. Remove the damaged download and retry."
    }

    $destination = $installation
    if ($package.subdirectory) { $destination = Join-Path $installation $package.subdirectory }
    New-Item -ItemType Directory -Force $destination | Out-Null
    & tar -xf $archive -C $destination
    if ($LASTEXITCODE) { throw "Extraction failed: $archive" }
}

. (Join-Path $PSScriptRoot 'activate-toolchain.ps1')
$targets = & (Join-Path $env:HELIUM_LLVM_BIN 'llc.exe') --version
if ($LASTEXITCODE -or !($targets -match '^\s+avr\s+-')) { throw 'LLVM AVR target is unavailable' }
& (Join-Path $env:HELIUM_AVR_BIN 'avr-gcc.exe') --version
if ($LASTEXITCODE) { throw 'AVR GCC cannot run' }
Write-Host 'Toolchain ready. Dot-source tools/activate-toolchain.ps1 in new shells.'
