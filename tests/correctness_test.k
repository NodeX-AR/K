// Covers the "upgrade" fixes: const semantics, dict/instance equality,
// arity checking, and insertion-ordered dicts.

// --- const blocks rebinding the name, not mutating what it holds ---
// (This matches JavaScript's `const` — a deliberate, documented choice.)
const numbers = [1, 2, 3];
numbers.append(4); // mutating the list itself is fine
assert(len(numbers) == 4, "const list can still be mutated in place");

// --- dict == dict is a runtime error, not a silent false ---
let d1 = { "a": 1 };
let d2 = { "a": 1 };
let dictCompareErrored = false;
try {
    let same = d1 == d2;
} catch e {
    dictCompareErrored = true;
}
assert(dictCompareErrored, "comparing two dicts with == should raise an error, not silently return false");

// --- class instance == instance is also a runtime error ---
class Point {
    fn init(x, y) { self.x = x; self.y = y; }
}
let p1 = new Point(1, 2);
let p2 = new Point(1, 2);
let instanceCompareErrored = false;
try {
    let same = p1 == p2;
} catch e {
    instanceCompareErrored = true;
}
assert(instanceCompareErrored, "comparing two instances with == should raise an error, not silently return false");

// --- arity mismatch is a real error now, not silent null-fill/drop ---
fn addTwo(a, b) { return a + b; }
let arityErrored = false;
try {
    addTwo(1, 2, 3, 4); // too many arguments
} catch e {
    arityErrored = true;
}
assert(arityErrored, "calling a function with too many arguments should raise an error");

let tooFewErrored = false;
try {
    addTwo(1); // too few arguments, and 'b' has no default
} catch e {
    tooFewErrored = true;
}
assert(tooFewErrored, "calling a function with too few required arguments should raise an error");

// default parameters still make a call with fewer args legal
fn greet(name, greeting = "Hello") { return f"{greeting}, {name}!"; }
assert(greet("K") == "Hello, K!", "default parameters still work with a short call");

// --- dicts iterate in insertion order, not hash order ---
let ordered = {};
ordered["z"] = 1;
ordered["a"] = 2;
ordered["m"] = 3;
let keys = ordered.keys();
assert(keys[0] == "z" and keys[1] == "a" and keys[2] == "m", "dict keys should come back in insertion order");

print("correctness_test: all assertions passed");
