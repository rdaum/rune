use crate::arena::Arena;
use crate::error::Error;
use crate::object::*;
use crate::opcode::CodeVec;
use std::fmt;

use anyhow::{bail, Result};

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct FnArgs {
    pub rest: bool,
    pub required: u16,
    pub optional: u16,
    pub max_stack_usage: u16,
    pub advice: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LispFn<'ob> {
    pub body: Expression<'ob>,
    pub args: FnArgs,
}

impl FnArgs {
    pub fn num_of_fill_args(self, args: u16) -> Result<u16> {
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

impl<'ob> std::convert::TryFrom<crate::object::Object<'ob>> for &LispFn<'ob> {
    type Error = crate::error::Error;
    fn try_from(obj: crate::object::Object<'ob>) -> Result<Self, Self::Error> {
        match obj.val() {
            crate::object::Value::LispFn(x) => Ok(x),
            x => Err(crate::error::Error::Type(
                crate::error::Type::Func,
                x.get_type(),
            )),
        }
    }
}
impl<'ob> std::convert::TryFrom<crate::object::Object<'ob>> for Option<&LispFn<'ob>> {
    type Error = crate::error::Error;
    fn try_from(obj: crate::object::Object<'ob>) -> Result<Self, Self::Error> {
        match obj.val() {
            crate::object::Value::LispFn(x) => Ok(Some(x)),
            crate::object::Value::Nil => Ok(None),
            x => Err(crate::error::Error::Type(
                crate::error::Type::Func,
                x.get_type(),
            )),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Expression<'ob> {
    pub op_codes: CodeVec,
    pub constants: Vec<Object<'ob>>,
}

impl<'ob> LispFn<'ob> {
    pub fn new(
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

pub type BuiltInFn = for<'ob> fn(
    &[Object<'ob>],
    &mut crate::data::Environment<'ob>,
    &'ob Arena,
) -> Result<Object<'ob>>;

#[derive(Copy, Clone)]
pub struct SubrFn {
    pub subr: BuiltInFn,
    pub args: FnArgs,
    pub name: &'static str,
}
define_unbox_ref!(SubrFn, Func);

impl SubrFn {
    pub fn new(
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

impl std::cmp::PartialEq for SubrFn {
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
        let func = obj.as_lisp_fn().expect("expected lispfn");
        assert_eq!(func.body.op_codes, vec_into![0, 1, 2].into());
        assert_eq!(func.body.constants, vec_into_object![1; arena]);
        assert_eq!(func.args.required, 0);
        assert_eq!(func.args.optional, 0);
        assert!(!func.args.rest);
    }
}
