// Covers Phase E: a real Tensor value type, elementwise broadcasting,
// and the minimal weight save/load format. Plain nested-list matrix math
// (see examples/matrix_math.k) is untouched by any of this — this file
// exercises the new, separate Tensor type specifically.

// --- construction and shape ---
let t = tensor([[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]]);
assert(shape(t)[0] == 2 and shape(t)[1] == 3, "tensor() should infer shape from the nested list");
assert(len(t) == 2, "len(tensor) should return the size of the first dimension");

let z = zeros([2, 3]);
assert(shape(z)[0] == 2 and shape(z)[1] == 3, "zeros() should build the requested shape");
assert(to_list(z)[0][0] == 0.0, "zeros() should fill with 0.0");

let o = ones([3]);
assert(to_list(o) == [1.0, 1.0, 1.0], "ones() should fill with 1.0");

// --- indexing ---
assert(t[0][0] == 1.0, "tensor indexing should return the right scalar");
assert(t[1][2] == 6.0, "tensor indexing should return the right scalar (row 1)");
assert(to_list(t[0]) == [1.0, 2.0, 3.0], "indexing a 2-D tensor with one index should return a 1-D sub-tensor");

// --- elementwise ops + broadcasting ---
let a = tensor([[1.0, 2.0], [3.0, 4.0]]);
let b = tensor([[10.0, 20.0], [30.0, 40.0]]);
let sum = a + b;
assert(to_list(sum) == [[11.0, 22.0], [33.0, 44.0]], "tensor + tensor should add elementwise");

let scaled = a * 2.0;
assert(to_list(scaled) == [[2.0, 4.0], [6.0, 8.0]], "tensor * scalar should broadcast the scalar");

// broadcasting a row vector against a matrix
let row = tensor([10.0, 20.0]);
let broadcasted = a + row;
assert(to_list(broadcasted) == [[11.0, 22.0], [13.0, 24.0]], "a (2,2) tensor plus a (2,) tensor should broadcast row-wise");

// --- matmul ---
let m1 = tensor([[1.0, 2.0], [3.0, 4.0]]);
let m2 = tensor([[5.0, 6.0], [7.0, 8.0]]);
let product = m1 @ m2;
assert(to_list(product) == [[19.0, 22.0], [43.0, 50.0]], "tensor @ tensor should do real matrix multiplication");

// mixing a plain nested list with a tensor should also work (auto-coerced)
let mixed = m1 @ [[5.0, 6.0], [7.0, 8.0]];
assert(to_list(mixed) == [[19.0, 22.0], [43.0, 50.0]], "tensor @ plain-list should auto-coerce the list side");

// --- elementwise math builtins on tensors ---
let negs = tensor([-1.0, 2.0, -3.0]);
assert(to_list(relu(negs)) == [0.0, 2.0, 0.0], "relu() should work elementwise on a tensor");

// --- reshape ---
let flat = tensor([1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
let reshaped = reshape(flat, [2, 3]);
assert(to_list(reshaped) == [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]], "reshape() should reinterpret the same data with a new shape");

// --- tensor equality is exact elementwise, not a runtime error ---
assert(tensor([1.0, 2.0]) == tensor([1.0, 2.0]), "two tensors with identical shape+data should be ==");
assert(!(tensor([1.0, 2.0]) == tensor([1.0, 3.0])), "two tensors with different data should not be ==");

// --- weight save/load round-trip ---
let path = "k_tensor_test_scratch.json";
save_weights(a, path);
let loaded = load_weights(path);
assert(to_list(loaded) == to_list(a), "load_weights(save_weights(t)) should round-trip exactly");
remove_file(path);

// --- existing plain-list matrix math is completely unaffected ---
let listMatmul = [[1, 2], [3, 4]] @ [[5, 6], [7, 8]];
assert(listMatmul == [[19, 22], [43, 50]], "plain nested-list '@' should still work exactly as before, returning a list");

print("tensor_test: all assertions passed");
