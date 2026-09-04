// This file is EXPECTED to fail to compile — reassigning a const is a
// compile-time error, not a runtime one, so it can't be wrapped in
// try/catch and checked from inside the script. `k test` knows to expect
// failure here because of the `_shouldfail_` in the filename.

const pi = 3.14159;
pi = 3;
