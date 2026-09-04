//! The VM: a flat per-frame instruction loop (no AST recursion) plus the
//! native builtin implementations. Nested K function calls recurse through
//! `run_frame` at the Rust level (one Rust stack frame per K call), which
//! is the same approach real interpreters like CPython's `ceval` use —
//! the win here isn't eliminating native recursion, it's eliminating the
//! AST-node dispatch and HashMap-based variable lookups that dominated the
//! tree-walker's per-step cost.

use crate::chunk::OpCode;
use crate::value::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub struct VM {
    pub globals: HashMap<String, Value>,
    pub output: String,
    /// State for the xorshift64* PRNG behind `random()`/`randint()`. No
    /// external `rand` crate — this is a small, well-known, self-contained
    /// generator, good enough for scripting use (shuffling, sampling,
    /// non-cryptographic randomness), not for anything security-sensitive.
    rng_state: std::cell::Cell<u64>,
    /// Extra command-line arguments after the script path, exposed to K
    /// scripts via `args()`. Empty unless the host (main.rs) sets it.
    pub script_args: Vec<String>,
}

const BUILTINS: &[&str] = &[
    "print", "len", "str", "int", "float", "bool", "type", "range",
    "abs", "min", "max", "sum", "sorted", "round", "input", "assert", "args",
    "relu", "sigmoid", "tanh", "softmax", "transpose", "flatten",
    "sqrt", "floor", "ceil", "pow", "log", "exp", "sin", "cos", "tan",
    "map", "filter", "reduce",
    "json_encode", "json_decode",
    "read_file", "write_file", "append_file", "remove_file", "file_exists",
    "random", "randint", "time_now", "date_string",
    "tensor", "shape", "to_list", "zeros", "ones", "reshape", "save_weights", "load_weights",
];

