param(
    # Use explicit HELIUM_* overrides or PATH instead of the bundled installation.
    [switch]$UseConfiguredEnvironment
)

$ErrorActionPreference = 'Stop'
$heliumRoot = Split-Path $PSScriptRoot -Parent
if (!$UseConfiguredEnvironment) {
    . (Join-Path $PSScriptRoot 'activate-toolchain.ps1')
}

function Invoke-Checked {
    param([string]$Executable, [string[]]$Arguments)
    & $Executable @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "Failed ($LASTEXITCODE): $Executable $Arguments"
    }
}

function Get-HeliumTool {
    param([string]$Name, [string]$Directory)
    if ($Directory) { return Join-Path $Directory "$Name.exe" }
    return $Name
}

# Fail before tests if any machine-test dependency is missing or cannot start.
# The full gate never interprets a missing toolchain as a successful skipped test.
foreach ($name in @('llc', 'opt')) {
    $tool = Get-HeliumTool $name $env:HELIUM_LLVM_BIN
    $version = Invoke-Checked $tool @('--version')
    if ($name -eq 'llc' -and !($version -match '^\s+avr\s+-')) {
        throw 'LLVM has no AVR target. Run tools/setup-toolchain.ps1.'
    }
}
foreach ($name in @('avr-gcc', 'avr-size', 'avr-nm', 'avr-objdump', 'avr-objcopy')) {
    $tool = Get-HeliumTool $name $env:HELIUM_AVR_BIN
    $null = Invoke-Checked $tool @('--version')
}
$node = if ($env:HELIUM_NODE) { $env:HELIUM_NODE } else { 'node' }
$null = Invoke-Checked $node @('--version')

Push-Location $heliumRoot
try {
    Invoke-Checked $node @((Join-Path $PSScriptRoot 'check-simulator.cjs'))
    Invoke-Checked 'cargo' @('fmt', '--check')
    Invoke-Checked 'cargo' @('clippy', '--locked', '--all-targets', '--', '-D', 'warnings')
    Invoke-Checked 'cargo' @('test', '--locked', '--', '--include-ignored')
    Invoke-Checked 'cargo' @('test', '--locked', '--release', '--', '--include-ignored')
    Write-Host 'Full gate passed: formatting, Clippy, debug and release, including machine tests.'
} finally {
    Pop-Location
}
