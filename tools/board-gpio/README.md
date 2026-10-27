# Physical GPIO fixture (explicit opt-in)

This is a test fixture, not a Helium UART feature. `program.he` is compiled by Helium;
`telemetry.c` only snapshots exported counters and reports them. The Rust preparation
example inserts a call after the lifecycle loop body, using its actual FunctionId.

Requires a classic 16 MHz Nano with old bootloader, a known serial port, and free
D12/D13. It drives both outputs. Confirm wiring and permission before flashing.
Serial reporting uses D1/TX; do not attach another driver to this UART.

1. Run `./tools/board-gpio/prepare.ps1` to build six HEX files. This does not flash.
2. Back up flash using `avrdude -p m328p -c arduino -P COM5 -b 57600
   -U flash:r:build/board-gpio/original-flash.hex:i` (substitute the approved port;
   preserve an earlier backup instead of overwriting it).
3. Run `./tools/board-gpio/run.ps1 -Port COM5` with explicit approval. This overwrites
   application firmware six times and verifies each upload, then reads 9600-baud
   telemetry. CSV and results.json are written under build/board-gpio.
4. Restore the original application when testing ends. The run script does **not**
   restore automatically. Preserve bootloader/fuses: restore only application pages
   touched by the test, taken from the backup, and verify by reading the full flash.
   Do not blindly write a full flash backup through a bootloader.

The telemetry format is `B2A TTTT EEEE PPPP` (hex u16 tick, error, progress counters).
The reader accounts for counter wrapping and requires both runtime contexts to advance.
It fails on any nonzero error count. It is a bounded physical smoke test; timing comes
from host UART receipt and is not a precision measurement.

The normal test gate never runs this procedure or accesses a serial port.
Recorded results and limitations are in docs/ATOMIC_GPIO.md.
