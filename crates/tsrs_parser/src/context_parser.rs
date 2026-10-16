//! Scoped parser context and speculation shared by both frontend representations.
use crate::token_parser::TokenParser;
use tsrs_ast::NodeFlags;

/// Context flags and parser checkpoints used by grammar routines.
/// Implementations restore parser state; allocation rollback follows the storage owner's rules.
pub trait ContextParser<'text>: TokenParser<'text> + Sized {
    type State;

    fn mark(&self) -> Self::State;
    fn rewind(&mut self, state: Self::State);
    fn context_flags(&self) -> NodeFlags;
    fn replace_context_flags(&mut self, flags: NodeFlags);

    #[inline(never)]
    fn look_ahead(&mut self, callback: impl FnOnce(&mut Self) -> bool) -> bool {
        let state = self.mark();
        let result = callback(self);
        self.rewind(state);
        result
    }

    fn set_context_flags(&mut self, flags: NodeFlags, value: bool) {
        if value {
            self.replace_context_flags(self.context_flags() | flags);
        } else {
            self.replace_context_flags(self.context_flags() & !flags);
        }
    }

    fn do_in_context<T>(
        &mut self,
        flags: NodeFlags,
        value: bool,
        f: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let save_context_flags = self.context_flags();
        self.set_context_flags(flags, value);
        let result = f(self);
        self.replace_context_flags(save_context_flags);
        result
    }

    fn in_yield_context(&self) -> bool {
        self.context_flags().intersects(NodeFlags::YieldContext)
    }

    fn in_disallow_in_context(&self) -> bool {
        self.context_flags()
            .intersects(NodeFlags::DisallowInContext)
    }

    fn in_disallow_conditional_types_context(&self) -> bool {
        self.context_flags()
            .intersects(NodeFlags::DisallowConditionalTypesContext)
    }

    fn in_decorator_context(&self) -> bool {
        self.context_flags().intersects(NodeFlags::DecoratorContext)
    }

    fn in_await_context(&self) -> bool {
        self.context_flags().intersects(NodeFlags::AwaitContext)
    }
}

impl ContextParser<'static> for crate::Parser {
    type State = crate::ParserState;

    fn mark(&self) -> Self::State {
        crate::Parser::mark(self)
    }
    fn rewind(&mut self, state: Self::State) {
        crate::Parser::rewind(self, state);
    }
    fn context_flags(&self) -> NodeFlags {
        self.context_flags
    }
    fn replace_context_flags(&mut self, flags: NodeFlags) {
        self.context_flags = flags;
    }
}
