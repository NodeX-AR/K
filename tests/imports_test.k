// Covers `import "file.k" as name;` — run from the repo root so the
// relative path below resolves (imports are resolved against the
// current working directory, same as a plain `import "file.k";`).

import "tests/fixtures/mathlib.k" as mathlib;

assert(mathlib.libName == "mathlib", "namespaced import should expose top-level consts");
assert(mathlib.square(5) == 25, "namespaced import should expose callable functions");
assert(mathlib.cube(3) == 27, "namespaced import works for more than one function");

print("imports_test: all assertions passed");
