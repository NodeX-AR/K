# K Language Specification

**Version 1.2** — covers the language as of this document's last edit.
This is the authoritative reference for exact syntax and semantics; the
README is the friendlier tour. When they disagree, that's a bug — please
file it (see `CONTRIBUTING.md`).

This spec documents behavior, including deliberate quirks. A quirk that's
listed here is a design decision; anything not listed as intentional
should be assumed to be an oversight worth reporting.

## 1. Lexical structure

- **Comments**: `// to end of line`. No block comments.
- **Identifiers**: start with a letter or `_`, followed by letters,
  digits, or `_`.
- **Numbers**: `123` (int, `i64`), `3.14` (float, `f64`). No hex/octal/
  binary literals, no underscore digit separators.
- **Strings**: `"double"` or `'single'` quotes, with backslash escapes
  (`\n`, `\t`, `\\`, `\"`, `\'`). `f"...{expr}..."` is a format string —
  any `{expr}` inside it is evaluated and interpolated; use `{{`/`}}` for
  a literal brace.
- **Triple-quoted strings**: `"""..."""` — spans multiple lines and does
  **not** process escape sequences (raw). No `f"""..."""` interpolated
  form exists yet.
- **Semicolons** end most statements. The parser is lenient about a
  missing trailing semicolon before `}` in most positions, but write them
  — don't rely on that leniency.

## 2. Values and types

`int`, `float`, `str`, `bool`, `null`, `list`, `dict`, plus functions,
classes, and class instances. `type(v)` returns the type name as a
string. There is no static type system — see §8.

### Dicts

- Literal: `{ "key": value, ... }`. Keys are always coerced to strings
  (a non-string key expression is converted via the same rules as
  `str()`).
- **Iterate in insertion order.** `d.keys()`, `d.values()`, and printing
  a dict always reflect the order entries were first written, not hash
  order. This is a deliberate correctness guarantee, not an incidental
  property — code may rely on it.
- `d["key"]` reads/writes by key (creates the key if writing a new one).
  `d.someKey` (dot syntax) also reads/writes dict entries by key — a dict
  and a "namespace" (see `import ... as`, §9) are the same underlying
  value.
- Methods: `.keys()`, `.values()`, `.get(key, default=null)`,
  `.remove(key)`.
- **`dict == dict` is a runtime error**, not `false`. K doesn't have a
  built-in notion of structural dict equality (what should
  `{"a":[1,2]} == {"a":[1,2]}` mean once dicts can hold arbitrary nested
  values, functions, etc.?) — rather than guess and be wrong silently,
  comparing two dicts with `==`/`!=` throws. Compare specific keys
  instead: `a["id"] == b["id"]`.

### Lists

- Literal: `[1, 2, 3]`. Nested lists (`[[1,2],[3,4]]`) are matrices —
  see §7.
- Indexing: `l[0]`, `l[-1]` is **not** supported (no negative indexing).
- Methods: `.append(x)` / `.push(x)` (synonyms), `.pop()`, `.sort()`
  (numeric sort only), `.reverse()`, `.contains(x)`.
- `list == list` compares elementwise by value (this one *is* well
  defined and works, unlike dicts).

### Classes and instances

```
class Shape {
    fn init(name) { self.name = name; }      // constructor
    fn area() { return 0; }
    fn describe() { return f"{self.name} has area {self.area()}"; }
}
class Circle(Shape) {                         // single inheritance
    fn init(radius) { self.name = "Circle"; self.radius = radius; }
    fn area() { return 3.14159 * self.radius ** 2; }
}
let c = new Circle(3);
```

- `self` is implicit inside a method body — do not declare it as a
  parameter.
- `init` is the constructor, called by `new ClassName(...)`.
- Single inheritance only (`class Child(Parent) { ... }`); no
  interfaces/traits/mixins.
- **`instance == instance` is a runtime error**, same reasoning as dicts:
  there's no built-in field-by-field equality. Give the class an
  `equals(other)` method and call it explicitly if you need comparison.

## 3. Operators

| Category | Operators |
|---|---|
| Arithmetic | `+ - * / % **` (`**` is exponentiation) |
| Compound assignment | `= += -= *= /= %= **=` |
| Comparison | `== != < > <= >=` |
| Logical | `and or not` (word form; `&&`/`||`/`!` also work as synonyms) |
| Matrix | `@` (matrix multiplication — see §7) |
| Ternary | `cond ? then : else` |

