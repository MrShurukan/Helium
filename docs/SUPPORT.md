# Language and backend support

There are three distinct boundaries:

1. `analyze(source)` parses and checks the language, producing typed AST or diagnostics.
2. `compile_ir(source, mcu)` also checks device pins and emits the supported **legacy untyped** IR subset.
3. `check` / `compile_assembly` require the implemented AVR subset. `build` additionally runs AVRA for HEX output.

| Feature | Language analysis | Legacy IR inspection | AVR assembly / HEX |
| --- | --- | --- | --- |
| Multiple `def` declarations | Yes | Yes | Yes |
| INPUT / OUTPUT | Yes | Yes | Yes |
| INPUT_PULLUP | Yes | Explicit unsupported error | Explicit unsupported error |
| INPUT_PULLDOWN on ATmega328P | Recognized syntax | Target error | Target error |
| `setup` then mandatory `loop` | Yes | Yes | Yes |
| Blocks and `if` / `else` | Yes | If its expressions are supported | Conditions must be `pin == HIGH/LOW` (either operand order) |
| Pin assignment of HIGH/LOW, INPUT/OUTPUT | Yes | Yes, including chained assignments | Yes |
| Local/global integer variables, assignment and addition | Yes | Nonnegative literals, reads, stores, addition | Explicit unsupported error |
| Boolean variables/literals | Yes | Loads/stores | Explicit unsupported error |
| `!bool`, `!PinState`, `!Pin` | Yes | Explicit unsupported error | Explicit unsupported error |
| `and` / `or` | Yes, bool operands only | Explicit unsupported error | Explicit unsupported error |
| Other numeric operations/comparisons, ternary | Yes, with type checks | Explicit unsupported error | Explicit unsupported error |
| `while` / `for` | Yes | Explicit unsupported error | Explicit unsupported error |
| Floating-point values and strings | Basic analysis only | Explicit unsupported error | Explicit unsupported error |
| Functions, calls, modules, arrays, structures | Not implemented | Not implemented | Not implemented |
| Timers, interrupts, UART and device libraries | Not implemented | Not implemented | Not implemented |

`check` is not a substitute for the assembler: AVRA still checks instruction encoding and branch ranges.
In particular, very large branch bodies may exceed the current short conditional-branch range.
Register allocation, general memory layout, arithmetic widths, overflow semantics and branch relaxation
belong to the next IR/backend iteration. Legacy IR arithmetic must not be treated as executable AVR support.

## Agreed language rules

- A declaration cannot reuse any currently visible name, including a pin name. Shadowing is forbidden.
  Names may be reused in separate sibling blocks. Setup locals are not visible in loop.
- A variable is not visible inside its initializer. Declarations with a concrete numeric/bool type
  and no initializer get zero/false. Strings currently get an empty default value at the AST level only.
- Unsuffixed integers infer i32, floating-point literals infer f32. Explicit integer literal contexts
  are checked against the target range, including negative signed minima and unsigned maxima.
- Implicit conversions between concrete numeric types must preserve all values of the source type.
  Integer literals converted to a float must be exactly representable. Decimal floating-point literals
  use normal floating-point rounding; this is not a promise of decimal exactness.
- Arithmetic overflow behavior is not yet finalized; arithmetic is not exposed by the AVR backend.
- `!bool` returns bool. `!PinState` swaps HIGH and LOW. `!Pin` returns PinState by reading the
  actual input level exactly once and inverting it. Negation itself never writes a pin.
  Thus `Led = !Led` is valid language syntax/types, but not generated AVR code in this iteration.
- `and` and `or` have short-circuit semantics. Code generation rejects them until it can preserve that behavior.
- For loops introduce their own scoped loop variable, which cannot shadow an existing name.
  Executable range/overflow behavior will be specified with their lowering.
- `setup` must precede `loop`; global variables and pins precede both. Exactly one loop is required.
- Pin checks describe chip register capabilities, not board wiring, fuse settings or electrical suitability.

Diagnostics from semantic analysis and target support have stable codes and source spans.
Parser/scanner diagnostics retain their original source excerpts under the E_PARSE code;
their migration to fully structured labels is deferred.
