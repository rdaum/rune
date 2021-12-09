#![allow(dead_code)]
use crate::error::{Error, Type};
use crate::object::Callable;
use crate::symbol::sym;
use crate::{
    arena::Arena,
    cons::{Cons, ElemIter},
    data::Environment,
    object::Object,
    symbol::Symbol,
};
use anyhow::{anyhow, bail, ensure, Result};
use fn_macros::defun;

struct Interpreter<'ob, 'brw> {
    vars: Vec<&'ob Cons<'ob>>,
    env: &'brw mut Environment<'ob>,
    arena: &'ob Arena,
}

#[defun]
pub(crate) fn eval<'ob, 'brw>(
    form: Object<'ob>,
    lexical: Option<Object<'ob>>,
    env: &'brw mut Environment<'ob>,
    arena: &'ob Arena,
) -> Result<Object<'ob>> {
    ensure!(
        matches!(lexical, Some(Object::True(_) | Object::Nil(_)) | None),
        "lexical enviroments are not yet supported: found {:?}",
        lexical
    );
    let mut interpreter = Interpreter {
        vars: Vec::new(),
        env,
        arena,
    };
    interpreter.eval_form(form)
}

pub(crate) fn call<'ob, 'brw>(
    form: Object<'ob>,
    args: Vec<Object<'ob>>,
    env: &'brw mut Environment<'ob>,
    arena: &'ob Arena,
) -> Result<Object<'ob>> {
    let mut frame = Interpreter {
        vars: Vec::new(),
        env,
        arena,
    };
    frame.call_closure(form.try_into()?, args)
}

