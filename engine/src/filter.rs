//! Each AST expression node gets compiled into a CompiledExpr or a CompiledValueExpr.
//! Therefore, Filter essentialy is a public API facade for a tree of Compiled(Value)Exprs.
//! When filter gets executed it calls `execute` method on its root expression which then
//! under the hood propagates field values to its leafs by recursively calling
//! their `execute` methods and aggregating results into a single boolean value
//! as recursion unwinds.

use crate::{
    execution_context::ExecutionContext,
    lhs_types::TypedArray,
    scheme::{Scheme, SchemeMismatchError},
    types::{LhsValue, Type},
};
use std::fmt;

type BoxedClosureToOneBool<'s, U> =
    Box<dyn for<'e> Fn(&'e ExecutionContext<'e, U>) -> bool + Sync + Send + 's>;

/// Boxed closure for [`crate::Expr`] AST node that evaluates to a simple [`bool`].
pub struct CompiledOneExpr<'s, U = ()>(BoxedClosureToOneBool<'s, U>);

impl<U> fmt::Debug for CompiledOneExpr<'_, U> {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_tuple("CompiledOneExpr")
            .field(&((&*self.0) as *const _))
            .finish()
    }
}

impl<'s, U> CompiledOneExpr<'s, U> {
    /// Creates a compiled expression IR from a generic closure.
    pub fn new(
        closure: impl for<'e> Fn(&'e ExecutionContext<'e, U>) -> bool + Sync + Send + 's,
    ) -> Self {
        CompiledOneExpr(Box::new(closure))
    }

    /// Executes the closure against a provided context with values.
    pub fn execute<'e>(&self, ctx: &'e ExecutionContext<'e, U>) -> bool {
        self.0(ctx)
    }

    /// Extracts the underlying boxed closure.
    pub fn into_boxed_closure(self) -> BoxedClosureToOneBool<'s, U> {
        self.0
    }
}

pub(crate) type CompiledVecExprResult = TypedArray<'static, bool>;

type BoxedClosureToVecBool<'s, U> =
    Box<dyn for<'e> Fn(&'e ExecutionContext<'e, U>) -> CompiledVecExprResult + Sync + Send + 's>;

/// Boxed closure for [`crate::Expr`] AST node that evaluates to a list of [`bool`].
pub struct CompiledVecExpr<'s, U = ()>(BoxedClosureToVecBool<'s, U>);

impl<U> fmt::Debug for CompiledVecExpr<'_, U> {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_tuple("CompiledVecExpr")
            .field(&((&*self.0) as *const _))
            .finish()
    }
}

impl<'s, U> CompiledVecExpr<'s, U> {
    /// Creates a compiled expression IR from a generic closure.
    pub fn new(
        closure: impl for<'e> Fn(&'e ExecutionContext<'e, U>) -> CompiledVecExprResult
            + Sync
            + Send
            + 's,
    ) -> Self {
        CompiledVecExpr(Box::new(closure))
    }

    /// Executes the closure against a provided context with values.
    pub fn execute<'e>(&self, ctx: &'e ExecutionContext<'e, U>) -> CompiledVecExprResult {
        self.0(ctx)
    }

    /// Extracts the underlying boxed closure.
    pub fn into_boxed_closure(self) -> BoxedClosureToVecBool<'s, U> {
        self.0
    }
}

/// Enum of boxed closure for [`crate::Expr`] AST nodes.
#[derive(Debug)]
pub enum CompiledExpr<'s, U = ()> {
    /// Variant for [`crate::Expr`] AST node that evaluates to a simple [`bool`].
    One(CompiledOneExpr<'s, U>),
    /// Variant for [`crate::Expr`] AST node that evaluates to a list of [`bool`].
    Vec(CompiledVecExpr<'s, U>),
}

impl<U> CompiledExpr<'_, U> {
    #[cfg(test)]
    pub(crate) fn execute_one<'e>(&self, ctx: &'e ExecutionContext<'e, U>) -> bool {
        match self {
            CompiledExpr::One(one) => one.execute(ctx),
            CompiledExpr::Vec(_) => unreachable!(),
        }
    }

    #[cfg(test)]
    pub(crate) fn execute_vec<'e>(
        &self,
        ctx: &'e ExecutionContext<'e, U>,
    ) -> CompiledVecExprResult {
        match self {
            CompiledExpr::One(_) => unreachable!(),
            CompiledExpr::Vec(vec) => vec.execute(ctx),
        }
    }
}

pub type CompiledValueResult<'a> = Result<LhsValue<'a>, Type>;

impl<'a> From<LhsValue<'a>> for CompiledValueResult<'a> {
    fn from(value: LhsValue<'a>) -> Self {
        Ok(value)
    }
}

