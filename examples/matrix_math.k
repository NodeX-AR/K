// Matrix math is built-in syntax ('@'), not a library import.

let inputs = [[1.5, 0.2]];
let weights = [[0.8, -0.1], [0.4, 0.9]];

let hidden = inputs @ weights;
print("hidden:", hidden);
print("activated:", relu(hidden));
print("transposed weights:", transpose(weights));
print("softmax:", softmax([2.0, 1.0, 0.1]));
