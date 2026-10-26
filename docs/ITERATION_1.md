> Historical record of iteration one. The legacy pipeline described here has since been removed; see README and DIAGNOSTICS.md for the current compiler.

# Iteration one review

## Scope and baseline

Work starts from f6534ea. The existing master commit 072a771 (`WIP Global variables + state`)
remains untouched and available in Git history. No reset, rebase or force-push is required.

The implementation keeps the grammar's overall design and the expression/statement AST.
The central changes are callable compilation stages, a consistent semantic analyzer, safe lowering
to the existing linear instruction set, explicit backend support checks, and a command-line boundary.
Rustfmt formatting is applied to the compiled Rust modules; some frontend diff is formatting only.

## Pipeline

`source -> parser -> typed AST -> support checks -> legacy linear IR -> AVR text`

The library (`src/lib.rs`) returns results and diagnostics. It performs no file writes, process exits,
assembly or flashing. The CLI owns files and options. `backend/assembler.rs` contains separate explicit
operations for AVRA and AVRDUDE. MCU definitions are embedded at build time.

The analyzer owns a stack of scopes. A declaration is analyzed before entering the scope and cannot
shadow a visible name. Every expression has its type assigned through one exit path, including ternaries.
Default initializers inherit a real source location. Numeric conversions and literal ranges are checked.

## Why the expression DAG was removed

The old structural DAG/cache treated matching expression shapes as reusable values even across mutation,
branches and pin reads. Its removal is the correctness fix, not a new optimization system.
The legacy instruction set remains. Lowering now walks the checked AST directly, allocates deterministic
register/label identifiers, snapshots local reads and retains source evaluation order. Every pin comparison
emits a fresh read. There is no common-subexpression elimination in this iteration.

The unused DAG, old environment and diagnostic-printer modules were removed rather than leaving an
alternative, incorrect pipeline in the source tree. The replacement analyzer and IR builder report
unsupported features before their former panic/todo paths. Internal invariant assertions still exist;
malformed-source regression cases cover scanner/parser recovery.

General typed IR, control-flow analysis, liveness and register allocation remain future work.
The GPIO backend uses one scratch register, and only accepts the validated subset whose condition value
is consumed immediately. It does not pretend that arbitrary legacy virtual registers map to r16.

## WIP disposition

| WIP change | Disposition |
| --- | --- |
| Boolean literal lowering | Preserved in legacy IR loads/stores, tested without claiming AVR variable support |
| Analyze operand of `!` | Implemented with bool / PinState / Pin semantics and operand errors |
| Pass analyzer environment to backend | Not carried forward; future typed program representation will own storage descriptions |
| Incomplete global variable lookup | Replaced by scoped analysis and explicit legacy global loads/stores for inspection; AVR storage stays unsupported |
| Latch example | Retained in examples/planned/latch.he; analysis passes, target support check rejects it |

## Tests and review entry points

- `tests/compiler.rs`: semantic and parser regressions, declared support boundaries, deterministic IR,
  preservation of chained pin writes and repeated reads. A tiny independent evaluator checks values stored
  by the arithmetic inspection subset; it is not an AVR simulator.
- `tests/snapshots/gpio.ir`: reviewed output for the original GPIO example.
- `tests/cli.rs`: execution from another directory, paths containing spaces, quiet output, missing tools,
  errors, and source-overwrite protection. Explicitly ignored tests require AVRA and validate generated HEX
  checksums and the 40-byte baseline. None invokes successful hardware flashing.

Run the README checks before reviewing subsequent changes. AVRA assembly success and the small IR evaluator
do not prove physical behavior, timing or interrupt correctness. Hardware/simulator tests will be added with
those features. No physical flashing was performed during this iteration.

## Next iteration

Update: the typed IR foundation described below is now implemented; see [IR_V1.md](IR_V1.md).
The AVR generator remains unchanged. LLVM evaluation and machine lowering are the next boundary.

Review follow-up: compiler errors now carry `DiagnosticKind` data and enum codes,
and scanner/parser spans use the same renderer as semantic and backend diagnostics.
The CLI offers automatic/forced/disabled color. `tests/diagnostics.rs` and the error
snapshots cover this contract, including Cyrillic source text, CRLF and tab alignment.
Russian translation remains a separate task; current messages are English.

New code has been separated into logical blocks. Compact enum aliases in nested
parser/analyzer patterns are intentional: expanding them often makes those patterns
harder to read. Formatting checks do not replace a manual readability review.

Agree on typed values and basic blocks, explicit local/global storage and hardware effects. Then implement
an executable integer/bool subset and startup/storage/register handling for ATmega328P. Add `!Pin` lowering
with single-read semantics. Only then introduce optimizations over verified representations.

The later acceptance application is a two-channel tank temperature controller with hysteresis, addressed
Dallas sensors, an LCD, encoder input, indicator outputs and UART. It will require explicit sensor-failure
behavior, nonblocking scheduling and library interfaces, without requiring a heap or C interoperability first.
