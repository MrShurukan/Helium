param([string]$Compiler = '')
$ErrorActionPreference = 'Stop'
$heliumRoot = Split-Path $PSScriptRoot -Parent
if (!$Compiler) { $Compiler = Join-Path $heliumRoot 'target/release/helium.exe' }
. (Join-Path $PSScriptRoot 'activate-toolchain.ps1')
$outputRoot = Join-Path $heliumRoot 'build/llvm-benchmark'
New-Item -ItemType Directory -Force $outputRoot | Out-Null
$rows = @()

foreach ($example in @('llvm-gpio', 'typed-ir', 'llvm-counter')) {
    foreach ($mode in @('none', 'speed', 'size')) {
        $destination = Join-Path $outputRoot "$example-$mode.hex"
        $watch = [Diagnostics.Stopwatch]::StartNew()
        $output = & $Compiler build (Join-Path $heliumRoot "examples/$example.he") --mcu atmega328p --opt $mode -o $destination
        $watch.Stop()
        if ($LASTEXITCODE) { throw "Build failed: $example $mode" }
        $directory = ($output | Where-Object { $_ -like 'Intermediates: *' }) -replace '^Intermediates: ', ''

        $sections = @{}
        foreach ($line in Get-Content (Join-Path $directory 'sections.txt')) {
            if ($line -match '^\s*(\.\S+)\s+(\d+)\s+') { $sections[$Matches[1]] = [int]$Matches[2] }
        }
        $userCode = 0
        foreach ($line in Get-Content (Join-Path $directory 'symbols.txt')) {
            if ($line -match '^[0-9a-f]+\s+([0-9a-f]+)\s+[tT]\s+(main|helium_f\d+)$') {
                $userCode += [Convert]::ToInt32($Matches[1], 16)
            }
        }
        $rows += [pscustomobject]@{
            example=$example; mode=$mode; flash=($sections['.text']+$sections['.data']);
            userCode=$userCode; runtimeAndPadding=($sections['.text']-$userCode);
            staticRam=($sections['.data']+$sections['.bss']+$sections['.noinit']);
            elapsedMs=$watch.ElapsedMilliseconds; directory=$directory
        }
    }
}
$rows | ConvertTo-Json | Set-Content (Join-Path $outputRoot 'results.json')
$rows | Select-Object example,mode,flash,userCode,runtimeAndPadding,staticRam,elapsedMs | Format-Table
