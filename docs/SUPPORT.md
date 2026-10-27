# Language support

LLVM is the only machine backend. No `--backend` or `--emit legacy-ir` option exists.
AVRA, the old linear IR and hand-written AVR assembly generation have been removed.

| Feature | Analysis | Helium IR / interpreter | LLVM machine output |
| --- | --- | --- | --- |
| bool, PinState | Yes | Yes | Yes |
| i8/u8, i16/u16 | Yes | Yes | Yes |
| i32/u32, i64/u64 | Yes | Yes | Located unsupported-type diagnostic |
| Float and string | Basic type checks | Not implemented | Not implemented |
| Variables, assignment, widening, + / - / * | Yes | Yes | Yes for supported types |
| Comparisons, short-circuit and/or, ternary, if, while | Yes | Yes | Yes |
| fn, arguments by value, explicit return | Yes | Yes | Yes for supported types |
| Direct or indirect recursion | Rejected | — | — |
| Shadowing | Rejected | — | — |
| for ranges | Integer bound checks | Not implemented | Not implemented |
| Division | Numeric type checks | Not implemented | Not implemented |
| GPIO reads/writes, !Pin, INPUT_PULLUP | Yes | Yes | Yes |
| Internal pull-down on ATmega328P | Recognized | Rejected | Rejected |
| Stored/computed PinMode values | Basic type checks | Not implemented | Not implemented |
| Calls in global initializers | Rejected | — | — |
| `noInterrupt` blocks and function requirements | Checked calls; nested scopes and early returns | Yes | Yes |
| Timer1 CTC, embedded ISR, shared-access checks | Yes; explicit CPU clock | Verified IR; interpreter has no asynchronous dispatch | Yes |
| UART0 TX polling, 8N1 | Typed methods, resources, ISR warnings | Verified IR; interpreter reports unsupported execution | Yes |
| Static UartTx parameters, named extensions, app/std modules | File-local imports, visibility, receiver registry | Specialized functions; no runtime device arguments | Yes |
| Heap, user structs/traits, other timers, other peripheral drivers | Not implemented | — | — |

`analyze` parses, type-checks and returns one `AnalyzedProgram` with resolved symbol
identities and a shared-access graph; use `statements()` to inspect its AST.
Setup-only and unused functions do not create runtime shared-access conflicts, but
all source bodies still undergo type and explicit noInterrupt-contract checks. `compile_typed_ir` also checks the executable Helium IR
subset. `compile_llvm` and `check` validate the machine subset before emitting LLVM IR.
`check_with_options` accepts the same Options (including cpu_hz) as `compile_llvm`.
The simple `check` wrapper uses defaults. IR verification defaults to enabled in every profile.
`check` needs no external tools; a full build still validates LLVM lowering and linking.
Restrictions apply to unreachable source too, so optimization does not silently determine
whether an unsupported feature is accepted.

After analysis, all expression types are concrete, including operands of comparisons
and statements after return. Unconstrained integer expressions default to i32 before
the machine support check and receive located unsupported-type diagnostics. A signed
literal such as -128:i8 is validated as a whole, not as a separate positive i8 token.
Computed PinMode expressions and first-class pin selection are rejected before lowering;
constant (including grouped/chained) pin mode statements remain supported.

## Semantic rules

- Names cannot shadow a visible variable, parameter, pin or function. Separate sibling scopes
  can reuse local names. Functions can be called before declaration.
- Integer literals must fit their contextual type. Unsuffixed, unconstrained integers default
  to i32, which currently needs an explicit smaller type for machine compilation.
- Implicit integer conversions must be lossless. Arithmetic wraps in the operation's width;
  widening its result does not undo an earlier overflow.
- Arguments and expression operands evaluate left to right. `and`, `or` and ternary evaluate
  only the required operand/branch.
- `!bool` returns bool; `!PinState` swaps HIGH/LOW; `!Pin` reads the physical input exactly
  once and inverts it. It does not write anything by itself. `Led = !Led` performs the write.
- Missing-return analysis is structural: loops may execute zero times. A later return can
  cover an earlier conditional's fallthrough paths. Constant-condition proof is not implemented.
- Globals/pins precede setup/loop; setup precedes loop; exactly one loop is required.
- GPIO validation describes MCU capabilities, not board wiring, fuses or electrical behavior.

See [diagnostics](DIAGNOSTICS.md) for ranges, related locations and the current audit.

GPIO writes preserve other pins on the same port through explicit atomic bit
instructions. Main/ISR accesses to distinct pins are allowed; a shared pin with
writes or mode changes still requires noInterrupt. Shared memory rules are unchanged.
See [atomic GPIO contract and validation](ATOMIC_GPIO.md).

`compile_llvm[_with_target]` returns `Compilation { output, warnings }`;
`check[_with_options]` returns warnings on success. AnalyzedProgram and Module also
retain source warnings. Warnings do not fail compilation and have no suppression
option yet. See [CALL_EFFECTS.md](CALL_EFFECTS.md) and [UART_API_PROPOSAL.md](UART_API_PROPOSAL.md).

Use `modules::Project` for explicit app/std roots or in-memory module texts, then
`compile_project_ir` / `compile_project_llvm`. In-memory `analyze` and `compile_llvm`
support bundled std but do not implicitly read app modules from cwd. See
[extensions and modules](STD_EXTENSIONS_PROPOSAL.md). UART execution still requires
the machine simulator; the IR interpreter does not emulate the peripheral.
