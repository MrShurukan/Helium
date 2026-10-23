# Dot-source this file to configure tools for this PowerShell session only.
$heliumRoot = Split-Path $PSScriptRoot -Parent
$env:HELIUM_LLVM_BIN = Join-Path $heliumRoot 'build/toolchain/ucrt64/bin'
$env:HELIUM_AVR_BIN = Join-Path $heliumRoot 'build/toolchain/avr/bin'
$env:HELIUM_AVR8JS = Join-Path $heliumRoot 'build/toolchain/avr8js/package'