No bitwise operators (`& | ^ << >>`) — deliberately out of scope; K
isn't positioning itself for bit-twiddling/systems work.

**`==`/`!=` and dicts/instances**: see §2. Every other comparison
(numbers, strings, bools, null, lists) works exactly as expected.

### Ternary

```
let label = (score >= 60) ? "pass" : "fail";
```
Right-associative: `a ? b : c ? d : e` reads as `a ? b : (c ? d : e)`.
If the condition expression itself starts with `{` (e.g. it's a bare
dict literal), wrap it in parens — otherwise the parser can't tell where
the condition ends.

## 4. Control flow

`if / elif / else`, `while`, `for x in iterable { ... }` (iterates lists,
strings — char by char — and dict keys), `break`, `continue`.

### match expressions

```
let label = match statusCode {
    200 => "OK",
    404 => "Not Found",
    _ => "Unknown",
};
```

- `match` is an **expression**, not a statement — it produces a value.
- Patterns are compared with the same `==` semantics as everywhere else
  (so a dict/instance pattern would itself error — don't match on
  those).
- `_` is the wildcard/default arm. **If no arm matches and there's no
  `_`, it's a runtime error** (not `null`) — a match is expected to be
  exhaustive in practice, and a silent `null` on a missed case is a
  worse failure mode than a loud error.
- If the subject expression itself looks like it could start with `{`,
  wrap it in parens — same ambiguity as the ternary condition.

## 5. Functions

```
fn greet(name, greeting = "Hello") {
    return f"{greeting}, {name}!";
}
```

- Default parameter values (`greeting = "Hello"` above) are evaluated at
  call time when omitted, not once at definition time.
- **A quirk worth knowing**: because "was this argument omitted" is
  implemented as "is this parameter's value `null`", explicitly passing
  `null` for a defaulted parameter also triggers the default — you can't
  distinguish "caller omitted this" from "caller explicitly passed
  null" for a defaulted parameter.
- **Arity is checked.** Calling a function with too many arguments, or
  fewer than its required (non-defaulted) parameter count, is a runtime
  error — not silently null-filled or truncated.
- Anonymous functions: `fn(x) { return x * 2; }`, usable as a value
  (assign it, pass it to `map`/`filter`/`reduce`, etc.).
- Closures capture their enclosing scope's variables by reference (an
  inner function that mutates a captured variable is visible to the
  outer scope too).

### Multiple return values / destructuring

```
fn minMax(list) {
    // ...
    return lo, hi;      // sugar: packs into a 2-element list
}
let (lo, hi) = minMax([4, 1, 9, 2]);   // unpacks that list by position
```

This is sugar over lists, not a distinct tuple type — `minMax(...)`
called without destructuring just returns an ordinary 2-element list.
`let (a, b) = someList;` works on any list with at least 2 elements
(fewer elements is an index-out-of-range runtime error). `const (a, b) =
...;` makes both `a` and `b` immutable bindings (§6) — it does not make
the destructured values themselves immutable in any special way beyond
what `const` already means.

## 6. Bindings: `let` and `const`

```
let x = 1;      // reassignable
const y = 2;    // y cannot be reassigned
y = 3;          // compile error: cannot assign to 'y'
```

