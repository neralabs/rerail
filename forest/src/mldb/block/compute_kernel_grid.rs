//! compute_kernel_grid.cc (and the parts of compute_kernel_grid.h it defines), in Rust (in progress: what is here
//! has been cut from the .cc, the rest is still there).
//!
//! Jeremy Barnes, 21 July 2021. Copyright (c) 2021 Jeremy Barnes. This file is part of MLDB. Copyright 2016 mldb.ai
//! inc. Apache-2.0 (see the LICENSE at `forest/src/mldb/`).
//!
//! Grid compute kernels: a kernel registered with its dimensions, parameters, tuneables, constraints and grid
//! expressions (a `GridComputeKernelTemplate`), specialized against a compute function's reflected arguments into
//! bind actions (`GridComputeKernelSpecialization`), and launched (`GridComputeQueue::enqueue`): the knowns (the
//! grid, the tuneables, the scalar arguments, and what the constraints solve to) give the grid and block sizes, and
//! each bind action sets its function argument (a buffer, an `args` structure assembled field by field, or
//! threadgroup memory) on the bind context, which dispatches.
//!
//! Here so far: the template (with `ComputeKernel`'s `addDimension`/`addParameter`/`addConstraint` from
//! compute_kernel.cc as its builders), the bind actions and their application, the specialization, and
//! `enqueue`. MLDB's `CommandExpression` (not copied) is `Expr`, the argument expressions the registrations use
//! (integers, names, `+ - * /`, calls, lists, field access), and the knowns are a map of `Value`s, not JSON.
//! The constraint solver solves `name == expression` both ways until nothing more is learnt; a constraint over
//! device memory (`readArrayElement`) is left unsolved, where upstream read the array back. Memory regions, the
//! hazard tracking, events, tracing and the multi-device context are not brought over: a buffer argument is a
//! Metal buffer.

use crate::compute_kernel_metal::{ComputeKernelType, GridComputeFunctionArgumentDisposition, KernelBaseType, MetalComputeFunction};
use metal::BufferRef;
use std::collections::BTreeMap;

/// A value an expression evaluates to, or a known holds.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    List(Vec<Value>),
}

impl Value {
    pub fn as_i64(&self) -> Result<i64, String> {
        match self {
            Value::Int(i) => Ok(*i),
            Value::Float(f) if f.fract() == 0.0 => Ok(*f as i64),
            v => Err(format!("{v:?} is not an integer")),
        }
    }

    pub fn as_f64(&self) -> Result<f64, String> {
        match self {
            Value::Int(i) => Ok(*i as f64),
            Value::Float(f) => Ok(*f),
            v => Err(format!("{v:?} is not a number")),
        }
    }

    pub fn as_sizes(&self) -> Result<Vec<u64>, String> {
        match self {
            Value::List(vs) => vs.iter().map(|v| v.as_i64().map(|i| i as u64)).collect(),
            v => Ok(vec![v.as_i64()? as u64]),
        }
    }
}

/// An argument expression (`CommandExpression::parseArgumentExpression`).
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Int(i64),
    Name(String),
    Bin(char, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
    List(Vec<Expr>),
    Field(Box<Expr>, String),
}

impl Expr {
    pub fn parse(src: &str) -> Result<Expr, String> {
        let toks = tokenize(src)?;
        let mut p = Parser { toks, pos: 0 };
        let e = p.expr()?;
        if p.pos != p.toks.len() {
            return Err(format!("{src}: unexpected {:?}", p.toks[p.pos]));
        }
        Ok(e)
    }

