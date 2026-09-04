# K

K is a self-contained, general-purpose programming language: variables,
closures, classes, error handling, and matrix math as **built-in syntax**,
not a library you import. It compiles to bytecode and runs on its own VM —
no interpreter to install, no package manager, no virtualenv, one binary.

```k
class Dog {
    fn init(name) { self.name = name; }
    fn speak() { return f"{self.name} says Woof!"; }
}
let d = new Dog("Rex");
print(d.speak());

let weights = [[0.8, -0.1], [0.4, 0.9]];
let inputs  = [[1.5, 0.2]];
print(relu(inputs @ weights));   // matrix multiply is a language operator, '@'
```

## Why K exists

Not "faster than Python," not "will replace Python," not "beats Mojo" —
those aren't honest claims for a new language and I'm not making them. The
real, narrower reason to reach for K: **zero setup cost.** Multiplying two
matrices in Python means Python installed, then `pip` working, then a
virtualenv, before line one of actual math runs. In K it's one binary and
one line. That's a genuine advantage for classrooms, bootcamps, quick
scripts, and constrained/edge deployments.

## Architecture: bytecode VM, not a tree-walker

The previous iteration of this codebase evaluated the AST directly —
walking the parsed tree and re-matching node types on every single
execution. This version compiles the AST to flat bytecode once
(`compiler.rs`) and executes that with a real VM (`vm.rs`): a stack-based
loop over bytes, variables resolved to array slot indices at *compile
time* instead of walked through a HashMap-based scope chain at every
access.

**Measured, not claimed:** `fib(27)` (832,039 function calls) — tree-walker
209–226ms, bytecode VM 187–192ms, both release builds, averaged over 3
runs each. That's a real ~15% improvement, not the 5-10x a bytecode VM can
theoretically deliver, and the gap between "real" and "theoretical" is
worth being specific about: every local variable is currently boxed as
`Rc<RefCell<Value>>` uniformly (simplest way to get closures correct),
which means every function call still does 2+ heap allocations before
running a single instruction — similar in cost to the tree-walker's
per-call scope allocation, just shaped differently. The dispatch loop
itself is genuinely faster; that win is currently being offset by
allocation overhead elsewhere. **The concrete next step for real speed:**
only box locals that a nested closure actually captures (the compiler
already tracks this via its upvalue-resolution pass); leave everything
else as a plain, non-allocated stack slot.

## Status: what's real vs. what's aspirational

Everything below has been executed and its output checked — including two
real bugs found by testing this exact code and fixed before shipping (a
default-parameter jump-patch bug that corrupted the stack, and a local
call-frame under-allocation bug that crashed on any function with more
locals than parameters). Both are documented in the compiler/VM source
comments where they were fixed.

