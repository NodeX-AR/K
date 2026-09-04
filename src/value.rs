//! Runtime value types for the bytecode VM. Deliberately similar to the
//! tree-walker's `Value` (same semantics, same builtins) so behavior is
//! consistent between the two engines while execution itself is now a
//! flat bytecode loop instead of AST recursion.

use crate::chunk::Chunk;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

pub type Rid<T> = Rc<RefCell<T>>;
/// A captured variable cell. Regular locals live directly on the VM stack;
/// only locals actually captured by a nested closure get boxed into one of
/// these, so the common (non-closure) case pays no extra indirection.
pub type Cell = Rc<RefCell<Value>>;

/// A dict's backing store. A plain `HashMap` iterates in an arbitrary,
/// insertion-independent order, so printing a dict or calling `.keys()`
/// twice in a row could show entries in a different order each time (and
/// definitely not the order they were written in) — surprising for a
/// beginner-facing language. This keeps a `HashMap` for O(1) lookup by key,
/// plus a parallel `Vec` recording insertion order, so iteration is always
/// stable and matches how the dict was written. `remove` is O(n) because of
/// the vec shift, which is a fine trade for how small most K dicts are.
#[derive(Default)]
pub struct OrderedMap {
    index: HashMap<String, usize>,
    entries: Vec<(String, Value)>,
}

impl OrderedMap {
    pub fn new() -> Self { Self::default() }

    pub fn insert(&mut self, key: String, value: Value) {
        if let Some(&i) = self.index.get(&key) {
            self.entries[i].1 = value;
        } else {
            self.index.insert(key.clone(), self.entries.len());
            self.entries.push((key, value));
        }
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.index.get(key).map(|&i| &self.entries[i].1)
    }

    pub fn contains_key(&self, key: &str) -> bool { self.index.contains_key(key) }

    pub fn remove(&mut self, key: &str) -> Option<Value> {
        let i = self.index.remove(key)?;
        let (_, v) = self.entries.remove(i);
        for idx in self.index.values_mut() {
            if *idx > i { *idx -= 1; }
        }
        Some(v)
    }

    pub fn len(&self) -> usize { self.entries.len() }
    pub fn is_empty(&self) -> bool { self.entries.is_empty() }
    pub fn keys(&self) -> impl Iterator<Item = &String> { self.entries.iter().map(|(k, _)| k) }
    pub fn values(&self) -> impl Iterator<Item = &Value> { self.entries.iter().map(|(_, v)| v) }
    pub fn iter(&self) -> impl Iterator<Item = (&String, &Value)> { self.entries.iter().map(|(k, v)| (k, v)) }
}

/// A dense numeric array: flat row-major storage plus a shape, replacing
/// the nested-list-of-lists representation for matrix/tensor math. Kept
/// immutable (no `RefCell`) — every op that "changes" a tensor (reshape,
/// elementwise math, matmul) produces a new one. That's a real constraint
/// (no in-place mutation, no tensor indexing assignment yet — see
/// docs/SPEC.md), traded for a much simpler, harder-to-get-wrong
/// implementation than an interior-mutable one would have been.
pub struct TensorObj {
    pub data: Vec<f64>,
    pub shape: Vec<usize>,
}

#[derive(Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    Str(Rc<String>),
    Bool(bool),
    Null,
    List(Rid<Vec<Value>>),
    Dict(Rid<OrderedMap>),
    Tensor(Rc<TensorObj>),
    Closure(Rc<ClosureObj>),
    BoundMethod(Box<Value>, Rc<ClosureObj>),
    Native(&'static str),
    Class(Rc<ClassObj>),
    Instance(Rid<InstanceObj>),
}

pub struct FunctionObj {
    pub name: String,
    /// Total number of declared parameters (including ones with a default
    /// value) — the most a call may pass.
    pub arity: usize,
    /// Number of parameters *without* a default — the fewest a call may
    /// pass. Equal to `arity` for a function with no default parameters.
    pub required_arity: usize,
    /// Total local-variable slots this function's body uses (self/receiver +
    /// parameters + every `let`/for-loop/catch binding declared anywhere in
    /// the body) — not just `arity`. Call frames must allocate this many
    /// slots, or any `let` beyond the parameter list indexes out of bounds.
    pub local_count: usize,
    pub chunk: Chunk,
    pub upvalue_count: usize,
}