impl From<Type> for CompiledValueResult<'_> {
    fn from(ty: Type) -> Self {
        Err(ty)
    }
}

type BoxedClosureToValue<'s, U> =
    Box<dyn for<'e> Fn(&'e ExecutionContext<'e, U>) -> CompiledValueResult<'e> + Sync + Send + 's>;

/// Boxed closure for [`crate::ValueExpr`] AST node that evaluates to an [`LhsValue`].
pub struct CompiledValueExpr<'s, U = ()>(BoxedClosureToValue<'s, U>);

impl<U> fmt::Debug for CompiledValueExpr<'_, U> {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_tuple("CompiledValueExpr")
            .field(&((&*self.0) as *const _))
            .finish()
    }
}

impl<'s, U> CompiledValueExpr<'s, U> {
    /// Creates a compiled expression IR from a generic closure.
    pub fn new(
        closure: impl for<'e> Fn(&'e ExecutionContext<'e, U>) -> CompiledValueResult<'e>
            + Sync
            + Send
            + 's,
    ) -> Self {
        CompiledValueExpr(Box::new(closure))
    }

    /// Executes the closure against a provided context with values.
    pub fn execute<'e>(&self, ctx: &'e ExecutionContext<'e, U>) -> CompiledValueResult<'e> {
        self.0(ctx)
    }

    /// Extracts the underlying boxed closure.
    pub fn into_boxed_closure(self) -> BoxedClosureToValue<'s, U> {
        self.0
    }
}

/// An IR for a compiled filter expression.
///
/// Currently it works by creating and combining boxed untyped closures and
/// performing indirect calls between them, which is fairly cheap, but,
/// surely, not as fast as an inline code with real JIT compilers.
///
/// On the other hand, it's much less risky than allocating, trusting and
/// executing code at runtime, because all the code being executed is
/// already statically generated and verified by the Rust compiler and only the
/// data differs. For the same reason, our "compilation" times are much better
/// than with a full JIT compiler as well.
///
/// In the future the underlying representation might change, but for now it
/// provides the best trade-off between safety and performance of compilation
/// and execution.
pub struct Filter<'s, U = ()> {
    root_expr: CompiledOneExpr<'s, U>,
    scheme: &'s Scheme,
}

impl<U> std::fmt::Debug for Filter<'_, U> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Filter")
            .field("root", &self.root_expr)
            .field("scheme", &(self.scheme as *const _))
            .finish()
    }
}

impl<'s, U> Filter<'s, U> {
    /// Creates a compiled expression IR from a generic closure.
    pub(crate) fn new(root_expr: CompiledOneExpr<'s, U>, scheme: &'s Scheme) -> Self {
        Filter { root_expr, scheme }
    }

    /// Executes a compiled filter expression against a provided context with values.
    pub fn execute<'e>(
        &self,
        ctx: &'e ExecutionContext<'e, U>,
    ) -> Result<bool, SchemeMismatchError> {
        if ctx.scheme() == self.scheme {
            Ok(self.root_expr.execute(ctx))
        } else {
            Err(SchemeMismatchError)
        }
    }
}

/// An IR for a compiled value expression.
pub struct FilterValue<'s, U = ()> {
    root_expr: CompiledValueExpr<'s, U>,
    scheme: &'s Scheme,
}

impl<'s, U> FilterValue<'s, U> {
    /// Creates a compiled expression IR from a generic closure.
    pub(crate) fn new(root_expr: CompiledValueExpr<'s, U>, scheme: &'s Scheme) -> Self {
        FilterValue { root_expr, scheme }
    }

    /// Executes a compiled value expression against a provided context with values.
    pub fn execute<'e>(
        &self,
        ctx: &'e ExecutionContext<'e, U>,
    ) -> Result<Result<LhsValue<'e>, Type>, SchemeMismatchError> {
        if ctx.scheme() == self.scheme {
            Ok(self.root_expr.execute(ctx))
        } else {
            Err(SchemeMismatchError)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Filter, SchemeMismatchError};
    use crate::execution_context::ExecutionContext;

    #[test]
    fn test_scheme_mismatch() {
        let scheme1 = Scheme! { foo: Int }.build();
        let scheme2 = Scheme! { foo: Int, bar: Int }.build();
        let filter = scheme1.parse("foo == 42").unwrap().compile();
        let ctx = ExecutionContext::new(&scheme2);

        assert_eq!(filter.execute(&ctx), Err(SchemeMismatchError));
    }

    #[test]
    fn ensure_send_and_sync() {
        fn is_send<T: Send>() {}
        fn is_sync<T: Sync>() {}

        is_send::<Filter<'_, ExecutionContext<'_>>>();
        is_sync::<Filter<'_, ExecutionContext<'_>>>();
    }
}