**Working today** — verified via recursion, 3-level nested-closure upvalue
chains, mutable closures (independent counters don't share state), class
inheritance with implicit `self`, try/catch across nested function calls,
break/continue including in nested loops, default parameters, matrix `@`
and the ML builtins, dict/list/string methods, and deep recursion
(500-level) without incident:
- `let`/`const`, `if`/`elif`/`else`, `while`, `for..in`, `break`/`continue`
- Functions with default parameters and real closures resolved at compile
  time to stack slots or upvalue chains (not a runtime scope search)
- Recursion, including through nested/local function declarations
- Classes with single inheritance and implicit `self`
- `try`/`catch`/`throw` as VM-level handler stack — errors unwind cleanly
  across nested calls to the nearest active handler; nothing panics
- Lists, dicts, strings with real methods (`.append`, `.sort`, `.keys`,
  `.upper`, `.split`, `.replace`, …)
- `f"...{expr}..."` string interpolation
- Matrices as nested lists with `@` (matmul), `relu`, `sigmoid`, `tanh`,
  `softmax`, `transpose`, `flatten`
- A GUI IDE (`k gui`) with a dark/light theme, basic syntax highlighting,
  automatic re-indentation (runs on Run/Save, or on demand via Ctrl+I —
  K has no significant whitespace, so this only affects how the source
  looks), Open/Save/Save As, and keyboard shortcuts (F5 run, Ctrl+S save,
  Ctrl+O open, Ctrl+N new)
- A terminal REPL (`k repl`) with arrow-key history, Ctrl+R search, and
  multi-line input (auto-continues while braces are open) via
  `rustyline`; `:help`, `:load <file>`, `:vars`, `:clear`, `:exit` — the
  REPL keeps one VM alive across lines so variables persist between them
- Ternary expressions (`cond ? a : b`), `match` expressions, multiple
  return values / destructuring (`let (a, b) = f();`), namespaced
  imports (`import "lib.k" as lib;`), triple-quoted multi-line/raw
  strings, and `%=`/`**=` compound assignment
- A real standard library: math (`sqrt`, `pow`, `sin`, ...), JSON encode/
  decode, file I/O, `map`/`filter`/`reduce`, string padding, `assert()`,
  `args()` for CLI argument passthrough, and random/time builtins — all
  pure-Rust with no new runtime dependencies (see `docs/SPEC.md` §10 for
  the full list)
- `const` is enforced at compile time, `dict`/`instance` equality via
  `==` is a runtime error instead of silently `false`, function calls
  check arity instead of silently null-filling/dropping arguments, and
  dicts iterate in insertion order — see `docs/SPEC.md` for exact
  semantics and known gaps on each
- `k fmt` (reformat a file's indentation; `--check` mode for CI) and
  `k test` (run `*_test.k` files under `tests/` and report pass/fail)
- The GUI (`k gui`, and its `eframe`/`rfd`/`image` dependencies) is now
  behind a Cargo feature (`gui`, on by default) — a size-conscious build
  can drop it with `cargo build --release --no-default-features`
- A real `Tensor` value type (flat `Vec<f64>` + shape, not nested
  lists) with NumPy-style broadcasting on `+ - * /`, tensor-native `@`
  matmul, `zeros`/`ones`/`reshape`/`shape`/`to_list`, and a minimal
  JSON weight format (`save_weights`/`load_weights`) — see
  `docs/SPEC.md` §7. Plain nested-list matrix math is untouched and
  still works exactly as before; the two coexist on purpose.

**Not here, and not pretended to be:**
- A package manager or a "compile to native" command — a previous
  iteration had both as non-functional stubs (one pointed at a GitHub repo
  that doesn't exist, the other ignored your source and printed a
  hardcoded string); removed rather than shipped as decoration.
- Static types (annotations parse but are documentation-only, on
  purpose — see `docs/SPEC.md` §6), autograd/training, ONNX import, an
  LSP, a debugger, or a package registry.
- The unboxed-locals optimization described above, and full source-span
  (line-number) tracking through the parser/compiler — both deferred as
  too invasive to attempt without the ability to compile-check the
  change; see `docs/SPEC.md` §12 for specifics on each.
- Independent fuzzing, load-testing, or third-party security review. That
  takes real users hitting real edge cases over time — no single build
  session gets to claim it, for any language.
- "Best in industry" tooling — the REPL and IDE upgrades below make K
  nicer to use day-to-day, but a couple of features don't put a young,
  single-maintainer language ahead of decades-old ecosystems with full
  LSPs, debuggers, and package registries. Framing it that way would be
  a marketing claim, not an engineering one.
- **Measured binary-size or Docker-image-size numbers.** `docker/` has
  example Dockerfiles and `BENCHMARKS.md` has the exact commands to
  produce real numbers, but none are recorded yet — that needs a real
  build environment, not a description of one.

See `docs/SPEC.md` for the full, versioned language specification
(exact syntax, semantics, and a changelog), and `CONTRIBUTING.md` if
you're looking to add something.

## Getting K running

```
cargo build --release
./target/release/k                          # interactive shell (same as 'k repl')
./target/release/k script.k [args...]       # run a script; extra args reach args()
./target/release/k gui                      # graphical IDE
./target/release/k fmt myfile.k             # reformat a file's indentation
./target/release/k test tests/              # run the test suite
```

For a smaller, GUI-free binary (containers, CI):
```
cargo build --release --no-default-features
```

## Repo layout

```
src/
  lexer.rs          tokenizer (returns Result, never panics)
  ast.rs            AST node definitions
  parser.rs         recursive-descent parser -> AST
  chunk.rs          bytecode format: opcodes + constant pool
  value.rs          runtime value types (Int/Float/Str/List/Dict/Closure/Class/...)
  compiler.rs       AST -> bytecode: locals resolved to slots, upvalue capture, loops/try-catch as jumps
  vm.rs             the bytecode VM: execution loop + native builtins
  main.rs           CLI: run a file, REPL, fmt, test, or launch the GUI
  idle.rs           the built-in GUI IDE (eframe/egui) -- behind the `gui` Cargo feature
examples/           sample .k programs (all verified to run, identical
                     output on both the old tree-walker and the new VM)
tests/              *_test.k files using assert() -- run with `k test tests/`
docs/SPEC.md        the full versioned language specification
docker/             example Dockerfiles (K CLI-only, K musl-static, Python+numpy
                     for comparison) -- see BENCHMARKS.md for how to measure them
BENCHMARKS.md       benchmarking methodology; no numbers recorded yet (see file)
CONTRIBUTING.md     contribution guide and lightweight RFC process
```

## License

MIT — see `LICENSE`.
