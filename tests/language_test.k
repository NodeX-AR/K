// Exercises: ternary, destructuring, match, namespaced-import-shaped
// dicts, multi-line strings, and %= / **=.
// Run with: k test tests/  (or: k test tests/language_test.k)

// --- ternary ---
let x = 5;
assert(((x > 3) ? "big" : "small") == "big", "ternary should pick the true branch");
assert((x > 3 ? "big" : "small") == "big", "ternary works without extra parens around the condition");

// --- multiple return values / destructuring ---
fn minMax(list) {
    let lo = list[0];
    let hi = list[0];
    for v in list {
        if v < lo { lo = v; }
        if v > hi { hi = v; }
    }
    return lo, hi;
}
let (lo, hi) = minMax([4, 1, 9, 2]);
assert(lo == 1, "destructured lo should be 1");
assert(hi == 9, "destructured hi should be 9");

// --- match expression ---
fn describe(n) {
    return match n {
        0 => "zero",
        1 => "one",
        _ => "many",
    };
}
assert(describe(0) == "zero", "match arm 0");
assert(describe(1) == "one", "match arm 1");
assert(describe(42) == "many", "match default");

// --- multi-line / raw string ---
let block = """line one
line two""";
assert(block.contains("line one"), "triple-quoted string keeps its content");
assert(block.contains("\n"), "triple-quoted string preserves the real newline");

// --- compound assignment ---
let n = 10;
n %= 4;
assert(n == 2, "%= should reduce mod 4");
let p = 2;
p **= 5;
assert(p == 32, "**= should raise to the power");

print("language_test: all assertions passed");
