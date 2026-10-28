# Helium

Helium is a small, statically typed language for AVR microcontrollers, implemented in Rust.
The current executable target is **ATmega328P**. LLVM is the only machine backend.
See [supported language features](docs/SUPPORT.md) before using a feature.
For architecture, decisions, review priorities and current limitations, start with
the [review handoff](docs/HANDOFF.md).

## Build and test

Install stable Rust. On Windows this project works with the GNU toolchain;
MSVC additionally needs the Microsoft linker/SDK.

```text
cargo build --locked
cargo test --locked
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
```

Ordinary tests require no external compiler or connected board. Machine output requires
LLVM with AVR support and the AVR GNU toolchain. On Windows:

```powershell
./tools/setup-toolchain.ps1
. ./tools/activate-toolchain.ps1
./tools/test-all.ps1
```

The setup script installs pinned tools under `build/toolchain/`. Machine execution tests
also require Node.js and AVR8js. Tests never flash hardware. AVRA is no longer a dependency.
See [toolchain setup and measurements](docs/LLVM_PROTOTYPE.md).

`tools/test-all.ps1` checks every external dependency, formatting and Clippy, then runs
all tests (including machine tests) sequentially in debug and release. Missing tools
fail the command. Use `-UseConfiguredEnvironment` to honor existing HELIUM_* overrides
or PATH instead of the bundled toolchain. No hardware is flashed.
GitHub Actions runs host tests, formatting and Clippy on Windows and Linux; the full
machine gate is a separate required local check before a release.

## Commands

```text
cargo run --locked -- check examples/functions-latch.he --mcu atmega328p
cargo run --locked -- build examples/functions-latch.he --mcu atmega328p --opt size
cargo run --locked -- build examples/functions-latch.he --mcu atmega328p --emit llvm
cargo run --locked -- build examples/functions-latch.he --mcu atmega328p --emit asm
cargo run --locked -- build examples/functions-latch.he --mcu atmega328p --emit elf
cargo run --locked -- build examples/functions-latch.he --mcu atmega328p --emit ir --verify-ir --optimize-ir
cargo run --locked -- build examples/functions-latch.he --mcu atmega328p --emit ast
```

`check` validates the executable subset without launching external tools. It does not
validate the installed toolchain or replace a full build. `build` defaults to HEX and
**does not flash**. There is no backend selection option.

`ast`, `ir` and `llvm` output require only Helium; `llvm` uses the `.ll` extension.
`asm`, `elf` and `hex` invoke external tools. Assembly uses GNU/LLVM AVR syntax.
The library exposes `analyze`, `compile_typed_ir`, `compile_llvm`, `check` and
`check_with_options(source, mcu, Options)` for clock/configuration-aware checks.

`analyze` returns one `AnalyzedProgram`: `statements()` exposes the analyzed tree,
`symbols()` its resolved declarations and `shared_analysis()` the call/resource graph.
The result is read-only. Symbol IDs are local to that result, not persistent IDs across edits.
There is no separate AST-only analysis path; `build --emit ast` displays the complete result.

Internal IR verification is enabled by default in both debug and release.
`--verify-ir` remains an explicit assertion of that policy; the library can set `Options.verify=false`.
Source validation is always enabled. `--optimize-ir` runs experimental Helium passes
on `--emit ir` only. `--opt none|speed|size` controls machine optimization through LLVM.
AST/Helium IR inspection can accept features outside the current machine subset.

Outputs default to `build/` beside the source; `-o path/to/output.hex` selects another path.
Machine builds discard temporary artifacts by default. Add `--intermediates` to retain
them in a unique directory beside the output, or `--intermediates build/inspect` to
update the same directory on each successful build (relative paths use the current directory):

```text
helium build examples/uart-array.he --mcu atmega328p --cpu-hz 16000000 --opt size --intermediates build/inspect
```

That folder contains `module.ir` (Helium IR, matching `--emit ir`), `module.ll`
(LLVM input), `optimized.ll` (LLVM after the selected optimization) and `program.hex`,
plus assembly, ELF and reports. No repeated builds or output-extension changes are needed.
The flag also works with explicit `--emit hex|elf|asm`; it does not apply to textual emits
or `check`. `--intermediates=path` is also accepted. Known compiler artifact names are
replaced, stale compiler artifacts are removed, and unrelated files are preserved.
Do not run concurrent builds into the same fixed artifact directory.
Failed external stages do not replace an existing destination firmware.
ELF/HEX builds report Flash occupation, static SRAM reservation and remaining SRAM
for the stack using the linked ELF and target capacities. Maximum stack and ISR
latency remain explicitly unknown. With `--intermediates`, the artifact directory retains `memory.txt` and
`timing.txt` (source IRQ-mask/call inventory, not a timing bound). See
[memory and timing](docs/MEMORY_AND_TIMING.md) for accounting and limitations.

