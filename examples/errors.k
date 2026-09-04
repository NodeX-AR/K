// Errors are catchable values, never a crash.

fn safe_divide(a, b) {
    try {
        return a / b;
    } catch e {
        print("error:", e);
        return null;
    }
}

print(safe_divide(10, 2));
print(safe_divide(10, 0));
print("program kept running");