pub struct ClosureObj {
    pub function: Rc<FunctionObj>,
    pub upvalues: Vec<Cell>,
}

pub struct ClassObj {
    pub name: String,
    pub parent: Option<Rc<ClassObj>>,
    pub methods: RefCell<HashMap<String, Rc<ClosureObj>>>,
}

pub struct InstanceObj {
    pub class: Rc<ClassObj>,
    pub fields: HashMap<String, Value>,
}

pub fn find_method(class: &Rc<ClassObj>, name: &str) -> Option<Rc<ClosureObj>> {
    if let Some(m) = class.methods.borrow().get(name) { return Some(m.clone()); }
    class.parent.as_ref().and_then(|p| find_method(p, name))
}

pub fn to_display(v: &Value) -> String {
    match v {
        Value::Int(i) => i.to_string(),
        Value::Float(f) => if f.fract() == 0.0 && f.is_finite() { format!("{:.1}", f) } else { f.to_string() },
        Value::Str(s) => (**s).clone(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".to_string(),
        Value::List(items) => format!("[{}]", items.borrow().iter().map(to_repr).collect::<Vec<_>>().join(", ")),
        Value::Dict(d) => format!("{{{}}}", d.borrow().iter().map(|(k, v)| format!("\"{}\": {}", k, to_repr(v))).collect::<Vec<_>>().join(", ")),
        // Deliberately doesn't print the data: a tensor can hold thousands
        // of floats, and dumping them all on every `print()` would be far
        // more noise than signal. Use `to_list(t)` to see the values.
        Value::Tensor(t) => format!("Tensor(shape={:?})", t.shape),
        Value::Closure(c) => format!("<fn {}>", c.function.name),
        Value::BoundMethod(_, c) => format!("<method {}>", c.function.name),
        Value::Native(n) => format!("<builtin {}>", n),
        Value::Class(c) => format!("<class {}>", c.name),
        Value::Instance(i) => format!("<{} instance>", i.borrow().class.name),
    }
}

fn to_repr(v: &Value) -> String { match v { Value::Str(s) => format!("\"{}\"", s), other => to_display(other) } }

pub fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Int(_) => "int", Value::Float(_) => "float", Value::Str(_) => "str", Value::Bool(_) => "bool",
        Value::Null => "null", Value::List(_) => "list", Value::Dict(_) => "dict", Value::Tensor(_) => "tensor",
        Value::Closure(_) => "func", Value::BoundMethod(..) => "method", Value::Native(_) => "builtin",
        Value::Class(_) => "class", Value::Instance(_) => "instance",
    }
}

pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Null => false,
        Value::Int(i) => *i != 0,
        Value::Float(f) => *f != 0.0,
        Value::Str(s) => !s.is_empty(),
        Value::List(l) => !l.borrow().is_empty(),
        Value::Dict(d) => !d.borrow().is_empty(),
        Value::Tensor(t) => !t.data.is_empty(),
        _ => true,
    }
}

pub fn value_eq(a: &Value, b: &Value) -> bool {
    use Value::*;
    match (a, b) {
        (Int(x), Int(y)) => x == y,
        (Float(x), Float(y)) => x == y,
        (Int(x), Float(y)) | (Float(y), Int(x)) => *x as f64 == *y,
        (Str(x), Str(y)) => x == y,
        (Bool(x), Bool(y)) => x == y,
        (Null, Null) => true,
        (List(x), List(y)) => {
            let (xb, yb) = (x.borrow(), y.borrow());
            xb.len() == yb.len() && xb.iter().zip(yb.iter()).all(|(p, q)| value_eq(p, q))
        }
        // Exact elementwise equality. Unlike Dict/Instance (see checked_eq
        // in vm.rs), a tensor's shape+data fully determine its value, so
        // there's no ambiguity about what "equal" should mean here — this
        // does NOT need to go through the same "error instead of guessing"
        // treatment dicts/instances got.
        (Tensor(x), Tensor(y)) => x.shape == y.shape && x.data == y.data,
        _ => false,
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "{}", to_display(self)) }
}