impl VM {
    pub fn new() -> Self {
        let mut globals = HashMap::new();
        for name in BUILTINS { globals.insert(name.to_string(), Value::Native(name)); }
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E3779B97F4A7C15)
            | 1; // xorshift64* must never be seeded with 0
        VM { globals, output: String::new(), rng_state: std::cell::Cell::new(seed), script_args: Vec::new() }
    }

    fn next_random_u64(&self) -> u64 {
        let mut x = self.rng_state.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng_state.set(x);
        x
    }

    pub fn run_program(&mut self, function: Rc<FunctionObj>) -> String {
        let local_count = function.local_count;
        let closure = Rc::new(ClosureObj { function, upvalues: Vec::new() });
        let mut locals: Vec<Cell> = Vec::with_capacity(local_count);
        for _ in 0..local_count { locals.push(Rc::new(RefCell::new(Value::Null))); }
        match self.run_frame(closure, locals) {
            Ok(_) => {}
            Err(v) => self.output.push_str(&format!("Uncaught error: {}\n", to_display(&v))),
        }
        std::mem::take(&mut self.output)
    }

    /// Executes one function's bytecode to completion, returning its result.
    /// A K-level function call (Call/Invoke/Instantiate) recurses into this
    /// via `call_value`; a thrown/propagated error unwinds naturally through
    /// the `?` operator until an enclosing try/catch's handler catches it.
    fn run_frame(&mut self, closure: Rc<ClosureObj>, mut locals: Vec<Cell>) -> Result<Value, Value> {
        let code_ptr: *const Vec<u8> = &closure.function.chunk.code;
        let const_ptr: *const Vec<Value> = &closure.function.chunk.constants;
        // Safety: `closure` (and thus its chunk) stays alive for the whole
        // function; we never mutate the chunk while executing it.
        let code: &Vec<u8> = unsafe { &*code_ptr };
        let constants: &Vec<Value> = unsafe { &*const_ptr };

        let mut stack: Vec<Value> = Vec::new();
        let mut ip: usize = 0;
        let mut handlers: Vec<(usize, usize)> = Vec::new(); // (stack_len_at_push, catch_ip)

        loop {
            let byte = code[ip];
            ip += 1;
            let op = OpCode::from_u8(byte);
            match self.exec_one(op, code, &mut ip, constants, &mut stack, &mut locals, &closure, &mut handlers) {
                Ok(None) => continue,
                Ok(Some(v)) => return Ok(v),
                Err(errval) => {
                    if let Some((slen, catch_ip)) = handlers.pop() {
                        stack.truncate(slen);
                        stack.push(errval);
                        ip = catch_ip;
                        continue;
                    }
                    return Err(errval);
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn exec_one(
        &mut self,
        op: OpCode,
        code: &[u8],
        ip: &mut usize,
        constants: &[Value],
        stack: &mut Vec<Value>,
        locals: &mut Vec<Cell>,
        closure: &Rc<ClosureObj>,
        handlers: &mut Vec<(usize, usize)>,
    ) -> Result<Option<Value>, Value> {
        macro_rules! read_u16 { () => {{ let v = ((code[*ip] as u16) << 8) | (code[*ip+1] as u16); *ip += 2; v }} }
        macro_rules! read_u8 { () => {{ let v = code[*ip]; *ip += 1; v }} }
        macro_rules! push { ($v:expr) => { stack.push($v) } }
        macro_rules! pop { () => { stack.pop().expect("VM stack underflow (compiler bug)") } }

        match op {
            OpCode::Constant => { let idx = read_u16!(); push!(constants[idx as usize].clone()); }
            OpCode::Nil => push!(Value::Null),
            OpCode::True => push!(Value::Bool(true)),
            OpCode::False => push!(Value::Bool(false)),
            OpCode::Pop => { pop!(); }
            OpCode::GetLocal => { let slot = read_u8!(); push!(locals[slot as usize].borrow().clone()); }
            OpCode::SetLocal => { let slot = read_u8!(); let v = pop!(); *locals[slot as usize].borrow_mut() = v.clone(); push!(v); }
            OpCode::GetUpvalue => { let idx = read_u8!(); push!(closure.upvalues[idx as usize].borrow().clone()); }
            OpCode::SetUpvalue => { let idx = read_u8!(); let v = pop!(); *closure.upvalues[idx as usize].borrow_mut() = v.clone(); push!(v); }
            OpCode::GetGlobal => {
                let idx = read_u16!();
                let name = str_const(constants, idx);
                match self.globals.get(name) { Some(v) => push!(v.clone()), None => return Err(rt_err(format!("undefined variable '{}'", name))) }
            }
            OpCode::DefineGlobal => { let idx = read_u16!(); let name = str_const(constants, idx).to_string(); let v = pop!(); self.globals.insert(name, v); }
            OpCode::SetGlobal => {
                let idx = read_u16!();
                let name = str_const(constants, idx).to_string();
                let v = pop!();
                if !self.globals.contains_key(&name) { return Err(rt_err(format!("undefined variable '{}'", name))); }
                self.globals.insert(name, v.clone());
                push!(v);
            }
            OpCode::GetProperty => {
                let idx = read_u16!();
                let name = str_const(constants, idx).to_string();
                let target = pop!();
                push!(self.get_property(&target, &name)?);
            }
            OpCode::SetProperty => {
                let idx = read_u16!();
                let name = str_const(constants, idx).to_string();
                let value = pop!();
                let target = pop!();
                match &target {
                    Value::Instance(inst) => { inst.borrow_mut().fields.insert(name, value.clone()); push!(value); }
                    _ => return Err(rt_err(format!("cannot set field '{}' on {}", name, type_name(&target)))),
                }
            }
            OpCode::GetIndex => { let index = pop!(); let target = pop!(); push!(self.get_index(&target, &index)?); }
            OpCode::SetIndex => {
                let value = pop!(); let index = pop!(); let target = pop!();
                self.set_index(&target, &index, value.clone())?;
                push!(value);
            }
            OpCode::BuildList => {
                let count = read_u16!() as usize;
                let mut items = Vec::with_capacity(count);
                for _ in 0..count { items.push(pop!()); }
                items.reverse();
                push!(Value::List(Rc::new(RefCell::new(items))));
            }
            OpCode::BuildDict => {
                let pairs = read_u16!() as usize;
                let mut flat = Vec::with_capacity(pairs * 2);
                for _ in 0..pairs * 2 { flat.push(pop!()); }
                flat.reverse();
                let mut map = OrderedMap::new();
                for chunk2 in flat.chunks(2) {
                    let key = match &chunk2[0] { Value::Str(s) => (**s).clone(), other => to_display(other) };
                    map.insert(key, chunk2[1].clone());
                }
                push!(Value::Dict(Rc::new(RefCell::new(map))));
            }
            OpCode::Equal => { let b = pop!(); let a = pop!(); push!(Value::Bool(value_eq(&a, &b))); }
            OpCode::NotEqual => { let b = pop!(); let a = pop!(); push!(Value::Bool(!value_eq(&a, &b))); }
            OpCode::EqUser => { let b = pop!(); let a = pop!(); push!(Value::Bool(checked_eq(&a, &b)?)); }
            OpCode::NotEqUser => { let b = pop!(); let a = pop!(); push!(Value::Bool(!checked_eq(&a, &b)?)); }
            OpCode::Greater | OpCode::GreaterEqual | OpCode::Less | OpCode::LessEqual => {
                let b = pop!(); let a = pop!();
                let r = match (&a, &b) {
                    (Value::Str(x), Value::Str(y)) => match op { OpCode::Greater => x > y, OpCode::GreaterEqual => x >= y, OpCode::Less => x < y, _ => x <= y },
                    _ => { let (x, y) = (num(&a)?, num(&b)?); match op { OpCode::Greater => x > y, OpCode::GreaterEqual => x >= y, OpCode::Less => x < y, _ => x <= y } }
                };
                push!(Value::Bool(r));
            }
            OpCode::Add => {
                let b = pop!(); let a = pop!();
                push!(if matches!(a, Value::Tensor(_)) || matches!(b, Value::Tensor(_)) { tensor_binop(&a, &b, |x, y| x + y)? } else { add_values(&a, &b)? });
            }
            OpCode::Subtract => {
                let b = pop!(); let a = pop!();
                push!(if matches!(a, Value::Tensor(_)) || matches!(b, Value::Tensor(_)) { tensor_binop(&a, &b, |x, y| x - y)? } else { arith(&a, &b, i64::wrapping_sub, |x, y| x - y)? });
            }
            OpCode::Multiply => {
                let b = pop!(); let a = pop!();
                push!(if matches!(a, Value::Tensor(_)) || matches!(b, Value::Tensor(_)) { tensor_binop(&a, &b, |x, y| x * y)? } else { arith(&a, &b, i64::wrapping_mul, |x, y| x * y)? });
            }
            OpCode::Divide => {
                let b = pop!(); let a = pop!();
                push!(if matches!(a, Value::Tensor(_)) || matches!(b, Value::Tensor(_)) {
                    // Elementwise divide follows ordinary float division
                    // (inf/nan on a zero divisor) rather than the scalar
                    // path's hard error below — erroring out an entire
                    // tensor op because one of many elements divided by
                    // zero would be a worse failure mode than IEEE
                    // semantics here. Documented in docs/SPEC.md.
                    tensor_binop(&a, &b, |x, y| x / y)?
                } else {
                    let d = num(&b)?;
                    if d == 0.0 { return Err(rt_err("division by zero")); }
                    Value::Float(num(&a)? / d)
                });
            }
            OpCode::Modulo => { let b = pop!(); let a = pop!(); push!(match (&a, &b) { (Value::Int(x), Value::Int(y)) if *y != 0 => Value::Int(x % y), _ => Value::Float(num(&a)? % num(&b)?) }); }
            OpCode::Power => { let b = pop!(); let a = pop!(); push!(Value::Float(num(&a)?.powf(num(&b)?))); }
            OpCode::MatMul => {
                let b = pop!(); let a = pop!();
                push!(if matches!(a, Value::Tensor(_)) || matches!(b, Value::Tensor(_)) { tensor_matmul(&a, &b)? } else { matmul(&a, &b)? });
            }
            OpCode::Not => { let v = pop!(); push!(Value::Bool(!truthy(&v))); }
            OpCode::Negate => { let v = pop!(); push!(Value::Float(-num(&v)?)); }
            OpCode::ToStr => { let v = pop!(); push!(Value::Str(Rc::new(to_display(&v)))); }
            OpCode::Jump => { let target = read_u16!(); *ip = target as usize; }
            OpCode::JumpIfFalse => { let target = read_u16!(); if !truthy(stack.last().expect("VM stack underflow (compiler bug)")) { *ip = target as usize; } }
            OpCode::Loop => { let target = read_u16!(); *ip = target as usize; }
            OpCode::PushTry => { let catch_ip = read_u16!(); handlers.push((stack.len(), catch_ip as usize)); }
            OpCode::PopTry => { handlers.pop(); }
            OpCode::Throw => { let v = pop!(); return Err(v); }
            OpCode::Len => {
                let v = pop!();
                push!(Value::Int(match &v {
                    Value::List(l) => l.borrow().len() as i64,
                    Value::Dict(d) => d.borrow().len() as i64,
                    Value::Str(s) => s.chars().count() as i64,
                    Value::Tensor(t) => *t.shape.first().unwrap_or(&0) as i64,
                    other => return Err(rt_err(format!("len() requires a list, dict, string, or tensor, got {}", type_name(other)))),
                }));
            }
            OpCode::GetIterList => {
                let v = pop!();
                push!(match &v {
                    Value::List(_) => v,
                    Value::Str(s) => Value::List(Rc::new(RefCell::new(s.chars().map(|c| Value::Str(Rc::new(c.to_string()))).collect()))),
                    Value::Dict(d) => Value::List(Rc::new(RefCell::new(d.borrow().keys().map(|k| Value::Str(Rc::new(k.clone()))).collect()))),
                    other => return Err(rt_err(format!("value of type {} is not iterable", type_name(other)))),
                });
            }
            OpCode::Closure => {
                let fn_idx = read_u16!();
                let template_fn = match &constants[fn_idx as usize] { Value::Closure(c) => c.function.clone(), _ => unreachable!("compiler always stores a closure template here") };
                let mut upvalues = Vec::with_capacity(template_fn.upvalue_count);
                for _ in 0..template_fn.upvalue_count {
                    let is_local = read_u8!() != 0;
                    let index = read_u8!();
                    upvalues.push(if is_local { locals[index as usize].clone() } else { closure.upvalues[index as usize].clone() });
                }
                push!(Value::Closure(Rc::new(ClosureObj { function: template_fn, upvalues })));
            }
            OpCode::Class => {
                let idx = read_u16!();
                let name = str_const(constants, idx).to_string();
                let parent_val = pop!();
                let parent = match parent_val { Value::Null => None, Value::Class(c) => Some(c), _ => return Err(rt_err("base class must be a class")) };
                push!(Value::Class(Rc::new(ClassObj { name, parent, methods: RefCell::new(HashMap::new()) })));
            }
            OpCode::Method => {
                let idx = read_u16!();
                let name = str_const(constants, idx).to_string();
                let method_val = pop!();
                let closure_obj = match method_val { Value::Closure(c) => c, _ => unreachable!("compiler always emits a closure before Method") };
                if let Some(Value::Class(c)) = stack.last() { c.methods.borrow_mut().insert(name, closure_obj); }
            }
            OpCode::Inherit => {}
            OpCode::Call => {
                let argc = read_u8!() as usize;
                let mut args = Vec::with_capacity(argc);
                for _ in 0..argc { args.push(pop!()); }
                args.reverse();
                let callee = pop!();
                push!(self.call_value(callee, args)?);
            }
            OpCode::Invoke => {
                let idx = read_u16!();
                let name = str_const(constants, idx).to_string();
                let argc = read_u8!() as usize;
                let mut args = Vec::with_capacity(argc);
                for _ in 0..argc { args.push(pop!()); }
                args.reverse();
                let target = pop!();
                push!(self.invoke(target, &name, args)?);
            }
            OpCode::Instantiate => {
                let argc = read_u8!() as usize;
                let mut args = Vec::with_capacity(argc);
                for _ in 0..argc { args.push(pop!()); }
                args.reverse();
                let class_val = pop!();
                let class = match class_val { Value::Class(c) => c, _ => return Err(rt_err(format!("cannot instantiate {}: not a class", type_name(&class_val)))) };
                let instance = Rc::new(RefCell::new(InstanceObj { class: class.clone(), fields: HashMap::new() }));
                if let Some(init) = find_method(&class, "init") {
                    self.invoke_closure(&init, Value::Instance(instance.clone()), args)?;
                }
                push!(Value::Instance(instance));
            }
            OpCode::Return => { return Ok(Some(pop!())); }
            OpCode::JumpIfFalsePop => { let target = read_u16!(); let v = pop!(); if !truthy(&v) { *ip = target as usize; } }
        }
        Ok(None)
    }

    fn get_property(&self, target: &Value, name: &str) -> Result<Value, Value> {
        match target {
            Value::Instance(inst) => {
                let b = inst.borrow();
                if let Some(v) = b.fields.get(name) { return Ok(v.clone()); }
                if let Some(m) = find_method(&b.class, name) { return Ok(Value::BoundMethod(Box::new(target.clone()), m)); }
                Err(rt_err(format!("no field or method '{}' on {}", name, b.class.name)))
            }
            Value::Dict(d) => d.borrow().get(name).cloned().ok_or_else(|| rt_err(format!("key '{}' not found", name))),
            _ => Err(rt_err(format!("cannot access field '{}' on {}", name, type_name(target)))),
        }
    }

    fn get_index(&self, target: &Value, index: &Value) -> Result<Value, Value> {
        match (target, index) {
            (Value::List(l), Value::Int(i)) => {
                let l = l.borrow();
                let (len, ii) = norm_index(l.len(), *i);
                if ii < 0 || ii as usize >= len { return Err(rt_err("list index out of range")); }
                Ok(l[ii as usize].clone())
            }
            (Value::Str(s), Value::Int(i)) => {
                let chars: Vec<char> = s.chars().collect();
                let (len, ii) = norm_index(chars.len(), *i);
                if ii < 0 || ii as usize >= len { return Err(rt_err("string index out of range")); }
                Ok(Value::Str(Rc::new(chars[ii as usize].to_string())))
            }
            (Value::Dict(d), Value::Str(k)) => d.borrow().get(k.as_str()).cloned().ok_or_else(|| rt_err(format!("key '{}' not found", k))),
            (Value::Tensor(t), Value::Int(i)) => {
                if t.shape.is_empty() { return Err(rt_err("cannot index a 0-dimensional tensor")); }
                let (len, ii) = norm_index(t.shape[0], *i);
                if ii < 0 || ii as usize >= len { return Err(rt_err("tensor index out of range")); }
                let ii = ii as usize;
                if t.shape.len() == 1 {
                    Ok(Value::Float(t.data[ii]))
                } else {
                    // Indexing a higher-rank tensor along its first axis
                    // returns the sub-tensor at that index — same idea as
                    // indexing a matrix (list of lists) returns a row.
                    let chunk_size: usize = t.shape[1..].iter().product();
                    let start = ii * chunk_size;
                    Ok(Value::Tensor(Rc::new(TensorObj { data: t.data[start..start + chunk_size].to_vec(), shape: t.shape[1..].to_vec() })))
                }
            }
            _ => Err(rt_err(format!("value of type {} is not indexable", type_name(target)))),
        }
    }

    fn set_index(&self, target: &Value, index: &Value, value: Value) -> Result<(), Value> {
        match (target, index) {
            (Value::List(l), Value::Int(i)) => {
                let mut l = l.borrow_mut();
                let (len, ii) = norm_index(l.len(), *i);
                if ii < 0 || ii as usize >= len { return Err(rt_err("list index out of range")); }
                l[ii as usize] = value;
                Ok(())
            }
            (Value::Dict(d), Value::Str(k)) => { d.borrow_mut().insert((**k).clone(), value); Ok(()) }
            _ => Err(rt_err("invalid index assignment target")),
        }
    }

    fn call_value(&mut self, callee: Value, args: Vec<Value>) -> Result<Value, Value> {
        match callee {
            Value::Closure(c) => self.invoke_closure(&c, Value::Null, args),
            Value::BoundMethod(receiver, c) => self.invoke_closure(&c, *receiver, args),
            Value::Native(name) => self.call_native(name, args),
            other => Err(rt_err(format!("value of type {} is not callable", type_name(&other)))),
        }
    }

    fn invoke_closure(&mut self, c: &Rc<ClosureObj>, receiver: Value, args: Vec<Value>) -> Result<Value, Value> {
        if args.len() < c.function.required_arity || args.len() > c.function.arity {
            let range = if c.function.required_arity == c.function.arity {
                format!("{}", c.function.arity)
            } else {
                format!("{}-{}", c.function.required_arity, c.function.arity)
            };
            return Err(rt_err(format!(
                "{}() takes {} argument{}, got {}",
                c.function.name, range, if c.function.arity == 1 { "" } else { "s" }, args.len()
            )));
        }
        let mut locals: Vec<Cell> = Vec::with_capacity(c.function.local_count);
        locals.push(Rc::new(RefCell::new(receiver)));
        for i in 0..c.function.arity {
            locals.push(Rc::new(RefCell::new(args.get(i).cloned().unwrap_or(Value::Null))));
        }
        // Slots beyond the parameters belong to `let`/for-loop/catch bindings
        // declared in the body; they start Null and get set when that
        // binding's bytecode actually runs.
        while locals.len() < c.function.local_count {
            locals.push(Rc::new(RefCell::new(Value::Null)));
        }
        self.run_frame(c.clone(), locals)
    }

    fn invoke(&mut self, target: Value, name: &str, args: Vec<Value>) -> Result<Value, Value> {
        match &target {
            Value::Instance(inst) => {
                let class = inst.borrow().class.clone();
                match find_method(&class, name) {
                    Some(m) => self.invoke_closure(&m, target.clone(), args),
                    None => Err(rt_err(format!("no method '{}' on class {}", name, class.name))),
                }
            }
            Value::List(l) => list_method(l, name, args),
            Value::Dict(d) => {
                // A dict-shaped "namespace" (from `import "x.k" as ns;`, or
                // any dict a user builds themselves) can hold real callable
                // values. Try calling an actual stored entry first; only
                // fall back to the built-in dict methods (keys/values/get/
                // remove) if there's no entry under that name. This means a
                // user dict with a literal key "keys" holding a non-function
                // value would shadow the built-in `.keys()` method — a rare
                // edge case, worth knowing about.
                if let Some(v) = d.borrow().get(name).cloned() {
                    return self.call_value(v, args);
                }
                dict_method(d, name, args)
            }
            Value::Str(s) => str_method(s, name, args),
            _ => Err(rt_err(format!("cannot call method '{}' on {}", name, type_name(&target)))),
        }
    }

    fn call_native(&mut self, name: &str, args: Vec<Value>) -> Result<Value, Value> {
        match name {
            "print" => {
                let line: Vec<String> = args.iter().map(to_display).collect();
                self.output.push_str(&line.join(" "));
                self.output.push('\n');
                Ok(Value::Null)
            }
            "len" => match args.get(0) {
                Some(Value::List(l)) => Ok(Value::Int(l.borrow().len() as i64)),
                Some(Value::Dict(d)) => Ok(Value::Int(d.borrow().len() as i64)),
                Some(Value::Str(s)) => Ok(Value::Int(s.chars().count() as i64)),
                _ => Err(rt_err("len() requires a list, dict, or string")),
            },
            "str" => Ok(Value::Str(Rc::new(to_display(args.get(0).unwrap_or(&Value::Null))))),
            "int" => match args.get(0) {
                Some(Value::Int(i)) => Ok(Value::Int(*i)),
                Some(Value::Float(f)) => Ok(Value::Int(*f as i64)),
                Some(Value::Bool(b)) => Ok(Value::Int(if *b { 1 } else { 0 })),
                Some(Value::Str(s)) => s.trim().parse::<i64>().map(Value::Int).map_err(|_| rt_err(format!("cannot convert '{}' to int", s))),
                _ => Err(rt_err("int() requires a number, bool, or string")),
            },
            "float" => match args.get(0) { Some(v) => Ok(Value::Float(num(v)?)), None => Err(rt_err("float() requires an argument")) },
            "bool" => Ok(Value::Bool(truthy(args.get(0).unwrap_or(&Value::Null)))),
            "type" => Ok(Value::Str(Rc::new(type_name(args.get(0).unwrap_or(&Value::Null)).to_string()))),
            "range" => {
                let (start, end, step) = match args.len() {
                    1 => (0i64, as_int(&args[0])?, 1i64),
                    2 => (as_int(&args[0])?, as_int(&args[1])?, 1i64),
                    3 => (as_int(&args[0])?, as_int(&args[1])?, as_int(&args[2])?),
                    _ => return Err(rt_err("range() takes 1 to 3 arguments")),
                };
                if step == 0 { return Err(rt_err("range() step cannot be 0")); }
                let mut v = Vec::new();
                if step > 0 { let mut i = start; while i < end { v.push(Value::Int(i)); i += step; } }
                else { let mut i = start; while i > end { v.push(Value::Int(i)); i += step; } }
                Ok(Value::List(Rc::new(RefCell::new(v))))
            }
            "abs" => Ok(Value::Float(num(args.get(0).unwrap_or(&Value::Null))?.abs())),
            "min" => reduce_numeric(&args, f64::min),
            "max" => reduce_numeric(&args, f64::max),
            "sum" => match args.get(0) { Some(Value::List(l)) => { let mut t = 0.0; for v in l.borrow().iter() { t += num(v)?; } Ok(Value::Float(t)) } _ => Err(rt_err("sum() requires a list")) },
            "sorted" => match args.get(0) {
                Some(Value::List(l)) => { let mut v = l.borrow().clone(); v.sort_by(|a, b| num(a).unwrap_or(0.0).partial_cmp(&num(b).unwrap_or(0.0)).unwrap_or(std::cmp::Ordering::Equal)); Ok(Value::List(Rc::new(RefCell::new(v)))) }
                _ => Err(rt_err("sorted() requires a list")),
            },
            "round" => Ok(Value::Int(num(args.get(0).unwrap_or(&Value::Null))?.round() as i64)),
            "input" => {
                let mut line = String::new();
                match std::io::stdin().read_line(&mut line) {
                    Ok(0) => Ok(Value::Str(Rc::new(String::new()))), // EOF
                    Ok(_) => {
                        if line.ends_with('\n') { line.pop(); if line.ends_with('\r') { line.pop(); } }
                        Ok(Value::Str(Rc::new(line)))
                    }
                    Err(e) => Err(rt_err(format!("input() failed: {}", e))),
                }
            }
            "assert" => match args.get(0) {
                Some(v) if truthy(v) => Ok(Value::Null),
                _ => Err(rt_err(match args.get(1) { Some(m) => to_display(m), None => "assertion failed".to_string() })),
            },
            "args" => Ok(Value::List(Rc::new(RefCell::new(self.script_args.iter().map(|s| Value::Str(Rc::new(s.clone()))).collect())))),
            "relu" => Ok(map_elementwise(args.get(0).unwrap_or(&Value::Null), |x| x.max(0.0))),
            "sigmoid" => Ok(map_elementwise(args.get(0).unwrap_or(&Value::Null), |x| 1.0 / (1.0 + (-x).exp()))),
            "tanh" => Ok(map_elementwise(args.get(0).unwrap_or(&Value::Null), |x| x.tanh())),
            "softmax" => softmax(args.get(0).unwrap_or(&Value::Null)),
            "transpose" => transpose(args.get(0).unwrap_or(&Value::Null)),
            "flatten" => Ok(Value::List(Rc::new(RefCell::new(flatten(args.get(0).unwrap_or(&Value::Null)))))),
            "sqrt" => Ok(Value::Float(num(args.get(0).unwrap_or(&Value::Null))?.sqrt())),
            "floor" => Ok(Value::Float(num(args.get(0).unwrap_or(&Value::Null))?.floor())),
            "ceil" => Ok(Value::Float(num(args.get(0).unwrap_or(&Value::Null))?.ceil())),
            "pow" => Ok(Value::Float(num(args.get(0).unwrap_or(&Value::Null))?.powf(num(args.get(1).unwrap_or(&Value::Null))?))),
            "log" => {
                let x = num(args.get(0).unwrap_or(&Value::Null))?;
                match args.get(1) { Some(b) => Ok(Value::Float(x.log(num(b)?))), None => Ok(Value::Float(x.ln())) }
            }
            "exp" => Ok(Value::Float(num(args.get(0).unwrap_or(&Value::Null))?.exp())),
            "sin" => Ok(Value::Float(num(args.get(0).unwrap_or(&Value::Null))?.sin())),
            "cos" => Ok(Value::Float(num(args.get(0).unwrap_or(&Value::Null))?.cos())),
            "tan" => Ok(Value::Float(num(args.get(0).unwrap_or(&Value::Null))?.tan())),
            "map" => {
                let list = match args.get(0) { Some(Value::List(l)) => l.clone(), _ => return Err(rt_err("map() requires a list as the first argument")) };
                let f = args.get(1).cloned().ok_or_else(|| rt_err("map() requires a function as the second argument"))?;
                let items = list.borrow().clone();
                let mut out = Vec::with_capacity(items.len());
                for item in items { out.push(self.call_value(f.clone(), vec![item])?); }
                Ok(Value::List(Rc::new(RefCell::new(out))))
            }
            "filter" => {
                let list = match args.get(0) { Some(Value::List(l)) => l.clone(), _ => return Err(rt_err("filter() requires a list as the first argument")) };
                let f = args.get(1).cloned().ok_or_else(|| rt_err("filter() requires a function as the second argument"))?;
                let items = list.borrow().clone();
                let mut out = Vec::new();
                for item in items { if truthy(&self.call_value(f.clone(), vec![item.clone()])?) { out.push(item); } }
                Ok(Value::List(Rc::new(RefCell::new(out))))
            }
            "reduce" => {
                let list = match args.get(0) { Some(Value::List(l)) => l.clone(), _ => return Err(rt_err("reduce() requires a list as the first argument")) };
                let f = args.get(1).cloned().ok_or_else(|| rt_err("reduce() requires a function as the second argument"))?;
                let items = list.borrow().clone();
                let mut iter = items.into_iter();
                let mut acc = match args.get(2) {
                    Some(v) => v.clone(),
                    None => iter.next().ok_or_else(|| rt_err("reduce() of an empty list needs an initial value"))?,
                };
                for item in iter { acc = self.call_value(f.clone(), vec![acc, item])?; }
                Ok(acc)
            }
            "json_encode" => Ok(Value::Str(Rc::new(json_encode(args.get(0).unwrap_or(&Value::Null))))),
            "json_decode" => match args.get(0) {
                Some(Value::Str(s)) => json_decode(s),
                _ => Err(rt_err("json_decode() requires a string")),
            },
            "read_file" => match args.get(0) {
                Some(Value::Str(p)) => std::fs::read_to_string(p.as_str()).map(|s| Value::Str(Rc::new(s))).map_err(|e| rt_err(format!("cannot read '{}': {}", p, e))),
                _ => Err(rt_err("read_file() requires a string path")),
            },
            "write_file" => match (args.get(0), args.get(1)) {
                (Some(Value::Str(p)), Some(content)) => std::fs::write(p.as_str(), to_display(content)).map(|_| Value::Null).map_err(|e| rt_err(format!("cannot write '{}': {}", p, e))),
                _ => Err(rt_err("write_file() requires a path and content")),
            },
            "append_file" => match (args.get(0), args.get(1)) {
                (Some(Value::Str(p)), Some(content)) => {
                    use std::io::Write as _;
                    std::fs::OpenOptions::new().create(true).append(true).open(p.as_str())
                        .and_then(|mut f| f.write_all(to_display(content).as_bytes()))
                        .map(|_| Value::Null)
                        .map_err(|e| rt_err(format!("cannot append to '{}': {}", p, e)))
                }
                _ => Err(rt_err("append_file() requires a path and content")),
            },
            "remove_file" => match args.get(0) {
                Some(Value::Str(p)) => std::fs::remove_file(p.as_str()).map(|_| Value::Null).map_err(|e| rt_err(format!("cannot remove '{}': {}", p, e))),
                _ => Err(rt_err("remove_file() requires a string path")),
            },
            "file_exists" => match args.get(0) {
                Some(Value::Str(p)) => Ok(Value::Bool(std::path::Path::new(p.as_str()).exists())),
                _ => Err(rt_err("file_exists() requires a string path")),
            },
            "random" => Ok(Value::Float((self.next_random_u64() >> 11) as f64 / (1u64 << 53) as f64)),
            "randint" => {
                let a = as_int(args.get(0).unwrap_or(&Value::Null))?;
                let b = as_int(args.get(1).unwrap_or(&Value::Null))?;
                if a > b { return Err(rt_err("randint() requires the first argument to be <= the second")); }
                let range = (b - a + 1) as u64;
                Ok(Value::Int(a + (self.next_random_u64() % range) as i64))
            }
            "time_now" => Ok(Value::Float(
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0),
            )),
            "date_string" => {
                let dur = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
                let total_secs = dur.as_secs() as i64;
                let days = total_secs.div_euclid(86400);
                let secs_of_day = total_secs.rem_euclid(86400);
                let (y, m, d) = civil_from_days(days);
                let (h, mi, s) = (secs_of_day / 3600, (secs_of_day % 3600) / 60, secs_of_day % 60);
                Ok(Value::Str(Rc::new(format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC", y, m, d, h, mi, s))))
            }
            "tensor" => match args.get(0) {
                Some(v) => nested_list_to_tensor(v).map(|t| Value::Tensor(Rc::new(t))),
                None => Err(rt_err("tensor() requires a nested list argument")),
            },
            "shape" => match args.get(0) {
                Some(Value::Tensor(t)) => Ok(Value::List(Rc::new(RefCell::new(t.shape.iter().map(|d| Value::Int(*d as i64)).collect())))),
                Some(other) => Err(rt_err(format!("shape() requires a tensor, got {}", type_name(other)))),
                None => Err(rt_err("shape() requires a tensor argument")),
            },
            "to_list" => match args.get(0) {
                Some(Value::Tensor(t)) => Ok(tensor_to_nested_list(t)),
                Some(other) => Err(rt_err(format!("to_list() requires a tensor, got {}", type_name(other)))),
                None => Err(rt_err("to_list() requires a tensor argument")),
            },
            "zeros" => make_filled_tensor(args.get(0), 0.0),
            "ones" => make_filled_tensor(args.get(0), 1.0),
            "reshape" => {
                let t = match args.get(0) { Some(Value::Tensor(t)) => t.clone(), _ => return Err(rt_err("reshape() requires a tensor as the first argument")) };
                let new_shape: Vec<usize> = match args.get(1) {
                    Some(Value::List(l)) => l.borrow().iter().map(as_int).collect::<Result<Vec<i64>, _>>()?.into_iter().map(|x| x.max(0) as usize).collect(),
                    _ => return Err(rt_err("reshape() requires a list of dimensions as the second argument")),
                };
                let expected: usize = new_shape.iter().product();
                if expected != t.data.len() {
                    return Err(rt_err(format!("cannot reshape a tensor of {} elements into shape {:?} ({} elements)", t.data.len(), new_shape, expected)));
                }
                Ok(Value::Tensor(Rc::new(TensorObj { data: t.data.clone(), shape: new_shape })))
            }
            "save_weights" => match (args.get(0), args.get(1)) {
                (Some(Value::Tensor(t)), Some(Value::Str(path))) => {
                    let shape_json = json_encode(&Value::List(Rc::new(RefCell::new(t.shape.iter().map(|d| Value::Int(*d as i64)).collect()))));
                    let data_json = json_encode(&Value::List(Rc::new(RefCell::new(t.data.iter().map(|x| Value::Float(*x)).collect()))));
                    let json = format!("{{\"shape\":{},\"data\":{}}}", shape_json, data_json);
                    std::fs::write(path.as_str(), json).map(|_| Value::Null).map_err(|e| rt_err(format!("cannot write '{}': {}", path, e)))
                }
                _ => Err(rt_err("save_weights() requires a tensor and a path")),
            },
            "load_weights" => match args.get(0) {
                Some(Value::Str(path)) => {
                    let text = std::fs::read_to_string(path.as_str()).map_err(|e| rt_err(format!("cannot read '{}': {}", path, e)))?;
                    let decoded = json_decode(&text)?;
                    let dict = match &decoded { Value::Dict(d) => d.clone(), _ => return Err(rt_err("weights file must contain a JSON object with 'shape' and 'data'")) };
                    let shape_val = dict.borrow().get("shape").cloned().ok_or_else(|| rt_err("weights file is missing 'shape'"))?;
                    let data_val = dict.borrow().get("data").cloned().ok_or_else(|| rt_err("weights file is missing 'data'"))?;
                    let shape: Vec<usize> = match shape_val { Value::List(l) => l.borrow().iter().map(as_int).collect::<Result<Vec<i64>, _>>()?.into_iter().map(|x| x.max(0) as usize).collect(), _ => return Err(rt_err("'shape' must be a list of ints")) };
                    let data: Vec<f64> = match data_val { Value::List(l) => l.borrow().iter().map(num).collect::<Result<Vec<f64>, _>>()?, _ => return Err(rt_err("'data' must be a list of numbers")) };
                    let expected: usize = shape.iter().product();
                    if data.len() != expected { return Err(rt_err(format!("weights file is corrupt: shape {:?} implies {} elements, but 'data' has {}", shape, expected, data.len()))); }
                    Ok(Value::Tensor(Rc::new(TensorObj { data, shape })))
                }
                _ => Err(rt_err("load_weights() requires a path")),
            },
            _ => Err(rt_err(format!("unknown builtin '{}'", name))),
        }
    }
}

fn str_const(constants: &[Value], idx: u16) -> &str { match &constants[idx as usize] { Value::Str(s) => s, _ => unreachable!("compiler always stores names as string constants") } }
fn rt_err(msg: impl Into<String>) -> Value { Value::Str(Rc::new(msg.into())) }
fn norm_index(len: usize, i: i64) -> (usize, i64) { let ii = if i < 0 { len as i64 + i } else { i }; (len, ii) }

fn num(v: &Value) -> Result<f64, Value> {
    match v {
        Value::Int(i) => Ok(*i as f64),
        Value::Float(f) => Ok(*f),
        Value::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
        other => Err(rt_err(format!("expected a number, got {}", type_name(other)))),
    }
}
fn as_int(v: &Value) -> Result<i64, Value> { match v { Value::Int(i) => Ok(*i), Value::Float(f) => Ok(*f as i64), other => Err(rt_err(format!("expected an integer, got {}", type_name(other)))) } }

fn arith(l: &Value, r: &Value, fi: fn(i64, i64) -> i64, ff: fn(f64, f64) -> f64) -> Result<Value, Value> {
    match (l, r) { (Value::Int(a), Value::Int(b)) => Ok(Value::Int(fi(*a, *b))), _ => Ok(Value::Float(ff(num(l)?, num(r)?))) }
}

fn add_values(l: &Value, r: &Value) -> Result<Value, Value> {
    match (l, r) {
        (Value::Str(a), _) => Ok(Value::Str(Rc::new(format!("{}{}", a, to_display(r))))),
        (_, Value::Str(b)) => Ok(Value::Str(Rc::new(format!("{}{}", to_display(l), b)))),
        (Value::List(a), Value::List(b)) => { let mut v = a.borrow().clone(); v.extend(b.borrow().clone()); Ok(Value::List(Rc::new(RefCell::new(v)))) }
        _ => arith(l, r, i64::wrapping_add, |a, b| a + b),
    }
}

fn list_method(l: &Rid<Vec<Value>>, name: &str, args: Vec<Value>) -> Result<Value, Value> {
    match name {
        "append" | "push" => { l.borrow_mut().push(args.into_iter().next().unwrap_or(Value::Null)); Ok(Value::Null) }
        "pop" => Ok(l.borrow_mut().pop().unwrap_or(Value::Null)),
        "sort" => { l.borrow_mut().sort_by(|a, b| num(a).unwrap_or(0.0).partial_cmp(&num(b).unwrap_or(0.0)).unwrap_or(std::cmp::Ordering::Equal)); Ok(Value::Null) }
        "reverse" => { l.borrow_mut().reverse(); Ok(Value::Null) }
        "contains" => { let t = args.into_iter().next().unwrap_or(Value::Null); Ok(Value::Bool(l.borrow().iter().any(|v| value_eq(v, &t)))) }
        _ => Err(rt_err(format!("no list method '{}'", name))),
    }
}
fn dict_method(d: &Rid<OrderedMap>, name: &str, args: Vec<Value>) -> Result<Value, Value> {
    match name {
        "keys" => Ok(Value::List(Rc::new(RefCell::new(d.borrow().keys().map(|k| Value::Str(Rc::new(k.clone()))).collect())))),
        "values" => Ok(Value::List(Rc::new(RefCell::new(d.borrow().values().cloned().collect())))),
        "get" => { let key = match args.get(0) { Some(Value::Str(s)) => (**s).clone(), _ => return Err(rt_err("get() requires a string key")) }; Ok(d.borrow().get(&key).cloned().unwrap_or_else(|| args.get(1).cloned().unwrap_or(Value::Null))) }
        "remove" => { let key = match args.get(0) { Some(Value::Str(s)) => (**s).clone(), _ => return Err(rt_err("remove() requires a string key")) }; Ok(d.borrow_mut().remove(&key).unwrap_or(Value::Null)) }
        _ => Err(rt_err(format!("no dict method '{}'", name))),
    }
}
fn str_method(s: &str, name: &str, args: Vec<Value>) -> Result<Value, Value> {
    match name {
        "upper" => Ok(Value::Str(Rc::new(s.to_uppercase()))),
        "lower" => Ok(Value::Str(Rc::new(s.to_lowercase()))),
        "trim" => Ok(Value::Str(Rc::new(s.trim().to_string()))),
        "split" => { let sep = match args.get(0) { Some(Value::Str(x)) => (**x).clone(), _ => " ".to_string() }; Ok(Value::List(Rc::new(RefCell::new(s.split(sep.as_str()).map(|p| Value::Str(Rc::new(p.to_string()))).collect())))) }
        "replace" => { let a = match args.get(0) { Some(Value::Str(x)) => (**x).clone(), _ => return Err(rt_err("replace() requires string arguments")) }; let b = match args.get(1) { Some(Value::Str(x)) => (**x).clone(), _ => String::new() }; Ok(Value::Str(Rc::new(s.replace(a.as_str(), b.as_str())))) }
        "contains" => Ok(Value::Bool(matches!(args.get(0), Some(Value::Str(a)) if s.contains(a.as_str())))),
        "startsWith" => Ok(Value::Bool(matches!(args.get(0), Some(Value::Str(a)) if s.starts_with(a.as_str())))),
        "endsWith" => Ok(Value::Bool(matches!(args.get(0), Some(Value::Str(a)) if s.ends_with(a.as_str())))),
        "padStart" | "padLeft" => pad_string(s, &args, true),
        "padEnd" | "padRight" => pad_string(s, &args, false),
        "repeat" => { let n = as_int(args.get(0).unwrap_or(&Value::Null))?.max(0) as usize; Ok(Value::Str(Rc::new(s.repeat(n)))) }
        _ => Err(rt_err(format!("no string method '{}'", name))),
    }
}

fn pad_string(s: &str, args: &[Value], at_start: bool) -> Result<Value, Value> {
    let width = match args.get(0) { Some(v) => as_int(v)?.max(0) as usize, None => return Err(rt_err("pad methods require a target width")) };
    let fill = match args.get(1) { Some(Value::Str(f)) => f.chars().next().unwrap_or(' '), _ => ' ' };
    let cur_len = s.chars().count();
    if cur_len >= width { return Ok(Value::Str(Rc::new(s.to_string()))); }
    let pad: String = std::iter::repeat(fill).take(width - cur_len).collect();
    Ok(Value::Str(Rc::new(if at_start { format!("{}{}", pad, s) } else { format!("{}{}", s, pad) })))
}

fn as_matrix(v: &Value) -> Option<Vec<Vec<f64>>> {
    if let Value::List(rows) = v {
        let rows = rows.borrow();
        let mut m = Vec::new();
        for row in rows.iter() {
            if let Value::List(cols) = row { let cols = cols.borrow(); let mut r = Vec::with_capacity(cols.len()); for c in cols.iter() { r.push(num(c).ok()?); } m.push(r); } else { return None; }
        }
        Some(m)
    } else { None }
}
fn matrix_to_value(m: Vec<Vec<f64>>) -> Value { Value::List(Rc::new(RefCell::new(m.into_iter().map(|row| Value::List(Rc::new(RefCell::new(row.into_iter().map(Value::Float).collect())))).collect()))) }
fn matmul(l: &Value, r: &Value) -> Result<Value, Value> {
    let a = as_matrix(l).ok_or_else(|| rt_err("'@' requires two matrices (lists of lists of numbers)"))?;
    let b = as_matrix(r).ok_or_else(|| rt_err("'@' requires two matrices (lists of lists of numbers)"))?;
    if a.is_empty() || b.is_empty() || a[0].len() != b.len() { return Err(rt_err("matrix dimension mismatch for '@' (inner dimensions must match)")); }
    let mut result = vec![vec![0.0; b[0].len()]; a.len()];
    for i in 0..a.len() { for j in 0..b[0].len() { for k in 0..a[0].len() { result[i][j] += a[i][k] * b[k][j]; } } }
    Ok(matrix_to_value(result))
}

// ---- Tensor: flat Vec<f64> + shape, with NumPy-style broadcasting ----
// (Phase E.) Coexists with the older nested-list matrix representation
// above rather than replacing it — `[[1,2],[3,4]] @ [[5,6],[7,8]]` (plain
// lists) keeps working exactly as before via `matmul`/`as_matrix`; a
// Tensor only enters the picture when the script explicitly calls
// `tensor(...)`, `zeros(...)`, `ones(...)`, or `load_weights(...)`, or
// when a Tensor meets a list/scalar in an arithmetic expression (in which
// case the list/scalar is coerced into a Tensor for that one operation).

/// Right-aligned NumPy broadcasting: shapes are compatible dimension-by-
/// dimension (comparing from the trailing/last dimension backward) when
/// each pair is either equal or one of them is 1; a shorter shape is
/// treated as having 1s in its missing leading dimensions.
fn broadcast_shape(a: &[usize], b: &[usize]) -> Result<Vec<usize>, String> {
    let len = a.len().max(b.len());
    let mut out = vec![0usize; len];
    for i in 0..len {
        let ai = if i < len - a.len() { 1 } else { a[i - (len - a.len())] };
        let bi = if i < len - b.len() { 1 } else { b[i - (len - b.len())] };
        out[i] = if ai == bi { ai } else if ai == 1 { bi } else if bi == 1 { ai } else {
            return Err(format!("cannot broadcast tensor shapes {:?} and {:?}", a, b));
        };
    }
    Ok(out)
}

fn strides_for(shape: &[usize]) -> Vec<usize> {
    let mut strides = vec![1usize; shape.len()];
    for i in (0..shape.len().saturating_sub(1)).rev() { strides[i] = strides[i + 1] * shape[i + 1]; }
    strides
}

/// Maps a full output multi-index back to a flat offset into an input
/// tensor that may have fewer dimensions than the output (right-aligned)
/// or a size-1 dimension where the output isn't (broadcast: always read
/// index 0 along that axis).
fn broadcast_index(shape: &[usize], strides: &[usize], out_idx: &[usize], out_rank: usize) -> usize {
    let offset = out_rank - shape.len();
    let mut flat = 0usize;
    for i in 0..shape.len() {
        let coord = if shape[i] == 1 { 0 } else { out_idx[i + offset] };
        flat += coord * strides[i];
    }
    flat
}

fn broadcast_binop_raw(a: &TensorObj, b: &TensorObj, f: impl Fn(f64, f64) -> f64) -> Result<TensorObj, String> {
    let out_shape = broadcast_shape(&a.shape, &b.shape)?;
    let out_strides = strides_for(&out_shape);
    let (a_strides, b_strides) = (strides_for(&a.shape), strides_for(&b.shape));
    let numel: usize = out_shape.iter().product();
    let mut data = Vec::with_capacity(numel);
    let mut idx = vec![0usize; out_shape.len()];
    for flat in 0..numel {
        let mut rem = flat;
        for d in 0..out_shape.len() {
            idx[d] = if out_strides[d] == 0 { 0 } else { rem / out_strides[d] };
            rem = if out_strides[d] == 0 { 0 } else { rem % out_strides[d] };
        }
        let ai = broadcast_index(&a.shape, &a_strides, &idx, out_shape.len());
        let bi = broadcast_index(&b.shape, &b_strides, &idx, out_shape.len());
        data.push(f(a.data[ai], b.data[bi]));
    }
    Ok(TensorObj { data, shape: out_shape })
}

/// Coerces anything arithmetic-shaped (a Tensor as-is; a number as a
/// 0-dimensional/scalar tensor; a rectangular nested list) into flat
/// data+shape, so `tensor + [[1,2],[3,4]]` and `tensor * 2` both work
/// without the caller needing to `tensor(...)`-wrap everything by hand.
fn to_tensor_like(v: &Value) -> Option<(Vec<f64>, Vec<usize>)> {
    match v {
        Value::Tensor(t) => Some((t.data.clone(), t.shape.clone())),
        Value::Int(i) => Some((vec![*i as f64], vec![])),
        Value::Float(f) => Some((vec![*f], vec![])),
        Value::Bool(b) => Some((vec![if *b { 1.0 } else { 0.0 }], vec![])),
        Value::List(_) => nested_list_to_tensor(v).ok().map(|t| (t.data, t.shape)),
        _ => None,
    }
}

fn tensor_binop(a: &Value, b: &Value, f: impl Fn(f64, f64) -> f64) -> Result<Value, Value> {
    let (a_data, a_shape) = to_tensor_like(a).ok_or_else(|| rt_err(format!("cannot use {} in a tensor operation", type_name(a))))?;
    let (b_data, b_shape) = to_tensor_like(b).ok_or_else(|| rt_err(format!("cannot use {} in a tensor operation", type_name(b))))?;
    let result = broadcast_binop_raw(&TensorObj { data: a_data, shape: a_shape }, &TensorObj { data: b_data, shape: b_shape }, f).map_err(rt_err)?;
    Ok(Value::Tensor(Rc::new(result)))
}

fn tensor_matmul(a: &Value, b: &Value) -> Result<Value, Value> {
    let (ad, ashape) = to_tensor_like(a).ok_or_else(|| rt_err("'@' requires matrices or tensors"))?;
    let (bd, bshape) = to_tensor_like(b).ok_or_else(|| rt_err("'@' requires matrices or tensors"))?;
    if ashape.len() != 2 || bshape.len() != 2 || ashape[1] != bshape[0] {
        return Err(rt_err(format!("matrix dimension mismatch for '@': shapes {:?} and {:?}", ashape, bshape)));
    }
    let (m, k, n) = (ashape[0], ashape[1], bshape[1]);
    let mut data = vec![0.0; m * n];
    for i in 0..m {
        for j in 0..n {
            let mut sum = 0.0;
            for kk in 0..k { sum += ad[i * k + kk] * bd[kk * n + j]; }
            data[i * n + j] = sum;
        }
    }
    Ok(Value::Tensor(Rc::new(TensorObj { data, shape: vec![m, n] })))
}

/// Converts an arbitrary-rank rectangular nested list of numbers into flat
/// data+shape. "Rectangular" is enforced by checking the flattened element
/// count against the shape inferred from the first element at each level —
/// a ragged list (rows of different lengths) is a clear error instead of
/// silently producing garbage or a wrong shape.
fn nested_list_to_tensor(v: &Value) -> Result<TensorObj, Value> {
    fn shape_of(v: &Value) -> Vec<usize> {
        match v {
            Value::List(l) => {
                let b = l.borrow();
                let mut s = vec![b.len()];
                if let Some(first) = b.first() { s.extend(shape_of(first)); }
                s
            }
            _ => vec![],
        }
    }
    fn collect(v: &Value, out: &mut Vec<f64>) -> Result<(), Value> {
        match v {
            Value::List(l) => { for item in l.borrow().iter() { collect(item, out)?; } Ok(()) }
            other => { out.push(num(other)?); Ok(()) }
        }
    }
    let shape = shape_of(v);
    let mut data = Vec::new();
    collect(v, &mut data)?;
    let expected: usize = shape.iter().product();
    if data.len() != expected {
        return Err(rt_err("tensor() requires a rectangular nested list (every sub-list at a given level must have the same length)"));
    }
    Ok(TensorObj { data, shape })
}

fn tensor_to_nested_list(t: &TensorObj) -> Value {
    fn build(data: &[f64], shape: &[usize]) -> Value {
        match shape {
            [] => Value::Float(data[0]),
            [_] => Value::List(Rc::new(RefCell::new(data.iter().map(|x| Value::Float(*x)).collect()))),
            [_, rest @ ..] => {
                let chunk_size: usize = rest.iter().product();
                Value::List(Rc::new(RefCell::new(data.chunks(chunk_size).map(|c| build(c, rest)).collect())))
            }
        }
    }
    build(&t.data, &t.shape)
}

fn make_filled_tensor(shape_arg: Option<&Value>, fill: f64) -> Result<Value, Value> {
    let shape: Vec<usize> = match shape_arg {
        Some(Value::List(l)) => l.borrow().iter().map(as_int).collect::<Result<Vec<i64>, _>>()?.into_iter().map(|x| x.max(0) as usize).collect(),
        Some(Value::Int(n)) => vec![(*n).max(0) as usize],
        _ => return Err(rt_err("zeros()/ones() require a shape: an int, or a list of ints")),
    };
    let numel: usize = shape.iter().product();
    Ok(Value::Tensor(Rc::new(TensorObj { data: vec![fill; numel], shape })))
}
fn transpose(v: &Value) -> Result<Value, Value> {
    if let Value::Tensor(t) = v {
        if t.shape.len() != 2 { return Err(rt_err("transpose() on a tensor currently only supports 2-D tensors")); }
        let (rows, cols) = (t.shape[0], t.shape[1]);
        let mut data = vec![0.0; rows * cols];
        for i in 0..rows { for j in 0..cols { data[j * rows + i] = t.data[i * cols + j]; } }
        return Ok(Value::Tensor(Rc::new(TensorObj { data, shape: vec![cols, rows] })));
    }
    let m = as_matrix(v).ok_or_else(|| rt_err("transpose() requires a matrix (list of lists) or a 2-D tensor"))?;
    if m.is_empty() { return Ok(matrix_to_value(m)); }
    let (rows, cols) = (m.len(), m[0].len());
    let mut t = vec![vec![0.0; rows]; cols];
    for i in 0..rows { for j in 0..cols { t[j][i] = m[i][j]; } }
    Ok(matrix_to_value(t))
}
fn map_elementwise(v: &Value, f: fn(f64) -> f64) -> Value {
    match v {
        Value::List(l) => Value::List(Rc::new(RefCell::new(l.borrow().iter().map(|x| map_elementwise(x, f)).collect()))),
        Value::Int(i) => Value::Float(f(*i as f64)),
        Value::Float(x) => Value::Float(f(*x)),
        Value::Tensor(t) => Value::Tensor(Rc::new(TensorObj { data: t.data.iter().map(|x| f(*x)).collect(), shape: t.shape.clone() })),
        other => other.clone(),
    }
}
fn softmax(v: &Value) -> Result<Value, Value> {
    if let Value::Tensor(t) = v {
        if t.shape.len() != 1 { return Err(rt_err("softmax() on a tensor currently only supports 1-D tensors")); }
        let max = t.data.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let exps: Vec<f64> = t.data.iter().map(|x| (x - max).exp()).collect();
        let sum: f64 = exps.iter().sum();
        return Ok(Value::Tensor(Rc::new(TensorObj { data: exps.into_iter().map(|x| x / sum).collect(), shape: t.shape.clone() })));
    }
    if let Value::List(l) = v {
        let nums: Vec<f64> = l.borrow().iter().map(num).collect::<Result<_, _>>()?;
        let max = nums.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let exps: Vec<f64> = nums.iter().map(|x| (x - max).exp()).collect();
        let sum: f64 = exps.iter().sum();
        Ok(Value::List(Rc::new(RefCell::new(exps.into_iter().map(|x| Value::Float(x / sum)).collect()))))
    } else { Err(rt_err("softmax() requires a list of numbers or a 1-D tensor")) }
}
fn flatten(v: &Value) -> Vec<Value> {
    match v {
        Value::List(l) => l.borrow().iter().flat_map(flatten).collect(),
        Value::Tensor(t) => t.data.iter().map(|x| Value::Float(*x)).collect(),
        other => vec![other.clone()],
    }
}
fn reduce_numeric(args: &[Value], f: fn(f64, f64) -> f64) -> Result<Value, Value> {
    let nums: Vec<f64> = if args.len() == 1 {
        if let Value::List(l) = &args[0] { l.borrow().iter().map(|v| num(v).unwrap_or(0.0)).collect() } else { vec![num(&args[0])?] }
    } else { let mut v = Vec::new(); for a in args { v.push(num(a)?); } v };
    if nums.is_empty() { return Err(rt_err("min()/max() require at least one value")); }
    let mut acc = nums[0];
    for n in &nums[1..] { acc = f(acc, *n); }
    Ok(Value::Float(acc))
}

/// Like `value_eq`, but used for every user-written `==`/`!=` (via
/// `EqUser`/`NotEqUser`) and `match` arms. Dicts and instances don't have a
/// sensible default notion of equality — silently returning `false` for
/// `some_dict == other_dict` reads as "these are different" when really
/// it's "K doesn't know how to compare these," which is a much easier bug
/// to miss. Comparing anything else (numbers, strings, bools, null, lists)
/// still works exactly like `==` always has.
fn checked_eq(a: &Value, b: &Value) -> Result<bool, Value> {
    match (a, b) {
        (Value::Dict(_), _) | (_, Value::Dict(_)) => Err(rt_err("'==' is not supported for dicts yet — compare specific keys instead, e.g. a[\"id\"] == b[\"id\"]")),
        (Value::Instance(_), _) | (_, Value::Instance(_)) => Err(rt_err("'==' is not supported for class instances — give the class an 'equals' method and call it explicitly")),
        _ => Ok(value_eq(a, b)),
    }
}

/// Howard Hinnant's `civil_from_days` (public domain): converts a day count
/// since 1970-01-01 into a proleptic-Gregorian (year, month, day). Pure
/// integer arithmetic, correct across the full range `i64` can represent —
/// used instead of a date/time crate so `date_string()` costs nothing in
/// binary size.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };
    (year, m, d)
}

// ---- JSON encode/decode: hand-rolled, no serde/serde_json dependency ----
// (keeps the binary small and avoids depending on a crate whose API we
// can't verify against in this environment). Standard recursive-descent
// parser; nothing exotic.

fn json_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn json_encode(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Str(s) => json_quote(s),
        Value::List(l) => format!("[{}]", l.borrow().iter().map(json_encode).collect::<Vec<_>>().join(",")),
        Value::Dict(d) => format!("{{{}}}", d.borrow().iter().map(|(k, v)| format!("{}:{}", json_quote(k), json_encode(v))).collect::<Vec<_>>().join(",")),
        // Same {"shape": [...], "data": [...]} shape save_weights() writes,
        // so json_encode(tensor) and save_weights(tensor, path) produce
        // mutually-readable output.
        Value::Tensor(t) => format!(
            "{{\"shape\":{},\"data\":{}}}",
            json_encode(&Value::List(Rc::new(RefCell::new(t.shape.iter().map(|d| Value::Int(*d as i64)).collect())))),
            json_encode(&Value::List(Rc::new(RefCell::new(t.data.iter().map(|x| Value::Float(*x)).collect())))),
        ),
        // Functions/classes/instances have no JSON form — best-effort
        // fallback so json_encode() never panics, just produces a string
        // that at least says what it was.
        other => json_quote(&to_display(other)),
    }
}

fn json_decode(s: &str) -> Result<Value, Value> {
    let chars: Vec<char> = s.chars().collect();
    let mut pos = 0usize;
    let v = json_parse_value(&chars, &mut pos).map_err(rt_err)?;
    json_skip_ws(&chars, &mut pos);
    if pos != chars.len() { return Err(rt_err("trailing characters after JSON value")); }
    Ok(v)
}
fn json_skip_ws(chars: &[char], pos: &mut usize) { while *pos < chars.len() && chars[*pos].is_whitespace() { *pos += 1; } }
fn json_expect_lit(chars: &[char], pos: &mut usize, lit: &str) -> Result<(), String> {
    for expected in lit.chars() {
        if *pos >= chars.len() || chars[*pos] != expected { return Err(format!("invalid JSON literal, expected '{}'", lit)); }
        *pos += 1;
    }
    Ok(())
}
fn json_parse_value(chars: &[char], pos: &mut usize) -> Result<Value, String> {
    json_skip_ws(chars, pos);
    if *pos >= chars.len() { return Err("unexpected end of JSON input".into()); }
    match chars[*pos] {
        '"' => json_parse_string(chars, pos).map(|s| Value::Str(Rc::new(s))),
        '{' => json_parse_object(chars, pos),
        '[' => json_parse_array(chars, pos),
        't' => { json_expect_lit(chars, pos, "true")?; Ok(Value::Bool(true)) }
        'f' => { json_expect_lit(chars, pos, "false")?; Ok(Value::Bool(false)) }
        'n' => { json_expect_lit(chars, pos, "null")?; Ok(Value::Null) }
        c if c == '-' || c.is_ascii_digit() => json_parse_number(chars, pos),
        other => Err(format!("unexpected character '{}' in JSON", other)),
    }
}
fn json_parse_string(chars: &[char], pos: &mut usize) -> Result<String, String> {
    *pos += 1; // opening quote
    let mut s = String::new();
    while *pos < chars.len() && chars[*pos] != '"' {
        if chars[*pos] == '\\' {
            *pos += 1;
            if *pos >= chars.len() { return Err("unterminated escape in JSON string".into()); }
            match chars[*pos] {
                '"' => s.push('"'), '\\' => s.push('\\'), '/' => s.push('/'),
                'n' => s.push('\n'), 't' => s.push('\t'), 'r' => s.push('\r'),
                'b' => s.push('\u{8}'), 'f' => s.push('\u{c}'),
                'u' => {
                    if *pos + 4 >= chars.len() { return Err("invalid \\u escape in JSON string".into()); }
                    let hex: String = chars[*pos + 1..*pos + 5].iter().collect();
                    let code = u32::from_str_radix(&hex, 16).map_err(|_| "invalid \\u escape in JSON string".to_string())?;
                    if let Some(c) = char::from_u32(code) { s.push(c); }
                    *pos += 4;
                }
                other => return Err(format!("invalid escape '\\{}' in JSON string", other)),
            }
            *pos += 1;
        } else {
            s.push(chars[*pos]);
            *pos += 1;
        }
    }
    if *pos >= chars.len() { return Err("unterminated JSON string".into()); }
    *pos += 1; // closing quote
    Ok(s)
}
fn json_parse_number(chars: &[char], pos: &mut usize) -> Result<Value, String> {
    let start = *pos;
    if chars[*pos] == '-' { *pos += 1; }
    while *pos < chars.len() && chars[*pos].is_ascii_digit() { *pos += 1; }
    let mut is_float = false;
    if *pos < chars.len() && chars[*pos] == '.' {
        is_float = true;
        *pos += 1;
        while *pos < chars.len() && chars[*pos].is_ascii_digit() { *pos += 1; }
    }
    if *pos < chars.len() && (chars[*pos] == 'e' || chars[*pos] == 'E') {
        is_float = true;
        *pos += 1;
        if *pos < chars.len() && (chars[*pos] == '+' || chars[*pos] == '-') { *pos += 1; }
        while *pos < chars.len() && chars[*pos].is_ascii_digit() { *pos += 1; }
    }
    let text: String = chars[start..*pos].iter().collect();
    if is_float { text.parse::<f64>().map(Value::Float).map_err(|_| format!("invalid JSON number '{}'", text)) }
    else { text.parse::<i64>().map(Value::Int).map_err(|_| format!("invalid JSON number '{}'", text)) }
}
fn json_parse_array(chars: &[char], pos: &mut usize) -> Result<Value, String> {
    *pos += 1; // '['
    let mut items = Vec::new();
    json_skip_ws(chars, pos);
    if *pos < chars.len() && chars[*pos] == ']' { *pos += 1; return Ok(Value::List(Rc::new(RefCell::new(items)))); }
    loop {
        items.push(json_parse_value(chars, pos)?);
        json_skip_ws(chars, pos);
        if *pos < chars.len() && chars[*pos] == ',' { *pos += 1; json_skip_ws(chars, pos); continue; }
        break;
    }
    json_skip_ws(chars, pos);
    if *pos >= chars.len() || chars[*pos] != ']' { return Err("expected ']' to close JSON array".into()); }
    *pos += 1;
    Ok(Value::List(Rc::new(RefCell::new(items))))
}
fn json_parse_object(chars: &[char], pos: &mut usize) -> Result<Value, String> {
    *pos += 1; // '{'
    let mut map = OrderedMap::new();
    json_skip_ws(chars, pos);
    if *pos < chars.len() && chars[*pos] == '}' { *pos += 1; return Ok(Value::Dict(Rc::new(RefCell::new(map)))); }
    loop {
        json_skip_ws(chars, pos);
        if *pos >= chars.len() || chars[*pos] != '"' { return Err("expected string key in JSON object".into()); }
        let key = json_parse_string(chars, pos)?;
        json_skip_ws(chars, pos);
        if *pos >= chars.len() || chars[*pos] != ':' { return Err("expected ':' after key in JSON object".into()); }
        *pos += 1;
        let value = json_parse_value(chars, pos)?;
        map.insert(key, value);
        json_skip_ws(chars, pos);
        if *pos < chars.len() && chars[*pos] == ',' { *pos += 1; continue; }
        break;
    }
    json_skip_ws(chars, pos);
    if *pos >= chars.len() || chars[*pos] != '}' { return Err("expected '}' to close JSON object".into()); }
    *pos += 1;
    Ok(Value::Dict(Rc::new(RefCell::new(map))))
}
