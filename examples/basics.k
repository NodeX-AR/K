// K basics: variables, control flow, functions, string interpolation.

let name = "K";
const version = 1;
print(f"Hello from {name} v{version}!");

fn greet(who, greeting = "Hello") {
    return f"{greeting}, {who}!";
}
print(greet("world"));
print(greet("K", "Welcome"));

let total = 0;
for n in range(1, 6) {
    total += n;
}
print(f"1..5 sums to {total}");

let i = 0;
while i < 3 {
    print("while loop:", i);
    i += 1;
}
