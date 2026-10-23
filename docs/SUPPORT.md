# Language and backend support

There are four distinct boundaries:

1. `analyze(source)` parses and checks the language, producing typed AST or diagnostics.
2. `compile_typed_ir(source, mcu, options)` constructs **Helium IR v1** for inspection and interpreted tests.
3. `compile_ir(source, mcu)` preserves the narrower **legacy untyped** inspection API.
4. `check` / `compile_assembly` require the implemented AVR subset. HEX builds additionally run AVRA.

`--emit ir` selects typed IR; `--emit legacy-ir` selects legacy inspection output.
The table below describes the older language/legacy/AVR boundaries. Typed IR v1 adds executable
integer/bool storage, add/subtract/multiply/negate, comparisons, ternary, while, short-circuit logic,
pin inversion and INPUT_PULLUP. Integer widths 8/16/32/64 are modeled and interpreted.
Function signatures/calls/returns exist in the IR, but not yet in source syntax.
For ranges, division, shifts, float, strings and computed PinMode values remain unsupported.
See [IR_V1.md](IR_V1.md) for the complete contract and verifier policy.

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
Register allocation, general memory layout, integer machine lowering and branch relaxation remain
backend work. Typed-IR interpretation does not prove AVR timing, ABI or physical behavior.

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
- Integer addition, subtraction, multiplication and signed negation wrap to the operation's width,
  including signed overflow. Invalid source literals remain errors. Division and shift rules are pending.
  Arithmetic is executable in the typed IR interpreter, not yet in generated AVR code.
- `!bool` returns bool. `!PinState` swaps HIGH and LOW. `!Pin` returns PinState by reading the
  actual input level exactly once and inverting it. Negation itself never writes a pin.
  Thus `Led = !Led` is valid language syntax/types, but not generated AVR code in this iteration.
- `and` and `or` have short-circuit semantics, implemented in typed IR; the legacy AVR backend rejects them.
- For loops introduce their own scoped loop variable, which cannot shadow an existing name.
  Executable range/overflow behavior will be specified with their lowering.
- `setup` must precede `loop`; global variables and pins precede both. Exactly one loop is required.
- Pin checks describe chip register capabilities, not board wiring, fuse settings or electrical suitability.

All compiler stages use the same diagnostic renderer and structured source spans.
`DiagnosticKind` carries the error and its parameters; `DiagnosticCode` is an enum
whose `as_str()` preserves public identifiers such as `E_PARSE` and `E_LITERAL_RANGE`.
Library callers can inspect kinds, codes and spans without matching translated text.

`Diagnostic::render()` produces plain text. `render_with()` accepts explicit rendering
options, without reading environment variables or changing global color settings.
The CLI supports `--color auto|always|never`; auto enables color only on a terminal
and respects `NO_COLOR`. An explicit `always` or `never` overrides automatic policy.
Tabs in excerpts use four-column tab stops; offsets continue to count graphemes.

Messages are currently English. Russian/English selection and a default-language
setting are deferred. Semantic/backend messages are centralized in
`src/diagnostic/messages.rs`; parser/scanner errors retain their typed variants and
plain English details. Those legacy detail strings also need cataloguing when the
translation is implemented. No language option is advertised until both locales work.