## Language and diagnostics

```text
fn add(left:u8, right:u8):u8 {
    return left + right;
}
```

Calls may precede declarations. Parameters are mutable local copies. Omit the result
type for procedures. Recursion and shadowing are forbidden. Global initializer calls
remain unsupported; use setup for runtime calls. See [the function contract](docs/FUNCTIONS.md).

`noInterrupt { ... }` saves and restores interrupt state, including nested blocks and
early returns. `noInterrupt fn` declares a checked calling requirement; it does not
automatically disable interrupts. See [the interrupt contract](docs/INTERRUPTS.md) and
[the example](examples/no-interrupt.he). Timer1 and ISR support are described below.

Diagnostics contain exact primary locations and semantic related labels. Missing-return
errors identify fallthrough paths; type errors can reference the declaration that required
the type; name conflicts show the first declaration. The CLI prints file/line/column,
source excerpts and `--color auto|always|never` controls color. Auto respects `NO_COLOR`.

Messages are currently English. Russian translation and an LSP server are future work.
Structured diagnostics and checked file/byte ranges prepare for editor integration without
requiring an editor to parse terminal output. See [diagnostic contract and audit](docs/DIAGNOSTICS.md).

## Hardware examples

[Latch with functions](examples/functions-latch.he) uses D2 to GND, INPUT_PULLUP and the
Nano's built-in D13 LED. It has no debounce yet. The user confirmed an earlier latch works
on Nano; newly generated firmware still needs its own hardware validation.
[Temperature hysteresis](examples/functions-thermostat.he) uses synthetic temperatures.

Flashing is separate and explicit; AVRDUDE is required only for this operation:

```text
cargo run --locked -- flash build/firmware.hex --mcu atmega328p --programmer usbasp
```

USB bootloader uploads may need board-specific port and baud settings in AVRDUDE directly.

## Development

- [Development plan](docs/DEVELOPMENT_PLAN.md)
- [Roadmap, technical debt and accepted limitations](docs/ROADMAP.md)
- [Current target description and B2 validation](docs/TARGET.md)
- [Atomic GPIO contract and B2a board validation](docs/ATOMIC_GPIO.md)

- [Support and semantic rules](docs/SUPPORT.md)
- [Typed IR contract](docs/IR_V1.md)
- [LLVM toolchain and historical measurements](docs/LLVM_PROTOTYPE.md)
- [Function contract](docs/FUNCTIONS.md)
- [Diagnostics and audit](docs/DIAGNOSTICS.md)
- [Grammar](grammar.txt)
- [Historical first iteration](docs/ITERATION_1.md)

Next application: temperature control for one or two brewing tanks, addressed Dallas
sensors, encoder/menu, LCD, outputs and UART. Wider integer machine support is a separate
iteration; it is not included in the backend removal and diagnostics work.

## Timer1

See [Timer1 and ISR](docs/TIMERS.md) for syntax, shared-access rules and limitations.
Programs with a timer require an explicit clock, for example:

```text
cargo run --locked -- check examples/timer-seconds.he --mcu atmega328p --cpu-hz 16000000
cargo run --locked -- timer-options --mcu atmega328p --cpu-hz 16000000 --timer Timer1 --every 1s
```

## UART TX and warnings

[UART TX](docs/UART_API_PROPOSAL.md) supports UART0 polling at a fixed 8N1 format.
Try `cargo run -- check examples/uart-tx.he --mcu atmega328p --cpu-hz 16000000`.
Blocking calls in an ISR are allowed with an unsuppressible `W_ISR_BLOCKING` warning;
shared-access protection still applies. See [call contracts](docs/CALL_EFFECTS.md).

## Extensions and the first std helper

```text
import extension std::uart::UartText;

device debug = UART0 { baud: 38400, mode: TX, }
setup { debug.newline(); debug.flush(); }
loop {}
```

See [the runnable example](examples/uart-extensions.he) and
[the module/extension contract](docs/STD_EXTENSIONS_PROPOSAL.md). Std source lives in
`lib/std/` and is bundled with the compiler. Application modules resolve relative
to the entry file, not the shell working directory. Static `device output: UartTx`
parameters preserve hardware identity without runtime pointers. Owning strings
and wrapper structs remain future work.

Fixed scalar arrays, temporary slices and `.clone()` are implemented: see
[the runnable slice/clone example](examples/uart-slice.he) and
[the array and borrowing contract](docs/ARRAYS.md). A slice helper accepts arrays
of different lengths; cloning an array creates an independent owner.

Текстовый вывод: [пример print_n](examples/uart-text.he), [контракт &str и результаты](docs/TEXT.md).
UTF-8 литералы находятся во Flash; std::uart::UartText предоставляет print/println.