    pub fn evaluate(&self, knowns: &Knowns) -> Result<Value, String> {
        Ok(match self {
            Expr::Int(i) => Value::Int(*i),
            Expr::Name(n) => knowns.get(n).cloned().ok_or_else(|| format!("unknown: {n}"))?,
            Expr::Bin(op, a, b) => {
                let (a, b) = (a.evaluate(knowns)?, b.evaluate(knowns)?);
                match (&a, &b) {
                    (Value::Int(x), Value::Int(y)) => Value::Int(match op {
                        '+' => x + y,
                        '-' => x - y,
                        '*' => x * y,
                        '/' if *y != 0 => x / y,
                        _ => return Err(format!("{x} {op} {y}")),
                    }),
                    _ => {
                        let (x, y) = (a.as_f64()?, b.as_f64()?);
                        Value::Float(match op {
                            '+' => x + y,
                            '-' => x - y,
                            '*' => x * y,
                            _ => x / y,
                        })
                    }
                }
            }
            Expr::Call(f, args) => {
                let vs = args.iter().map(|a| a.evaluate(knowns)).collect::<Result<Vec<_>, _>>()?;
                let ints = || vs.iter().map(Value::as_i64).collect::<Result<Vec<_>, _>>();
                match (f.as_str(), vs.len()) {
                    ("ceilDiv", 2) => {
                        let v = ints()?;
                        Value::Int((v[0] + v[1] - 1) / v[1])
                    }
                    ("min", 2) => Value::Int(ints()?.into_iter().min().unwrap()),
                    ("max", 2) => Value::Int(ints()?.into_iter().max().unwrap()),
                    _ => return Err(format!("cannot evaluate {f}({} arguments) here", vs.len())),
                }
            }
            Expr::List(es) => Value::List(es.iter().map(|e| e.evaluate(knowns)).collect::<Result<_, _>>()?),
            Expr::Field(_, f) => return Err(format!("cannot evaluate field {f} here")),
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Int(i64),
    Name(String),
    P(char),
}

fn tokenize(src: &str) -> Result<Vec<Tok>, String> {
    let cs: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < cs.len() {
        let c = cs[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() {
            let s = i;
            while i < cs.len() && cs[i].is_ascii_digit() {
                i += 1;
            }
            out.push(Tok::Int(cs[s..i].iter().collect::<String>().parse().unwrap()));
        } else if c.is_alphabetic() || c == '_' {
            let s = i;
            while i < cs.len() && (cs[i].is_alphanumeric() || cs[i] == '_') {
                i += 1;
            }
            out.push(Tok::Name(cs[s..i].iter().collect()));
        } else if "+-*/()[],.".contains(c) {
            out.push(Tok::P(c));
            i += 1;
        } else {
            return Err(format!("{src}: unexpected '{c}'"));
        }
    }
    Ok(out)
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }
    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(&Tok::P(c)) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, c: char) -> Result<(), String> {
        if self.eat(c) { Ok(()) } else { Err(format!("expected '{c}'")) }
    }
    fn expr(&mut self) -> Result<Expr, String> {
        let mut e = self.term()?;
        loop {
            let op = if self.eat('+') {
                '+'
            } else if self.eat('-') {
                '-'
            } else {
                return Ok(e);
            };
            e = Expr::Bin(op, Box::new(e), Box::new(self.term()?));
        }
    }
    fn term(&mut self) -> Result<Expr, String> {
        let mut e = self.factor()?;
        loop {
            let op = if self.eat('*') {
                '*'
            } else if self.eat('/') {
                '/'
            } else {
                return Ok(e);
            };
            e = Expr::Bin(op, Box::new(e), Box::new(self.factor()?));
        }
    }
    fn list(&mut self, close: char) -> Result<Vec<Expr>, String> {
        let mut es = Vec::new();
        if self.eat(close) {
            return Ok(es);
        }
        loop {
            es.push(self.expr()?);
            if self.eat(close) {
                return Ok(es);
            }
            self.expect(',')?;
        }
    }
    fn factor(&mut self) -> Result<Expr, String> {
        let mut e = match self.toks.get(self.pos).cloned() {
            Some(Tok::Int(i)) => {
                self.pos += 1;
                Expr::Int(i)
            }
            Some(Tok::Name(n)) => {
                self.pos += 1;
                if self.eat('(') { Expr::Call(n, self.list(')')?) } else { Expr::Name(n) }
            }
            Some(Tok::P('(')) => {
                self.pos += 1;
                let e = self.expr()?;
                self.expect(')')?;
                e
            }
            Some(Tok::P('[')) => {
                self.pos += 1;
                Expr::List(self.list(']')?)
            }
            t => return Err(format!("unexpected {t:?}")),
        };
        while self.eat('.') {
            match self.toks.get(self.pos).cloned() {
                Some(Tok::Name(f)) => {
                    self.pos += 1;
                    e = Expr::Field(Box::new(e), f);
                }
                t => return Err(format!("expected a field name, got {t:?}")),
            }
        }
        Ok(e)
    }
}

/// The knowns of a launch (`ComputeKernelConstraintSolution`'s values).
pub type Knowns = BTreeMap<String, Value>;

/// A parameter's declared type: its base type's name and its array bound, if an array (`u32[nf + 1]`,
/// `TreeTrainingInfo[=1]`, `UpdateWorkEntry[nndi:unordered]`).
#[derive(Clone, Debug, PartialEq)]
pub struct ParamType {
    pub base: String,
    pub bound: Option<Expr>,
}

impl ParamType {
    pub fn parse(s: &str) -> Result<ParamType, String> {
        match s.find('[') {
            None => Ok(ParamType { base: s.trim().to_string(), bound: None }),
            Some(i) => {
                let inner = s[i + 1..].strip_suffix(']').ok_or_else(|| format!("{s}: no closing ']'"))?;
                let inner = inner.trim().trim_start_matches('=');
                let inner = inner.split(':').next().unwrap_or(inner);
                Ok(ParamType { base: s[..i].trim().to_string(), bound: Some(Expr::parse(inner)?) })
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct KernelParameter {
    pub name: String,
    pub access: String,
    pub ty: ParamType,
}

#[derive(Clone, Debug)]
pub struct KernelDimension {
    pub name: String,
    pub range: String,
}

#[derive(Clone, Debug)]
pub struct KernelConstraint {
    pub lhs: String,
    pub op: String,
    pub rhs: Expr,
    pub description: String,
}

/// A kernel as registered (`GridComputeKernel`, `GridComputeKernelTemplate`).
#[derive(Clone, Debug, Default)]
pub struct GridComputeKernelTemplate {
    pub kernel_name: String,
    pub library_name: String,
    pub function_name: String,
    pub dims: Vec<KernelDimension>,
    pub params: Vec<KernelParameter>,
    pub tuneables: Vec<(String, i64)>,
    pub constraints: Vec<KernelConstraint>,
    pub pre_constraints: Vec<KernelConstraint>,
    pub post_constraints: Vec<KernelConstraint>,
    pub grid_expression: Option<Expr>,
    pub block_expression: Option<Expr>,
    pub allow_grid_padding_flag: bool,
    pub allow_grid_expansion_flag: bool,
}

impl GridComputeKernelTemplate {
    pub fn new(kernel_name: &str) -> Self {
        GridComputeKernelTemplate { kernel_name: kernel_name.to_string(), ..Default::default() }
    }

    pub fn add_dimension(&mut self, name: &str, range: &str) -> &mut Self {
        self.dims.push(KernelDimension { name: name.into(), range: range.into() });
        self
    }

    /// `addDimension(name, range, maxSize)`: the maximum size is not kept.
    pub fn add_dimension_max(&mut self, name: &str, range: &str, _max_size: u32) -> &mut Self {
        self.add_dimension(name, range)
    }

    pub fn add_parameter(&mut self, name: &str, access: &str, ty: &str) -> &mut Self {
        let ty = ParamType::parse(ty).unwrap_or_else(|e| panic!("kernel {}: parameter {name}: {e}", self.kernel_name));
        self.params.push(KernelParameter { name: name.into(), access: access.into(), ty });
        self
    }

    fn constraint(lhs: &str, op: &str, rhs: &str, description: &str) -> KernelConstraint {
        KernelConstraint { lhs: lhs.into(), op: op.into(), rhs: Expr::parse(rhs).unwrap_or_else(|e| panic!("constraint {lhs} {op} {rhs}: {e}")), description: description.into() }
    }

    pub fn add_constraint(&mut self, lhs: &str, op: &str, rhs: &str, description: &str) -> &mut Self {
        self.constraints.push(Self::constraint(lhs, op, rhs, description));
        self
    }

    pub fn add_pre_constraint(&mut self, lhs: &str, op: &str, rhs: &str, description: &str) -> &mut Self {
        self.pre_constraints.push(Self::constraint(lhs, op, rhs, description));
        self
    }

    pub fn add_post_constraint(&mut self, lhs: &str, op: &str, rhs: &str, description: &str) -> &mut Self {
        self.post_constraints.push(Self::constraint(lhs, op, rhs, description));
        self
    }

    pub fn allow_grid_padding(&mut self) -> &mut Self {
        self.allow_grid_padding_flag = true;
        self
    }

    pub fn allow_grid_expansion(&mut self) -> &mut Self {
        self.allow_grid_expansion_flag = true;
        self
    }

    pub fn set_grid_expression(&mut self, grid_expr: &str, block_expr: &str) -> &mut Self {
        if !grid_expr.is_empty() {
            self.grid_expression = Some(Expr::parse(grid_expr).unwrap_or_else(|e| panic!("{e}")));
        }
        if !block_expr.is_empty() {
            self.block_expression = Some(Expr::parse(block_expr).unwrap_or_else(|e| panic!("{e}")));
        }
        self
    }

    pub fn add_tuneable(&mut self, name: &str, default_value: i64) -> &mut Self {
        self.tuneables.push((name.into(), default_value));
        self
    }

    pub fn set_compute_function(&mut self, library_name: &str, kernel_name: &str) -> &mut Self {
        self.library_name = library_name.into();
        self.function_name = kernel_name.into();
        self
    }

    fn param_index(&self, name: &str) -> Option<usize> {
        self.params.iter().position(|p| p.name == name)
    }
}

/// How an `args` structure's field is filled.
#[derive(Clone, Debug, PartialEq)]
pub enum GridBindFieldActionType {
    SetFieldFromParam(usize),
    SetFieldFromKnown,
}

#[derive(Clone, Debug)]
pub struct GridBindFieldAction {
    pub action: GridBindFieldActionType,
    pub field_name: String,
    pub offset: u64,
    pub ty: KernelBaseType,
    pub expr: Expr,
}

#[derive(Clone, Debug)]
pub enum GridBindActionType {
    SetFromArg(usize),
    SetFromStruct(Vec<GridBindFieldAction>),
    SetFromKnown(Expr),
    SetThreadGroup(Expr),
}

#[derive(Clone, Debug)]
pub struct GridBindAction {
    pub action: GridBindActionType,
    pub arg_name: String,
    pub ty: ComputeKernelType,
    pub compute_function_arg_index: u64,
}

/// A kernel's argument at launch (`ComputeKernelArgument`): device memory, or a value.
#[derive(Clone, Copy, Debug)]
pub enum ComputeKernelArgument<'a> {
    Buffer(&'a BufferRef),
    Int(i64),
    Float(f64),
}

/// What the bind actions set their arguments on (`GridBindContext`).
pub trait GridBindContext {
    fn set_primitive(&mut self, arg_num: u64, bytes: &[u8]);
    fn set_buffer(&mut self, arg_num: u64, buffer: &BufferRef);
    fn set_thread_group_memory(&mut self, arg_num: u64, n_bytes: u64);
    fn launch(&mut self, grid: &[u64], block: &[u64]) -> Result<(), String>;
}

/// Writes a scalar into its field's bytes, as the field's type is.
fn write_scalar(bytes: &mut [u8], ty: &KernelBaseType, v: &Value, what: &str) -> Result<(), String> {
    let KernelBaseType::Basic { ty: basic, simd } = ty else {
        return Err(format!("{what}: only basic fields can be filled ({ty:?})"));
    };
    if simd.iter().product::<u32>() != 1 {
        return Err(format!("{what}: vector fields cannot be filled"));
    }
    let w = basic.width as usize;
    let b = &mut bytes[..w];
    match basic.name {
        "f32" => b.copy_from_slice(&(v.as_f64()? as f32).to_le_bytes()),
        "f16" => return Err(format!("{what}: half fields cannot be filled")),
        "bool" => b[0] = (v.as_i64()? != 0) as u8,
        _ => b.copy_from_slice(&v.as_i64()?.to_le_bytes()[..w]),
    }
    Ok(())
}

fn argument_value(arg: &ComputeKernelArgument) -> Option<Value> {
    match arg {
        ComputeKernelArgument::Int(i) => Some(Value::Int(*i)),
        ComputeKernelArgument::Float(f) => Some(Value::Float(*f)),
        ComputeKernelArgument::Buffer(_) => None,
    }
}

impl GridBindAction {
    pub fn apply(&self, args: &[Option<ComputeKernelArgument>], knowns: &Knowns, ctx: &mut dyn GridBindContext) -> Result<(), String> {
        match &self.action {
            GridBindActionType::SetFromArg(arg_num) => match args.get(*arg_num).copied().flatten() {
                Some(ComputeKernelArgument::Buffer(b)) => ctx.set_buffer(self.compute_function_arg_index, b),
                Some(a) => {
                    let mut bytes = vec![0u8; self.ty.base_type.width() as usize];
                    write_scalar(&mut bytes, &self.ty.base_type, &argument_value(&a).unwrap(), &self.arg_name)?;
                    ctx.set_primitive(self.compute_function_arg_index, &bytes);
                }
                None => return Err(format!("argument {arg_num} ({}) was not passed", self.arg_name)),
            },
            GridBindActionType::SetFromKnown(expr) => {
                let mut bytes = vec![0u8; self.ty.base_type.width() as usize];
                write_scalar(&mut bytes, &self.ty.base_type, &expr.evaluate(knowns)?, &self.arg_name)?;
                ctx.set_primitive(self.compute_function_arg_index, &bytes);
            }
            GridBindActionType::SetFromStruct(fields) => {
                let mut bytes = vec![0u8; self.ty.base_type.width() as usize];
                for field in fields {
                    let value = match &field.action {
                        GridBindFieldActionType::SetFieldFromParam(arg_num) => match args.get(*arg_num).copied().flatten().as_ref().and_then(argument_value) {
                            Some(v) => v,
                            None => field.expr.evaluate(knowns)?,
                        },
                        GridBindFieldActionType::SetFieldFromKnown => field.expr.evaluate(knowns).map_err(|e| format!("{}.{}: {e}", self.arg_name, field.field_name))?,
                    };
                    write_scalar(&mut bytes[field.offset as usize..], &field.ty, &value, &format!("{}.{}", self.arg_name, field.field_name))?;
                }
                ctx.set_primitive(self.compute_function_arg_index, &bytes);
            }
            GridBindActionType::SetThreadGroup(expr) => {
                let len = expr.evaluate(knowns)?.as_i64()? as u64;
                let n_bytes = (len * self.ty.base_type.width()).div_ceil(16) * 16;
                ctx.set_thread_group_memory(self.compute_function_arg_index, n_bytes);
            }
        }
        Ok(())
    }
}

/// A template specialized against its compute function's arguments.
pub struct GridComputeKernelSpecialization {
    pub kernel: GridComputeKernelTemplate,
    pub function: &'static MetalComputeFunction,
    pub bind_actions: Vec<GridBindAction>,
}

impl GridComputeKernelSpecialization {
    pub fn new(tmplate: &GridComputeKernelTemplate, function: &'static MetalComputeFunction) -> Result<Self, String> {
        let kernel_name = &tmplate.kernel_name;
        let mut actions = Vec::new();
        for (i, arg) in function.get_argument_info().iter().enumerate() {
            let arg_name = &arg.name;
            let ty = arg.ty.clone();
            let it = tmplate.param_index(arg_name);
            let action = match arg.disposition {
                GridComputeFunctionArgumentDisposition::Threadgroup => {
                    let arg_num = it.ok_or_else(|| format!("Grid thread group kernel parameter {arg_name} to kernel {kernel_name} has no corresponding argument to learn its length from"))?;
                    let bound = tmplate.params[arg_num].ty.bound.clone().ok_or_else(|| format!("{kernel_name}: thread group parameter {arg_name} has no length"))?;
                    GridBindActionType::SetThreadGroup(bound)
                }
                GridComputeFunctionArgumentDisposition::Buffer => match it {
                    Some(arg_num) => GridBindActionType::SetFromArg(arg_num),
                    None => {
                        let KernelBaseType::Struct { fields, .. } = &ty.base_type else {
                            return Err(format!("Kernel parameter {i} ({arg_name}) to Grid kernel {kernel_name} has no counterpart in formal parameter list"));
                        };
                        if ty.dims != 0 {
                            return Err(format!("Kernel parameter {i} ({arg_name}) to Grid kernel {kernel_name} has no counterpart in formal parameter list"));
                        }
                        let field_actions = fields
                            .iter()
                            .map(|f| {
                                let action = match tmplate.param_index(&f.name) {
                                    Some(n) => GridBindFieldActionType::SetFieldFromParam(n),
                                    None => GridBindFieldActionType::SetFieldFromKnown,
                                };
                                GridBindFieldAction { action, field_name: f.name.clone(), offset: f.offset, ty: f.ty.clone(), expr: Expr::Name(f.name.clone()) }
                            })
                            .collect();
                        GridBindActionType::SetFromStruct(field_actions)
                    }
                },
            };
            actions.push(GridBindAction { action, arg_name: arg_name.clone(), ty, compute_function_arg_index: arg.compute_function_arg_index });
        }
        Ok(GridComputeKernelSpecialization { kernel: tmplate.clone(), function, bind_actions: actions })
    }
}

/// Solves `name == expression` constraints both ways (a name on the right alone) until nothing more is learnt.
pub fn solve(knowns: &mut Knowns, constraints: &[&KernelConstraint]) {
    loop {
        let mut learnt = false;
        for c in constraints {
            if c.op != "==" {
                continue;
            }
            if !knowns.contains_key(&c.lhs) {
                if let Ok(v) = c.rhs.evaluate(knowns) {
                    knowns.insert(c.lhs.clone(), v);
                    learnt = true;
                }
            } else if let Expr::Name(n) = &c.rhs {
                if !knowns.contains_key(n) {
                    let v = knowns[&c.lhs].clone();
                    knowns.insert(n.clone(), v);
                    learnt = true;
                }
            }
        }
        if !learnt {
            return;
        }
    }
}

impl GridComputeKernelSpecialization {
    /// `bind` and `GridComputeQueue::enqueue`: binds the arguments (by parameter name) and launches over `grid`
    /// (one range per dimension) on the bind context.
    pub fn enqueue(&self, named: &[(&str, ComputeKernelArgument)], grid: &[u32], ctx: &mut dyn GridBindContext) -> Result<(), String> {
        let (args, knowns, mtl_grid, mtl_block) = self.resolve(named, grid)?;
        let err = |e: String| format!("Error launching Grid kernel {}: {e}", self.kernel.kernel_name);
        for action in &self.bind_actions {
            action.apply(&args, &knowns, ctx).map_err(err)?;
        }
        ctx.launch(&mtl_grid, &mtl_block).map_err(err)
    }

    /// A launch resolved once and replayed (`GridLaunchPlan`): the solve, the grid and every value argument's
    /// bytes, for these arguments. A replay binds only the buffers, which may change from launch to launch; the
    /// values may not (a launch with other values needs its own plan).
    pub fn plan(&self, named: &[(&str, ComputeKernelArgument)], grid: &[u32]) -> Result<GridLaunchPlan, String> {
        let (args, knowns, mtl_grid, mtl_block) = self.resolve(named, grid)?;
        let err = |e: String| format!("Error planning Grid kernel {}: {e}", self.kernel.kernel_name);
        let mut rec = RecordingBindContext::default();
        let mut buffers = Vec::new();
        for action in &self.bind_actions {
            match (&action.action, ) {
                (GridBindActionType::SetFromArg(arg_num), ) if matches!(args.get(*arg_num).copied().flatten(), Some(ComputeKernelArgument::Buffer(_))) => {
                    buffers.push((action.compute_function_arg_index, self.kernel.params[*arg_num].name.clone()));
                }
                _ => action.apply(&args, &knowns, &mut rec).map_err(err)?,
            }
        }
        Ok(GridLaunchPlan { primitives: rec.primitives, thread_groups: rec.thread_groups, buffers, grid: mtl_grid, block: mtl_block })
    }

    /// Replays a plan with these buffers (each of the plan's buffer arguments by name).
    pub fn launch_plan(&self, plan: &GridLaunchPlan, buffers: &[(&str, &BufferRef)], ctx: &mut dyn GridBindContext) -> Result<(), String> {
        for (i, bytes) in &plan.primitives {
            ctx.set_primitive(*i, bytes);
        }
        for &(i, n) in &plan.thread_groups {
            ctx.set_thread_group_memory(i, n);
        }
        for (i, name) in &plan.buffers {
            let b = buffers.iter().find(|(n, _)| n == name).ok_or_else(|| format!("Grid kernel {}: no buffer {name} for the plan", self.kernel.kernel_name))?.1;
            ctx.set_buffer(*i, b);
        }
        ctx.launch(&plan.grid, &plan.block)
    }

    /// The arguments in parameter order, the knowns solved from them, and the launch's grid and block.
    #[allow(clippy::type_complexity)]
    fn resolve<'b>(&self, named: &[(&str, ComputeKernelArgument<'b>)], grid: &[u32]) -> Result<(Vec<Option<ComputeKernelArgument<'b>>>, Knowns, Vec<u64>, Vec<u64>), String> {
        let kernel = &self.kernel;
        let err = |e: String| format!("Error launching Grid kernel {}: {e}", kernel.kernel_name);
        let mut args: Vec<Option<ComputeKernelArgument>> = vec![None; kernel.params.len()];
        for (name, a) in named {
            let i = kernel.param_index(name).ok_or_else(|| err(format!("no parameter {name}")))?;
            args[i] = Some(*a);
        }
        let mut knowns = Knowns::new();
        for (name, v) in &kernel.tuneables {
            knowns.insert(name.clone(), Value::Int(*v));
        }
        for (p, a) in kernel.params.iter().zip(&args) {
            if let Some(v) = a.as_ref().and_then(argument_value) {
                knowns.insert(p.name.clone(), v);
            }
        }
        for (i, &g) in grid.iter().enumerate() {
            let dim = kernel.dims.get(i).ok_or_else(|| err(format!("grid has {} dimensions, kernel {}", grid.len(), kernel.dims.len())))?;
            knowns.insert(dim.range.clone(), Value::Int(g as i64));
        }
        knowns.insert("grid".into(), Value::List(grid.iter().map(|&g| Value::Int(g as i64)).collect()));
        let constraints: Vec<&KernelConstraint> = kernel.pre_constraints.iter().chain(&kernel.constraints).collect();
        solve(&mut knowns, &constraints);

        let mut mtl_grid: Vec<u64> = Vec::new();
        let mut mtl_block: Vec<u64> = Vec::new();
        if let Some(e) = &kernel.grid_expression {
            mtl_grid = e.evaluate(&knowns).map_err(err)?.as_sizes().map_err(err)?;
        }
        if let Some(e) = &kernel.block_expression {
            mtl_block = e.evaluate(&knowns).map_err(err)?.as_sizes().map_err(err)?;
        }
        if mtl_grid.is_empty() {
            mtl_grid.push(1);
        }
        if mtl_block.is_empty() {
            mtl_block.push(1);
        }
        Ok((args, knowns, mtl_grid, mtl_block))
    }
}

/// A resolved launch (`GridComputeKernelSpecialization::plan`): the value arguments' bytes and threadgroup memory
/// by argument index, the buffer arguments' indexes by name, and the grid.
#[derive(Clone, Debug, Default)]
pub struct GridLaunchPlan {
    primitives: Vec<(u64, Vec<u8>)>,
    thread_groups: Vec<(u64, u64)>,
    buffers: Vec<(u64, String)>,
    grid: Vec<u64>,
    block: Vec<u64>,
}

/// Records what the bind actions set (for a plan).
#[derive(Default)]
struct RecordingBindContext {
    primitives: Vec<(u64, Vec<u8>)>,
    thread_groups: Vec<(u64, u64)>,
}

impl GridBindContext for RecordingBindContext {
    fn set_primitive(&mut self, arg_num: u64, bytes: &[u8]) {
        self.primitives.push((arg_num, bytes.to_vec()));
    }
    fn set_buffer(&mut self, _arg_num: u64, _buffer: &BufferRef) {}
    fn set_thread_group_memory(&mut self, arg_num: u64, n_bytes: u64) {
        self.thread_groups.push((arg_num, n_bytes));
    }
    fn launch(&mut self, _grid: &[u64], _block: &[u64]) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_expressions_and_the_solver() {
        let mut k = Knowns::new();
        k.insert("numActiveBuckets".into(), Value::Int(130));
        k.insert("gridBlockSize".into(), Value::Int(64));
        k.insert("numPartitionsAtOnce".into(), Value::Int(256));
        let g = Expr::parse("[numPartitionsAtOnce,ceilDiv(numActiveBuckets,gridBlockSize)]").unwrap();
        assert_eq!(g.evaluate(&k).unwrap().as_sizes().unwrap(), vec![256, 3]);
        assert_eq!(Expr::parse("(numActiveBuckets+31)/32 * 2").unwrap().evaluate(&k).unwrap(), Value::Int(10));
        assert!(Expr::parse("readArrayElement(treeDepthInfo, 0).numActivePartitions").unwrap().evaluate(&k).is_err());

        let t = ParamType::parse("UpdateWorkEntry[nndi:unordered]").unwrap();
        assert_eq!((t.base.as_str(), t.bound), ("UpdateWorkEntry", Some(Expr::Name("nndi".into()))));
        assert_eq!(ParamType::parse("TreeTrainingInfo[=1]").unwrap().bound, Some(Expr::Int(1)));
        assert_eq!(ParamType::parse("u32").unwrap().bound, None);

        let mut tm = GridComputeKernelTemplate::new("updateBuckets");
        tm.add_constraint("naf_plus_1", "==", "numActiveFeatures + 1", "help the solver");
        tm.add_constraint("numActiveFeatures", "==", "naf_plus_1 - 1", "help the solver");
        let mut k = Knowns::new();
        k.insert("naf_plus_1".into(), Value::Int(5));
        solve(&mut k, &tm.constraints.iter().collect::<Vec<_>>());
        assert_eq!(k["numActiveFeatures"], Value::Int(4));
    }
}
