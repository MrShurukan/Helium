# Build six test firmwares only. Never opens a serial port or flashes a board.
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot '../activate-toolchain.ps1')
function Checked($exe, $arguments) {
    & $exe @arguments
    if ($LASTEXITCODE -ne 0) { throw "Failed: $exe" }
}
Push-Location (Join-Path $PSScriptRoot '../..')
try {
    Checked 'cargo' @('run','--quiet','--example','prepare_gpio_board')
    Checked 'cargo' @('run','--quiet','--example','prepare_gpio_board','--','--optimize-ir')
    $boardDir=Join-Path (Get-Location) 'build/board-gpio'
    Checked (Join-Path $env:HELIUM_AVR_BIN 'avr-gcc.exe') @('-mmcu=atmega328p','-Os','-c','tools/board-gpio/telemetry.c','-o',"$boardDir/telemetry.o")
    foreach ($ir in @('plain-ir','optimized-ir')) {
        foreach ($level in @('none','speed','size')) {
            $stem="$boardDir/$ir-$level"
            $source=Get-Content "$boardDir/$ir.ll" -Raw
            if ($level -eq 'size') {
                $source=($source -split "`n" | ForEach-Object {
                    if ($_.StartsWith('define ')) { $_.Replace('addrspace(1)', 'addrspace(1) minsize optsize') } else { $_ }
                }) -join "`n"
            }
            [System.IO.File]::WriteAllText("$stem.ll",$source)
            $pipeline=if ($level -eq 'none') {'default<O0>'} else {'default<O2>'}
            $machineLevel=if ($level -eq 'none') {'-O0'} else {'-O2'}
            Checked (Join-Path $env:HELIUM_LLVM_BIN 'opt.exe') @('-S','-mtriple=avr-unknown-unknown','-mcpu=atmega328p',"-passes=$pipeline","$stem.ll",'-o',"$stem.opt.ll")
            Checked (Join-Path $env:HELIUM_LLVM_BIN 'llc.exe') @('-mtriple=avr-unknown-unknown','-mcpu=atmega328p',$machineLevel,'-filetype=obj',"$stem.opt.ll",'-o',"$stem.o")
            Checked (Join-Path $env:HELIUM_AVR_BIN 'avr-gcc.exe') @('-mmcu=atmega328p',"$stem.o","$boardDir/telemetry.o",'-o',"$stem.elf")
            Checked (Join-Path $env:HELIUM_AVR_BIN 'avr-objcopy.exe') @('-O','ihex','-R','.eeprom',"$stem.elf","$stem.hex")
        }
    }
} finally { Pop-Location }
