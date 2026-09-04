//! Bytecode format: a flat instruction stream + constant pool.
//! Replacing the AST tree-walk with this removes per-node recursive dispatch
//! and enum matching on boxed AST nodes; execution becomes a single flat
//! loop over bytes, which is what a real interpreter's hot path should be.

use crate::value::Value;

#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u8)]
pub enum OpCode {
    Constant,       // u16 const idx -> push constants[idx]
    Nil,
    True,
    False,
    Pop,
    GetLocal,       // u8 slot
    SetLocal,       // u8 slot
    GetGlobal,      // u16 name const idx
    DefineGlobal,   // u16 name const idx
    SetGlobal,      // u16 name const idx
    GetUpvalue,     // u8 index
    SetUpvalue,     // u8 index
    GetProperty,    // u16 name const idx
    SetProperty,    // u16 name const idx
    GetIndex,
    SetIndex,
    BuildList,      // u16 element count
    BuildDict,      // u16 pair count
    Equal,
    NotEqual,
    Greater,
    GreaterEqual,
    Less,
    LessEqual,
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
    Power,
    MatMul,
    Not,
    Negate,
    ToStr,          // pop value, push its display-string (for f-string parts)
    Jump,           // u16 forward offset
    JumpIfFalse,    // u16 forward offset; peeks, does not pop
    JumpIfFalsePop, // u16 forward offset; pops
    Loop,           // u16 backward offset
    Call,           // u8 arg count
    Closure,        // u16 function const idx, then per-upvalue: u8 is_local, u8 index
    Class,          // u16 name const idx
    Inherit,
    Method,         // u16 name const idx
    PushTry,        // u16 offset to catch handler
    PopTry,
    Throw,
    Len,            // pop list/dict/str, push its length as Int
    GetIterList,    // pop an iterable (list/str/dict), push a normalized List
    Instantiate,    // u8 arg count; pop args + class, construct + run init, push instance
    Invoke,         // u16 name const idx, u8 arg count; direct method-call dispatch
    Return,
    EqUser,         // like Equal, but for user-written '==': errors on dict/instance operands
                    // instead of silently returning false (Equal itself stays unchecked since
                    // the compiler also uses it internally for default-parameter detection).
    NotEqUser,      // same distinction, for user-written '!='
}

impl OpCode {
    pub fn from_u8(b: u8) -> OpCode {
        // Safety-free conversion: match explicitly so an out-of-range byte
        // is a clear panic during development rather than silent UB. The
        // compiler only ever emits valid opcodes, so this never fires on
        // bytecode produced by `Compiler`.
        use OpCode::*;
        const TABLE: &[OpCode] = &[
            Constant, Nil, True, False, Pop, GetLocal, SetLocal, GetGlobal, DefineGlobal, SetGlobal,
            GetUpvalue, SetUpvalue, GetProperty, SetProperty, GetIndex, SetIndex, BuildList, BuildDict,
            Equal, NotEqual, Greater, GreaterEqual, Less, LessEqual, Add, Subtract, Multiply, Divide,
            Modulo, Power, MatMul, Not, Negate, ToStr, Jump, JumpIfFalse, JumpIfFalsePop, Loop, Call,
            Closure, Class, Inherit, Method, PushTry, PopTry, Throw, Len, GetIterList, Instantiate, Invoke,
            Return, EqUser, NotEqUser,
        ];
        TABLE[b as usize]
    }
}

#[derive(Default, Clone)]
pub struct Chunk {
    pub code: Vec<u8>,
    pub constants: Vec<Value>,
}

impl Chunk {
    pub fn new() -> Self { Chunk::default() }

    pub fn emit_u8(&mut self, byte: u8) -> usize {
        self.code.push(byte);
        self.code.len() - 1
    }

    pub fn emit_op(&mut self, op: OpCode) -> usize { self.emit_u8(op as u8) }

    pub fn emit_u16(&mut self, val: u16) {
        self.code.push((val >> 8) as u8);
        self.code.push((val & 0xff) as u8);
    }

    pub fn patch_u16(&mut self, at: usize, val: u16) {
        self.code[at] = (val >> 8) as u8;
        self.code[at + 1] = (val & 0xff) as u8;
    }

    pub fn add_constant(&mut self, v: Value) -> u16 {
        self.constants.push(v);
        (self.constants.len() - 1) as u16
    }

    pub fn read_u16(&self, at: usize) -> u16 {
        ((self.code[at] as u16) << 8) | (self.code[at + 1] as u16)
    }
}