impl<'ob, 'brw> Interpreter<'ob, 'brw> {
    fn eval_form(&mut self, obj: Object<'ob>) -> Result<Object<'ob>> {
        match obj {
            Object::Symbol(sym) => self.var_ref(!sym),
            Object::Cons(cons) => self.eval_sexp(&cons),
            other => Ok(other),
        }
    }

    pub(crate) fn eval_sexp(&mut self, cons: &Cons<'ob>) -> Result<Object<'ob>> {
        let forms = cons.cdr();
        match cons.car() {
            Object::Symbol(sym) => symbol_match! {!sym;
                QUOTE => Self::quote(forms),
                LET => self.eval_let(forms, true),
                LET_STAR => self.eval_let(forms, false),
                IF => self.eval_if(forms),
                AND => self.eval_and(forms),
                OR => self.eval_or(forms),
                COND => self.eval_cond(forms),
                WHILE => self.eval_while(forms),
                PROGN => self.eval_progn(forms),
                PROG1 => self.eval_progx(forms, 1),
                PROG2 => self.eval_progx(forms, 2),
                SETQ => self.setq(forms),
                DEFVAR => self.defvar(forms),
                DEFCONST => self.defvar(forms),
                FUNCTION => self.eval_function(forms),
                @ func => self.eval_call(func, forms),
            },
            other => Err(anyhow!("Invalid Function: {}", other)),
        }
    }

    fn defvar(&mut self, obj: Object<'ob>) -> Result<Object<'ob>> {
        let mut forms = obj.as_list()?;

        match forms.next() {
            // (defvar x ...)
            Some(x) => {
                let name: Symbol = x?.try_into()?;
                let value = match forms.next() {
                    // (defvar x y)
                    Some(value) => self.eval_form(value?)?,
                    // (defvar x)
                    None => Object::NIL,
                };
                Ok(self.var_set(name, value))
            }
            // (defvar)
            None => Err(Error::ArgCount(1, 0).into()),
        }
    }

    fn parse_closure_env(obj: Object<'ob>) -> Result<Vec<&'ob Cons<'ob>>> {
        let forms = obj.as_list()?;
        let mut env = Vec::new();
        for form in forms {
            match form? {
                Object::Cons(pair) => {
                    env.push(!pair);
                }
                Object::True(_) => return Ok(env),
                x => bail!("Invalid closure environment member: {}", x),
            }
        }
        Err(anyhow!("Closure env did not end with `t`"))
    }

    fn parse_arg_list(bindings: Object) -> Result<(Vec<Symbol>, Vec<Symbol>, Option<Symbol>)> {
        let mut required = Vec::new();
        let mut optional = Vec::new();
        let mut rest = None;
        let mut arg_type = &mut required;
        let mut iter = bindings.as_list()?;
        while let Some(binding) = iter.next() {
            symbol_match! {
                binding?.try_into()?;
                AND_OPTIONAL => arg_type = &mut optional,
                AND_REST => {
                    if let Some(last) = iter.next() {
                        rest = Some(last?.try_into()?);
                        ensure!(
                            iter.next().is_none(),
                            "Found multiple arguments after &rest"
                        );
                    }
                },
                @ sym => {
                    arg_type.push(sym);
                }
            }
        }
        Ok((required, optional, rest))
    }

    fn bind_args(
        &self,
        arg_list: Object,
        args: Vec<Object<'ob>>,
        vars: &mut Vec<&'ob Cons<'ob>>,
    ) -> Result<()> {
        let (required, optional, rest) = Self::parse_arg_list(arg_list)?;

        let num_required_args = required.len() as u16;
        let num_optional_args = optional.len() as u16;
        let num_actual_args = args.len() as u16;
        // Ensure the minimum number of arguments is present
        ensure!(
            num_actual_args >= num_required_args,
            Error::ArgCount(num_required_args, num_actual_args)
        );

        let mut arg_values = args.into_iter();

        for name in required {
            let val = arg_values.next().unwrap();
            vars.push(cons!(name, val; self.arena).try_into().unwrap());
        }

        for name in optional {
            let val = arg_values.next().unwrap_or_default();
            vars.push(cons!(name, val; self.arena).try_into().unwrap());
        }

        if let Some(rest_name) = rest {
            let values = arg_values.as_slice();
            let list = crate::fns::slice_into_list(values, None, self.arena);
            vars.push(cons!(rest_name, list; self.arena).try_into().unwrap());
        } else {
            // Ensure too many args were not provided
            ensure!(
                arg_values.next().is_none(),
                Error::ArgCount(num_required_args + num_optional_args, num_actual_args)
            );
        }
        Ok(())
    }

    fn bind_variables(
        &self,
        forms: &mut ElemIter<'_, 'ob>,
        args: Vec<Object<'ob>>,
    ) -> Result<Vec<&'ob Cons<'ob>>> {
        // Add closure environment to variables
        // (closure ((x . 1) (y . 2) t) ...)
        //          ^^^^^^^^^^^^^^^^^^^
        let env = forms
            .next()
            .ok_or_else(|| anyhow!("Closure missing environment"))??;
        let mut vars = Self::parse_closure_env(env)?;

        // Add function arguments to variables
        // (closure (t) (x y &rest z) ...)
        //              ^^^^^^^^^^^^^
        let arg_list = forms
            .next()
            .ok_or_else(|| anyhow!("Closure missing argument list"))??;
        self.bind_args(arg_list, args, &mut vars)?;
        Ok(vars)
    }

    fn call_closure(
        &mut self,
        closure: &'ob Cons<'ob>,
        args: Vec<Object<'ob>>,
    ) -> Result<Object<'ob>> {
        match closure.car() {
            Object::Symbol(sym) if !sym == &sym::CLOSURE => {
                let mut forms = closure.cdr().as_list()?;
                let vars = self.bind_variables(&mut forms, args)?;

                let mut call_frame = Interpreter {
                    vars,
                    env: self.env,
                    arena: self.arena,
                };
                call_frame.implicit_progn(forms)
            }
            other => Err(Error::from_object(Type::Func, other).into()),
        }
    }

    fn eval_call(&mut self, name: Symbol, obj: Object<'ob>) -> Result<Object<'ob>> {
        use crate::bytecode;
        let func = match name.resolve_callable() {
            Some(x) => x,
            None => bail!("Invalid function: {}", name),
        };

        let mut eval_args =
            || -> Result<Vec<_>> { obj.as_list()?.map(|x| self.eval_form(x?)).collect() };

        match func {
            Callable::LispFn(func) => {
                let args = eval_args()?;
                bytecode::call_lisp(&func, args, self.env, self.arena)
            }
            Callable::SubrFn(func) => {
                let args = eval_args()?;
                if crate::debug::debug_enabled() {
                    println!("({} {:?})", name, args);
                }
                bytecode::call_subr(*func, args, self.env, self.arena)
            }
            Callable::Macro(mcro) => {
                let macro_args = obj.as_list()?.collect::<Result<Vec<_>>>()?;
                if crate::debug::debug_enabled() {
                    println!("(macro: {} {:?})", name, macro_args);
                }
                let value = mcro.get().call(macro_args, self.env, self.arena)?;
                self.eval_form(value)
            },
            Callable::Uncompiled(form) => match form.car() {
                Object::Symbol(sym) if !sym == &sym::CLOSURE => {
                    let args = eval_args()?;
                    if crate::debug::debug_enabled() {
                        println!("({} {:?})", name, args);
                    }
                    self.call_closure(!form, args)
                }
                other => Err(anyhow!("Invalid Function: {}", other)),
            },
        }
    }
    fn eval_function(&mut self, obj: Object<'ob>) -> Result<Object<'ob>> {
        let mut forms = obj.as_list()?;
        let len = forms.len() as u16;
        ensure!(len == 1, Error::ArgCount(1, len));

        match forms.next().unwrap()? {
            Object::Cons(cons) => {
                if cons.car() == (&sym::LAMBDA).into() {
                    let env = {
                        // TODO: remove temp vector
                        let env: Vec<_> =
                            self.vars.iter().map(|&x| Object::Cons(x.into())).collect();
                        crate::fns::slice_into_list(
                            env.as_slice(),
                            Some(cons!(true; self.arena)),
                            self.arena,
                        )
                    };
                    let end: Object = cons!(env, cons.cdr(); self.arena);
                    Ok(cons!(&sym::CLOSURE, end; self.arena))
                } else {
                    Ok(Object::Cons(cons))
                }
            }
            value => Ok(value),
        }
    }

    fn eval_progx(&mut self, obj: Object<'ob>, prog_num: u16) -> Result<Object<'ob>> {
        let mut count = 0;
        let mut returned_form = None;
        for form in obj.as_list()? {
            let value = self.eval_form(form?)?;
            count += 1;
            if prog_num == count {
                returned_form = Some(value);
            }
        }
        returned_form.ok_or_else(|| Error::ArgCount(prog_num, count).into())
    }

    fn eval_progn(&mut self, obj: Object<'ob>) -> Result<Object<'ob>> {
        self.implicit_progn(obj.as_list()?)
    }

    fn eval_while(&mut self, obj: Object<'ob>) -> Result<Object<'ob>> {
        let mut forms = obj.as_list()?;
        let condition = match forms.next() {
            Some(cond) => cond?,
            None => bail!(Error::ArgCount(1, 0)),
        };
        while self.eval_form(condition)? != Object::NIL {
            self.implicit_progn(forms.clone())?;
        }
        Ok(Object::NIL)
    }

    fn eval_cond(&mut self, obj: Object<'ob>) -> Result<Object<'ob>> {
        let mut last = Object::NIL;
        for form in obj.as_list()? {
            let mut clause = (form?).as_list()?;
            let first = clause.next().unwrap_or(Ok(Object::NIL))?;
            last = self.eval_form(first)?;
            if last != Object::NIL {
                if !clause.is_empty() {
                    last = self.implicit_progn(clause)?;
                }
                break;
            }
        }
        Ok(last)
    }

    fn eval_and(&mut self, obj: Object<'ob>) -> Result<Object<'ob>> {
        let mut last = Object::TRUE;
        for form in obj.as_list()? {
            last = self.eval_form(form?)?;
            if last == Object::NIL {
                break;
            }
        }
        Ok(last)
    }

    fn eval_or(&mut self, obj: Object<'ob>) -> Result<Object<'ob>> {
        let mut last = Object::NIL;
        for form in obj.as_list()? {
            last = self.eval_form(form?)?;
            if last != Object::NIL {
                break;
            }
        }
        Ok(last)
    }

    fn eval_if(&mut self, obj: Object<'ob>) -> Result<Object<'ob>> {
        let mut forms = obj.as_list()?;
        let condition = match forms.next() {
            Some(x) => x?,
            None => bail!(Error::ArgCount(2, 0)),
        };
        let true_branch = match forms.next() {
            Some(x) => x?,
            None => bail!(Error::ArgCount(2, 1)),
        };
        #[allow(clippy::if_not_else)]
        if self.eval_form(condition)? != Object::NIL {
            self.eval_form(true_branch)
        } else {
            self.implicit_progn(forms)
        }
    }

    fn setq(&mut self, obj: Object<'ob>) -> Result<Object<'ob>> {
        let mut forms = obj.as_list()?;
        let mut arg_cnt = 0;
        let mut last_value = None;
        loop {
            match Self::pairs(&mut forms)? {
                Some((Object::Symbol(var), Some(val))) => {
                    let val = self.eval_form(val)?;
                    last_value = Some(self.var_set(!var, val));
                }
                Some((other, Some(_))) => bail!(Error::from_object(Type::Symbol, other)),
                Some((_, None)) => bail!(Error::ArgCount(arg_cnt, arg_cnt + 1)),
                None => {
                    break;
                }
            };
            arg_cnt += 2;
        }
        // last_value will be None if forms is empty. In that case throw an error
        last_value.ok_or_else(|| Error::ArgCount(2, 0).into())
    }

    fn pairs(iter: &mut ElemIter<'_, 'ob>) -> Result<Option<(Object<'ob>, Option<Object<'ob>>)>> {
        match iter.next() {
            Some(first) => Ok(Some((first?, iter.next().transpose()?))),
            None => Ok(None),
        }
    }

    fn var_ref(&self, sym: Symbol) -> Result<Object<'ob>> {
        if sym.name.starts_with(':') {
            Ok(sym.into())
        } else {
            let mut iter = self.vars.iter().rev();
            match iter.find_map(|cons| (cons.car() == sym.into()).then(|| cons.cdr())) {
                Some(value) => Ok(value),
                None => match self.env.vars.get(sym) {
                    Some(&v) => Ok(v),
                    None => Err(anyhow!("Void variable: {}", sym)),
                },
            }
        }
    }

    fn var_set(&mut self, name: Symbol, new_value: Object<'ob>) -> Object<'ob> {
        let mut iter = self.vars.iter().rev();
        match iter.find(|cons| (cons.car() == name.into())) {
            Some(value) => {
                value.set_cdr(new_value).expect("env should be mutable");
            }
            None => {
                self.env.vars.insert(name, new_value);
            }
        }
        new_value
    }

    fn quote(value: Object<'ob>) -> Result<Object<'ob>> {
        let mut forms = value.as_list()?;
        match forms.len() {
            1 => Ok(forms.next().unwrap()?),
            x => Err(Error::ArgCount(1, x as u16).into()),
        }
    }

    fn eval_let(&mut self, form: Object<'ob>, parallel: bool) -> Result<Object<'ob>> {
        let mut iter = form.as_list()?;
        let prev_len = self.vars.len();
        match iter.next() {
            // (let x ...)
            Some(x) => {
                if parallel {
                    self.let_bind_parallel(x?)?;
                } else {
                    self.let_bind_serial(x?)?;
                }
            }
            // (let)
            None => bail!(Error::ArgCount(1, 0)),
        }
        let obj = self.implicit_progn(iter)?;
        self.vars.truncate(prev_len);
        Ok(obj)
    }

    fn let_bind_serial(&mut self, form: Object<'ob>) -> Result<()> {
        for binding in form.as_list()? {
            let binding = binding?;
            match binding {
                // (let ((x y)))
                Object::Cons(cons) => {
                    let var = self.let_bind_value(!cons)?;
                    self.vars.push(var);
                }
                // (let (x))
                Object::Symbol(_) => {
                    let val = cons!(binding; self.arena);
                    self.vars.push(val.try_into().unwrap());
                }
                // (let (1))
                x => bail!(Error::from_object(Type::Cons, x)),
            }
        }
        Ok(())
    }

    fn let_bind_parallel(&mut self, form: Object<'ob>) -> Result<()> {
        let mut let_bindings: Vec<&'ob Cons<'ob>> = Vec::new();
        for binding in form.as_list()? {
            let binding = binding?;
            match binding {
                // (let ((x y)))
                Object::Cons(cons) => {
                    let var = self.let_bind_value(!cons)?;
                    let_bindings.push(var);
                }
                // (let (x))
                Object::Symbol(_) => {
                    let val: Object = cons!(binding; self.arena);
                    let_bindings.push(val.try_into().unwrap());
                }
                // (let (1))
                x => bail!(Error::from_object(Type::Cons, x)),
            }
        }
        self.vars.append(&mut let_bindings);
        Ok(())
    }

    fn let_bind_value(&mut self, cons: &'ob Cons<'ob>) -> Result<&'ob Cons<'ob>> {
        let mut iter = cons.cdr().as_list()?;
        let value = match iter.len() {
            // (let ((x)))
            0 => Object::NIL,
            // (let ((x y)))
            1 => self.eval_form(iter.next().unwrap()?)?,
            // (let ((x y z ..)))
            _ => bail!("Let binding forms can only have 1 value"),
        };
        let name: Symbol = cons.car().try_into()?;
        let val = cons!(name, value; self.arena);
        Ok(val.try_into().unwrap())
    }

    fn implicit_progn(&mut self, forms: ElemIter<'_, 'ob>) -> Result<Object<'ob>> {
        let mut last = Object::NIL;
        for form in forms {
            last = self.eval_form(form?)?;
        }
        Ok(last)
    }
}

fn eval_function_body<'ob, 'brw>(
    forms: ElemIter<'_, 'ob>,
    vars: Vec<&'ob Cons<'ob>>,
    env: &'brw mut Environment<'ob>,
    arena: &'ob Arena,
) -> Result<Object<'ob>> {
    let mut call_frame = Interpreter { vars, env, arena };
    call_frame.implicit_progn(forms)
}

defsubr!(eval);

#[cfg(test)]
mod test {
    use crate::symbol::intern;

    use super::*;

    macro_rules! check_interpreter {
        ($compare:expr, $expect:expr) => {{
            let comp_arena = &Arena::new();
            let comp_env = &mut Environment::default();
            println!("Test String: {}", $compare);
            let obj = crate::reader::read($compare, comp_arena).unwrap().0;
            let expect: Object = comp_arena.add($expect);
            assert_eq!(eval(obj, None, comp_env, comp_arena).unwrap(), expect);
        }};
    }

    #[test]
    fn basic() {
        let arena = &Arena::new();
        check_interpreter!("1", 1);
        check_interpreter!("1.5", 1.5);
        check_interpreter!("nil", false);
        check_interpreter!("t", true);
        check_interpreter!("\"foo\"", "foo");
        check_interpreter!("'(1 2)", list!(1, 2; arena));
    }

    #[test]
    fn variables() {
        check_interpreter!("(let ())", false);
        check_interpreter!("(let (x) x)", false);
        check_interpreter!("(let ((x 1)) x)", 1);
        check_interpreter!("(let ((x 1)))", false);
        check_interpreter!("(let ((x 1) (y 2)) x y)", 2);
        check_interpreter!("(let ((x 1)) (let ((x 3)) x))", 3);
        check_interpreter!("(let ((x 1)) (let ((y 3)) x))", 1);
        check_interpreter!("(let ((x 1)) (setq x 2) x)", 2);
        check_interpreter!("(let* ())", false);
        check_interpreter!("(let* ((x 1) (y x)) y)", 1);
    }

    #[test]
    fn conditionals() {
        check_interpreter!("(if nil 1)", false);
        check_interpreter!("(if t 1)", 1);
        check_interpreter!("(if nil 1 2)", 2);
        check_interpreter!("(if t 1 2)", 1);
        check_interpreter!("(if nil 1 2 3)", 3);
        check_interpreter!("(and)", true);
        check_interpreter!("(and 1)", 1);
        check_interpreter!("(and 1 2)", 2);
        check_interpreter!("(and 1 nil)", false);
        check_interpreter!("(and nil 1)", false);
        check_interpreter!("(or)", false);
        check_interpreter!("(or nil)", false);
        check_interpreter!("(or nil 1)", 1);
        check_interpreter!("(or 1 2)", 1);
        check_interpreter!("(cond)", false);
        check_interpreter!("(cond nil)", false);
        check_interpreter!("(cond (1))", 1);
        check_interpreter!("(cond (1 2))", 2);
        check_interpreter!("(cond (nil 1) (2 3))", 3);
        check_interpreter!("(cond (nil 1) (2 3) (4 5))", 3);
    }

    #[test]
    fn special_forms() {
        check_interpreter!("(prog1 1 2 3)", 1);
        check_interpreter!("(prog2 1 2 3)", 2);
        check_interpreter!("(progn 1 2 3 4)", 4);
        check_interpreter!("(function 1)", 1);
        check_interpreter!("(quote 1)", 1);
        check_interpreter!("(if 1 2 3)", 2);
        check_interpreter!("(if nil 2 3)", 3);
        check_interpreter!("(if (and 1 nil) 2 3)", 3);
    }

    #[test]
    fn test_functions() {
        let arena = &Arena::new();
        check_interpreter!(
            "(function (lambda))",
            list![&sym::CLOSURE, list![true; arena]; arena]
        );
        let x = intern("x");
        let y = intern("y");
        check_interpreter!(
            "(function (lambda (x) x))",
            list![&sym::CLOSURE, list![true; arena], list![x; arena], x; arena]
        );
        check_interpreter!(
            "(let ((y 1)) (function (lambda (x) x)))",
            list![&sym::CLOSURE, list![cons!(y, 1; arena), true; arena], list![x; arena], x; arena]
        );

        check_interpreter!(
            "(let ((x #'(lambda (x &optional y &rest z) (cons x (cons y z))))) (funcall x 5))",
            list!(5, false; arena)
        );
        check_interpreter!(
            "(let ((x #'(lambda (x &optional y &rest z) (cons x (cons y z))))) (funcall x 5 7))",
            list!(5, 7; arena)
        );
        check_interpreter!(
            "(let ((x #'(lambda (x &optional y &rest z) (cons x (cons y z))))) (funcall x 5 7 11))",
            list!(5, 7, 11; arena)
        );
    }

    #[test]
    fn test_call() {
        check_interpreter!("(let ((x #'(lambda (x) x))) (funcall x 5))", 5);
        check_interpreter!("(let ((x #'(lambda () 3))) (funcall x))", 3);
        check_interpreter!(
            "(progn (defalias 'int-test-call #'(lambda (x) (+ x 3)))  (int-test-call 7))",
            10
        );
        // Test closures
        check_interpreter!("(let* ((y 7)(x #'(lambda () y))) (funcall x))", 7);
        check_interpreter!("(let* ((y 7)(x #'(lambda (x) (+ x y)))) (funcall x 3))", 10);
        // Test that closures capture their environments
        check_interpreter!(
            "(progn (setq func (let ((x 3)) #'(lambda (y) (+ y x)))) (funcall func 5))",
            8
        );
        // Test multiple closures
        check_interpreter!("(progn (setq funcs (let ((x 3)) (cons #'(lambda (y) (+ y x)) #'(lambda (y) (- y x))))) (* (funcall (car funcs) 5) (funcall (cdr funcs) 1)))", -16);
        // Test that closures close over variables
        check_interpreter!("(progn (setq funcs (let ((x 3)) (cons #'(lambda (y) (setq x y)) #'(lambda (y) (+ y x))))) (funcall (car funcs) 5) (funcall (cdr funcs) 4))", 9);
        // Test that closures in global function close over values and not
        // variables
        check_interpreter!("(progn (setq func (let ((x 3)) (defalias 'int-test-no-cap #'(lambda (y) (+ y x))) #'(lambda (y) (setq x y)))) (funcall func 4) (int-test-no-cap 5))", 8);
    }
}