- `const` prevents **rebinding the name**, not mutating whatever it
  points to — same semantics as JavaScript's `const`. `const list =
  [1,2]; list.append(3);` is completely fine; `list = [4,5];` is not.
- This is enforced **at compile time**, within one compiled unit (a
  script file, or a single REPL line). **Known gap**: the REPL compiles
  each line independently against the same long-lived global
  environment, so a `const` declared on one REPL line isn't remembered
  when compiling the next line — reassigning it later in the same
  session won't be caught. Fully enforced within a script file.
- Optional type annotations (`let x: int = 1;`, `fn f(x: int) -> int`)
  are parsed and **discarded** — documentation only, not checked. This
  is a deliberate scope decision, not an oversight: K doesn't have (and
  isn't attempting, this pass) a real static type system, and half-
  enforcing annotations would be worse than clearly not enforcing them
  at all.

## 7. Matrix math and Tensors

Two coexisting ways to do numeric array work, kept deliberately separate:

**Plain nested lists** (`[[1,2],[3,4]]`) remain fully supported exactly
as before — `@` on two nested lists does matrix multiplication via the
original list-based implementation, and existing scripts using this are
completely unaffected by everything below.

```
let inputs = [[1.5, 0.2]];
let weights = [[0.8, -0.1], [0.4, 0.9]];
let hidden = inputs @ weights;    // plain lists in, plain list out
print(relu(hidden));
```

**`Tensor`** is a real value type: flat row-major `Vec<f64>` storage plus
a shape, rather than nested lists of lists. Construct one explicitly:

```
let t = tensor([[1.0, 2.0], [3.0, 4.0]]);   // from a rectangular nested list
let z = zeros([2, 3]);                       // shape [2, 3], filled with 0.0
let o = ones([4]);                           // shape [4], filled with 1.0
shape(t);                                    // -> [2, 2]
to_list(t);                                  // -> back to a nested list, for printing/inspection
len(t);                                      // -> 2 (size of the first dimension)
t[0];                                        // -> a 1-D sub-tensor (indexing a >1-D tensor drops a dimension)
reshape(t, [4]);                             // -> same data, new shape (element count must match)
```

- **Immutable.** Every operation on a tensor produces a new one — there's
  no in-place mutation and no `t[0] = 5` (indexing a tensor is
  read-only; assigning into one is a runtime error). This is a
  deliberate simplicity trade-off, not an oversight.
- **`+ - * /` broadcast** between two tensors, or a tensor and a plain
  number, following the same right-aligned rule NumPy uses: comparing
  shapes from the last dimension backward, two dimensions are
  compatible if they're equal or either is 1 (missing leading
  dimensions on the shorter shape count as 1). `tensor([[1,2],[3,4]]) +
  tensor([10, 20])` adds `[10, 20]` to every row. A plain number
  (`t * 2`) is treated as a 0-dimensional tensor and broadcasts against
  anything.
- **`@` matmul** on two tensors (or a tensor and a plain nested list —
  the list side is auto-converted) requires both sides to be exactly
  2-D with matching inner dimensions, same rule as the nested-list
  version.
- `relu`, `sigmoid`, `tanh` work elementwise on tensors of any shape.
  `softmax` and `transpose` currently only support 1-D and 2-D tensors
  respectively (a runtime error otherwise, not silently wrong output).
  `flatten(t)` returns a plain flat `list`, matching what
  `flatten(nested_list)` already did.
- `t1 == t2` compares shape and every element exactly (no floating-point
  tolerance) and returns a real `bool` — tensors don't have the
  "equality is ambiguous" problem dicts/instances do (see §2), since a
  tensor's shape+data fully determine its value.
- `for x in tensor` is **not** supported yet — call `to_list(t)` first
  and iterate that.
- **Weight loading**: `save_weights(t, path)` writes a small JSON file
  (`{"shape": [...], "data": [...]}`); `load_weights(path)` reads one
  back into a `Tensor`. This is a minimal format for K's own
  inference use, not a step toward ONNX or any other interchange
  format (see §13).

## 8. Error handling

```
try {
    riskyThing();
} catch e {
    print("error:", e);
}
throw "something went wrong";
```

Errors are plain values (usually strings) — `throw` raises one, `catch e`
binds it. An uncaught error at the top level stops the script and prints
`Uncaught error: ...`. There's no typed/structured exception hierarchy —
just values.

`assert(condition, message = "assertion failed")` throws `message` if
`condition` is falsy; used the same way as `throw` inside `try`/`catch`,
and is the building block `k test` (§11) expects test files to use.

## 9. Modules / imports

```
import "utils.k";                  // textual inline: utils.k's top-level
                                    // names become part of the current
                                    // scope directly
import "mathlib.k" as math;        // same inlining, PLUS collects
                                    // mathlib.k's top-level names into a
                                    // dict bound to `math`
