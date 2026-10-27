# Explicit physical test. Requires a Nano with old bootloader at 16 MHz,
# free D12/D13, and approval to overwrite its application firmware.
param([Parameter(Mandatory=$true)][string]$Port)
$ErrorActionPreference='Stop'
Push-Location (Join-Path $PSScriptRoot '../..')
try {
    $results=@()
    foreach ($ir in @('plain-ir','optimized-ir')) {
        foreach ($level in @('none','speed','size')) {
            $name="$ir-$level"
            & avrdude -p m328p -c arduino -P $Port -b 57600 -D -U "flash:w:build/board-gpio/$name.hex:i" *> "build/board-gpio/$name.upload.log"
            if ($LASTEXITCODE -ne 0) { throw "Upload failed: $name" }
            $serial=[System.IO.Ports.SerialPort]::new($Port,9600,[System.IO.Ports.Parity]::None,8,[System.IO.Ports.StopBits]::One)
            $serial.ReadTimeout=500
            $serial.DtrEnable=$true
            $samples=@()
            try {
                $serial.Open()
                $timer=[System.Diagnostics.Stopwatch]::StartNew()
                while ($timer.Elapsed.TotalSeconds -lt 8) {
                    try { $line=$serial.ReadLine().Trim() } catch [System.TimeoutException] { continue }
                    if ($line -match '^B2A ([0-9A-F]{4}) ([0-9A-F]{4}) ([0-9A-F]{4})$') {
                        $samples += [pscustomobject]@{seconds=$timer.Elapsed.TotalSeconds;ticks=[Convert]::ToInt32($Matches[1],16);errors=[Convert]::ToInt32($Matches[2],16);progress=[Convert]::ToInt32($Matches[3],16)}
                        if ($samples.Count -ge 10 -and ($samples[-1].seconds-$samples[0].seconds) -ge 2) { break }
                    }
                }
            } finally { $serial.Dispose() }
            $samples | Export-Csv "build/board-gpio/$name.samples.csv" -NoTypeInformation
            if ($samples.Count -lt 10) { throw "Insufficient telemetry: $name ($($samples.Count) frames)" }
            if (@($samples | Where-Object {$_.errors -ne 0}).Count) { throw "GPIO mismatch: $name" }
            $ticks=0; $progress=0
            for ($i=1; $i -lt $samples.Count; $i++) {
                $ticks += ($samples[$i].ticks-$samples[$i-1].ticks+65536)%65536
                $progress += ($samples[$i].progress-$samples[$i-1].progress+65536)%65536
            }
            if ($ticks -le 1000 -or $progress -le 0) { throw "No runtime progress: $name" }
            $result=[pscustomobject]@{configuration=$name;frames=$samples.Count;seconds=$samples[-1].seconds-$samples[0].seconds;ticks=$ticks;iterations=$progress;errors=0}
            $results += $result
            $results | ConvertTo-Json | Set-Content -Encoding utf8 build/board-gpio/results.json
            $result | Format-List
        }
    }
} finally { Pop-Location }
