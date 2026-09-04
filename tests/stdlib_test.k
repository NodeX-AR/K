// Covers the new standard library builtins.

// --- math ---
assert(sqrt(16.0) == 4.0, "sqrt");
assert(floor(3.7) == 3.0, "floor");
assert(ceil(3.2) == 4.0, "ceil");
assert(pow(2.0, 10.0) == 1024.0, "pow");
assert(abs(exp(0.0) - 1.0) < 0.0001, "exp(0) is 1");
assert(abs(sin(0.0)) < 0.0001, "sin(0) is 0");
assert(abs(cos(0.0) - 1.0) < 0.0001, "cos(0) is 1");

// --- map / filter / reduce ---
let nums = [1, 2, 3, 4, 5];
let doubled = map(nums, fn(x) { return x * 2; });
assert(doubled[0] == 2 and doubled[4] == 10, "map should double every element");

let evens = filter(nums, fn(x) { return x % 2 == 0; });
assert(len(evens) == 2 and evens[0] == 2 and evens[1] == 4, "filter should keep only even numbers");

let total = reduce(nums, fn(acc, x) { return acc + x; }, 0);
assert(total == 15, "reduce should sum 1..5 to 15");

// --- JSON ---
let data = { "name": "K", "version": 1, "tags": ["fast", "small"] };
let encoded = json_encode(data);
let decoded = json_decode(encoded);
assert(decoded["name"] == "K", "json round-trip preserves string values");
assert(decoded["version"] == 1, "json round-trip preserves int values");
assert(decoded["tags"][0] == "fast", "json round-trip preserves nested lists");

// --- string padding ---
assert("7".padStart(3, "0") == "007", "padStart pads with the given character");
assert("7".padEnd(3, "-") == "7--", "padEnd pads on the right");
assert("ab".repeat(3) == "ababab", "repeat");

// --- file I/O (round-trip through a scratch file, cleaned up after) ---
let scratch = "k_stdlib_test_scratch.txt";
write_file(scratch, "first line\n");
append_file(scratch, "second line\n");
let contents = read_file(scratch);
assert(contents.contains("first line"), "write_file wrote the initial content");
assert(contents.contains("second line"), "append_file appended without overwriting");
assert(file_exists(scratch), "file_exists should see the scratch file");
remove_file(scratch);
assert(!file_exists(scratch), "remove_file should delete the scratch file");

// --- assert() itself, with a custom message, inside try/catch ---
let assertErrored = false;
try {
    assert(false, "this should be caught");
} catch e {
    assertErrored = true;
}
assert(assertErrored, "assert(false, ...) should raise a catchable error");

// --- random / time: can't assert exact values, just that they behave sanely ---
let r = random();
assert(r >= 0.0 and r < 1.0, "random() should be in [0, 1)");
let ri = randint(5, 5);
assert(ri == 5, "randint(n, n) should always return n");
assert(time_now() > 0.0, "time_now() should return a positive epoch time");
assert(len(date_string()) > 0, "date_string() should return something non-empty");

print("stdlib_test: all assertions passed");