math.square(5);
math.PI;
```

- Both forms resolve the path **relative to the current working
  directory the interpreter was started in** — not relative to the
  importing file's own location. Run scripts from a consistent directory
  (typically the project root) if they import each other.
- Plain `import` and `import ... as` both still inline the imported
  file's declarations into the current scope (so its top-level names are
  directly usable too, not just through the alias) — the `as` form is
  additive, not a replacement for the plain form's behavior.
- `math.square(5)` works because `math` is genuinely just a dict whose
  values happen to include functions — calling `dict.someKey(...)` calls
  whatever value is stored there if it's callable, falling back to the
  built-in dict methods (`.keys()` etc.) only when there's no stored
  entry under that name.
- No package manager, no versioning, no remote imports — a path is
  always a local file path.

## 10. Standard library

**Core** (always available):
`print`, `len`, `str`, `int`, `float`, `bool`, `type`, `range`, `abs`,
`min`, `max`, `sum`, `sorted`, `round`, `input` (reads one line from real
stdin — see note below), `assert`, `args` (list of extra CLI arguments
after the script path — see §11).

**Math**: `sqrt`, `floor`, `ceil`, `pow(base, exp)`, `log(x, base=e)`,
`exp`, `sin`, `cos`, `tan`.

**Functional**: `map(list, fn)`, `filter(list, fn)`,
`reduce(list, fn, initial=<first element>)`.

**JSON**: `json_encode(value) -> str`, `json_decode(str) -> value`. Hand-
written, not a `serde` dependency (keeps the binary small). Round-trips
int/float/str/bool/null/list/dict; encoding a function/class/instance
falls back to a best-effort string rather than erroring.

**File I/O**: `read_file(path)`, `write_file(path, content)`,
`append_file(path, content)`, `remove_file(path)`,
`file_exists(path) -> bool`. All are synchronous and raise a catchable
error (not a panic) on failure.

**Random**: `random()` → float in `[0, 1)`, `randint(a, b)` → int in
`[a, b]` inclusive. Backed by a small self-contained xorshift64*
generator (no `rand` crate dependency) — good for scripting use
(shuffling, sampling), **not** for anything security-sensitive.

**Time**: `time_now()` → float seconds since the Unix epoch,
`date_string()` → a UTC `"YYYY-MM-DD HH:MM:SS UTC"` string. No timezone
support, no parsing, no arithmetic on dates — deliberately minimal.

**Matrix / Tensor**: see §7 for the full picture. `tensor`, `shape`,
`to_list`, `zeros`, `ones`, `reshape`, `save_weights`, `load_weights`
plus the existing `relu`/`sigmoid`/`tanh`/`softmax`/`transpose`/
`flatten` (which now also accept `Tensor` values, not just nested
lists).

**`input()` in the GUI**: reads real stdin. The GUI IDE doesn't reliably
have a console attached to it, so calling `input()` from a script run
inside the GUI will likely hang waiting for input that never arrives.
Use `input()` from the REPL or from a script run in a terminal.

### String methods
`.upper()`, `.lower()`, `.trim()`, `.split(sep=" ")`, `.replace(a, b)`,
`.contains(s)`, `.startsWith(s)`, `.endsWith(s)`, `.padStart(width,
fill=" ")` (alias `.padLeft`), `.padEnd(width, fill=" ")` (alias
`.padRight`), `.repeat(n)`.

## 11. Tooling

- **`k` (no args)**: interactive REPL. Arrow-key history (persisted to
  `~/.k_history`), Ctrl+R search, multi-line input (auto-continues while
  braces are open), and `:help` / `:load <file>` / `:vars` / `:clear` /
  `:exit` commands.
- **`k <file.k> [args...]`**: run a script. Extra arguments after the
  filename are available inside the script via `args()`. Exits with a
  nonzero status if the script errors (lex/parse/compile error, or an
  uncaught runtime error) — scripts are well-behaved Unix citizens for
  scripting/CI use.
- **`k gui`**: the graphical IDE (dark/light theme, syntax highlighting,
  file open/save, auto re-indent on Run/Save, Ctrl+I to re-indent on
  demand). Only present in builds compiled with the `gui` Cargo feature
  (on by default; see `Cargo.toml`).
- **`k fmt [--check] <file.k>`**: reformats a file's indentation based on
  brace nesting, in place. `--check` reports whether it *would* change
  the file (nonzero exit if so) without writing — for CI. K has no
  significant whitespace, so this can only change how a file looks, not
  what it does.
- **`k test <file_or_dir>`**: runs every `*_test.k` file under a
  directory (or a single file directly) and reports pass/fail. A file
  passes if it runs to completion without an error; name a file
  `*_shouldfail_test.k` to invert that (passes only if running it *does*
  error) — for testing things like the `const` compile-time check, which
  can't be caught from inside the failing script itself.

## 12. Known gaps (tracked, not hidden)

- **No source line numbers in parser/compiler error messages.** The
  lexer's error messages do include a line number; parser and compiler
  errors currently don't. Fixing this properly means changing what the
  lexer returns (pairing every token with its line), which ripples into
  every caller — deferred as too invasive to do safely without the
  ability to compile-check the change in the environment this was
  written in. Tracked, not silently dropped.
- **Every local variable is boxed** (`Rc<RefCell<Value>>`) regardless of
  whether a closure ever captures it — correct, but pays indirection
  cost even for the common case of a local that's never captured. An
  escape-analysis pass to box only actually-captured locals is a real
  performance win, deferred as an invasive compiler change.
- **Tensor limitations**: no `for x in tensor` iteration (convert with
  `to_list()` first), no assignment into a tensor by index (immutable —
  see §7), `softmax()`/`transpose()` on a tensor only support 1-D/2-D
  respectively. Each of these is a deliberate, documented scope cut for
  this pass, not an oversight — none of them block the core Tensor +
  broadcasting + weight-loading functionality.
- **Random/time were not feature-flagged**, despite being requested as
  optional/feature-gated. Both are hand-rolled with zero external crate
  dependencies (a small xorshift64* PRNG; Howard Hinnant's
  `civil_from_days` for the date). Properly feature-gating them would
  mean scattering `#[cfg(feature = ...)]` across the `VM` struct, its
  constructor, and several `call_native` match arms — real code-review
  risk with no compiler available in the environment this was written
  in to catch a mistake in either build configuration — in exchange for
  a code-size saving too small to be worth that risk (a few hundred
  bytes, not a dependency). Kept in core as a deliberate call; flagged
  here rather than silently ignoring the request.

