use crate::arena::Arena;
use crate::error::Error;
use crate::object::{Function, IntoObject, Object, NIL};
use crate::opcode::CodeVec;
use crate::opcode::OpCode;
use std::fmt;

use anyhow::{bail, Result};

#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct FnArgs {
    pub(crate) rest: bool,
    pub(crate) required: u16,
    pub(crate) optional: u16,
    pub(crate) max_stack_usage: u16,
    pub(crate) advice: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LispFn<'ob> {
    pub(crate) body: Expression<'ob>,
    pub(crate) args: FnArgs,
}

impl FnArgs {
    pub(crate) fn num_of_fill_args(self, args: u16) -> Result<u16> {
        if args < self.required {
            bail!(Error::ArgCount(self.required, args));
        }
        let total_args = self.required + self.optional;
        if !self.rest && (args > total_args) {
            bail!(Error::ArgCount(total_args, args));
        }
        Ok(total_args.saturating_sub(args))
    }
}

define_unbox!(LispFn, Func, &LispFn<'ob>);

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Expression<'ob> {
    pub(crate) op_codes: CodeVec,
    pub(crate) constants: Vec<Object<'ob>>,
}

impl<'ob> LispFn<'ob> {
    pub(crate) fn new(
        op_codes: CodeVec,
        constants: Vec<Object<'ob>>,
        required: u16,
        optional: u16,
        rest: bool,
    ) -> Self {
        LispFn {
            body: Expression {
                op_codes,
                constants,
            },
            args: FnArgs {
                required,
                optional,
                rest,
                max_stack_usage: 0,
                advice: false,
            },
        }
    }
}

impl<'ob> IntoObject<'ob, Object<'ob>> for LispFn<'ob> {
    fn into_obj(self, arena: &'ob Arena) -> Object<'ob> {
        let x: Function = self.into_obj(arena);
        x.into()
    }
}

impl<'ob> Default for LispFn<'ob> {
    fn default() -> Self {
        LispFn::new(
            vec_into![OpCode::Constant0, OpCode::Ret].into(),
            vec![NIL],
            0,
            0,
            false,
        )
    }
}

pub(crate) type BuiltInFn = for<'ob> fn(
    &[Object<'ob>],
    &mut crate::data::Environment<'ob>,
    &'ob Arena,
) -> Result<Object<'ob>>;

#[derive(Copy, Clone)]
pub(crate) struct SubrFn {
    pub(crate) subr: BuiltInFn,
    pub(crate) args: FnArgs,
    pub(crate) name: &'static str,
}
define_unbox!(SubrFn, Func, &SubrFn);

#[cfg(test)]
impl SubrFn {
    pub(crate) fn new(
        name: &'static str,
        subr: BuiltInFn,
        required: u16,
        optional: u16,
        rest: bool,
    ) -> Self {
        Self {
            name,
            subr,
            args: FnArgs {
                required,
                optional,
                rest,
                max_stack_usage: 0,
                advice: false,
            },
        }
    }
}

impl std::fmt::Debug for SubrFn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({} -> {:?})", &self.name, self.args)
    }
}

impl PartialEq for SubrFn {
    fn eq(&self, other: &Self) -> bool {
        let lhs: fn(&'static _, &'static mut _, &'static _) -> _ = self.subr;
        lhs == other.subr
    }
}

impl<'ob> IntoObject<'ob, Object<'ob>> for SubrFn {
    fn into_obj(self, arena: &'ob Arena) -> Object<'ob> {
        let x: Function = self.into_obj(arena);
        x.into()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::object::Value;

    #[test]
    fn function() {
        let arena = &Arena::new();
        let constant: Object = 1.into_obj(arena);
        let func = LispFn::new(vec_into![0, 1, 2].into(), vec![constant], 0, 0, false);
        let obj: Object = func.into_obj(arena);
        assert!(matches!(obj.val(), Value::LispFn(_)));
        format!("{}", obj);
        let func = match obj.val() {
            Value::LispFn(x) => x,
            _ => unreachable!("expected lispfn"),
        };
        assert_eq!(func.body.op_codes, vec_into![0, 1, 2].into());
        assert_eq!(func.body.constants, vec_into_object![1; arena]);
        assert_eq!(func.args.required, 0);
        assert_eq!(func.args.optional, 0);
        assert!(!func.args.rest);
    }
}
