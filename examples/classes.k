// Classes with inheritance. 'self' is implicit inside a method --
// do not declare it as a parameter.

class Shape {
    fn init(name) { self.name = name; }
    fn area() { return 0; }
    fn describe() { return f"{self.name} has area {self.area()}"; }
}

class Circle(Shape) {
    fn init(radius) {
        self.name = "Circle";
        self.radius = radius;
    }
    fn area() { return 3.14159 * self.radius ** 2; }
}

let c = new Circle(3);
print(c.describe());