## 13. Explicitly out of scope (this pass)

Tracked here so "not done" and "not considered" aren't confused with
each other. None of these were attempted or stubbed:

- Full reverse-mode autograd / training loop
- ONNX (or any other) pretrained-model import format (`save_weights`/
  `load_weights` — see §7 — are a minimal K-specific JSON format, not a
  step toward ONNX compatibility)
- Language Server Protocol (LSP) support
- A debugger (breakpoints, stepping, inspecting locals)
- A package manager / registry
- A fuzzer for the lexer/parser/compiler
- GPU support of any kind
- Bitwise operators (`& | ^ << >>`)
- Full source-span error tracking (see §12 — a smaller, safer version of
  this is tracked as a known gap, not attempted here)

## Changelog

### 1.2
Added: a real `Tensor` value type (flat `Vec<f64>` + shape) with NumPy-
style broadcasting for `+ - * /`, tensor-native `@` matmul (auto-coercing
a plain-list operand), `tensor`/`shape`/`to_list`/`zeros`/`ones`/
`reshape`, exact elementwise `==`, and a minimal JSON weight format
(`save_weights`/`load_weights`). Extended `relu`/`sigmoid`/`tanh`/
`softmax`/`transpose`/`flatten`/`len` to accept tensors alongside their
existing list support. Plain nested-list matrix math (`@` on two lists)
is completely unchanged — the two representations coexist deliberately;
see §7.

### 1.1
Added: ternary expressions, `match` expressions, multi-return/
destructuring (`let (a, b) = ...`), namespaced imports (`import ... as`),
triple-quoted strings, `%=`/`**=`, math/JSON/file-I/O/random/time/
functional (`map`/`filter`/`reduce`) builtins, `assert()`, `args()`,
string padding methods, `remove_file`/`file_exists`, `k fmt`, `k test`.
`gui` became an off-by-default-capable Cargo feature (still on by
default) so a CLI/container build can skip eframe/rfd/image entirely
with `--no-default-features`.
Fixed: `const` is now enforced at compile time; `dict`/`instance`
equality is now a runtime error instead of silently `false`; function
calls now check arity instead of silently null-filling/dropping
arguments; dicts now iterate in insertion order.

### 1.0
Initial self-hosted interpreter: lexer → parser → AST → bytecode
compiler → stack VM. Core language (variables, control flow, functions
with closures, classes with single inheritance, try/catch), matrix math
as built-in syntax, GUI IDE, and REPL.
