# Helium

Helium is a small, statically typed language for AVR microcontrollers, implemented in Rust.
The current executable target is **ATmega328P**. This is an experimental compiler: the parser
and type checker intentionally recognize more of the language than the AVR backend supports.
See [the support matrix](docs/SUPPORT.md) before using a feature.

## Build and test

Install a stable Rust toolchain using rustup. On Windows, the GNU toolchain is sufficient for
this project; the MSVC toolchain additionally requires the Microsoft linker/SDK.
AVRA 1.4.2 is required only for producing HEX files. AVRDUDE is required only for explicit flashing.
Both external executables must be on PATH.

```text
cargo build --locked
cargo test --locked
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --test cli -- --ignored
```

The final command runs the AVRA integration tests. These tests never flash hardware.
Ordinary tests do not require AVRA, AVRDUDE or a connected board.
The current development environment was verified with Rust 1.99.1 (Windows GNU) and AVRA 1.4.2.

## Commands

```text
cargo run --locked -- check goal.he --mcu atmega328p
cargo run --locked -- build goal.he --mcu atmega328p
cargo run --locked -- build goal.he --mcu atmega328p --emit asm
cargo run --locked -- build goal.he --mcu atmega328p --emit ir
cargo run --locked -- build goal.he --mcu atmega328p --emit ast
```

`check` checks syntax, types and the implemented AVR subset without invoking an external tool.
`build` defaults to HEX and invokes AVRA; it **does not flash**.
`--emit asm`, `ir` and `ast` stop at the requested stage and require only Helium.
AST output includes the inferred types. IR output is the legacy inspection representation,
not a guarantee that the program can be compiled into AVR instructions.

By default outputs go into `build/` beside the source file, regardless of the current working directory.
Use `-o path/to/result.asm` (or `.hex`, `.ir`, `.ast`) to choose a file explicitly.
ATmega328P definitions are embedded in the compiler and assembly output, so neither execution
nor assembling the resulting `.asm` depends on finding `assembly_includes/` at runtime.
A HEX build retains the assembly file and may also create AVRA object/EEPROM sidecar files.

Flashing is a separate, explicit command:

```text
cargo run --locked -- flash build/goal.hex --mcu atmega328p --programmer usbasp
```

The programmer must be supplied. Flashing was not tested against physical hardware during this iteration.
The old positional command (`helium file.he atmega328p`) is replaced by these commands.

## Current example

`goal.he` and `test.he` demonstrate reading a button and driving an LED.
The integration test verifies that `goal.he` assembles to **40 bytes of code** and validates the Intel HEX checksums.
This is a regression baseline for this example, not a general size comparison with other languages.

The unfinished latch example from the old WIP commit is retained in
[examples/planned/latch.he](examples/planned/latch.he). It passes language analysis but is deliberately
rejected by the current AVR support check. It is not a working firmware example yet.

## Development

- [Language and backend support](docs/SUPPORT.md)
- [Architecture and iteration-one review](docs/ITERATION_1.md)
- [Grammar](grammar.txt)

The next end-to-end application is temperature control for one or two brewing tanks:
addressed Dallas temperature sensors, an encoder/menu, an LCD, two indicator outputs and UART diagnostics.
Exact sensor and LCD interfaces will be selected before implementing drivers.
