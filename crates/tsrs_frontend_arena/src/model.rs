use crate::Text;
use crate::generated::{NodeData, RawNodeData};
use std::ptr::NonNull;
use tsrs_ast::flow::FlowFlags;
use tsrs_ast::{CheckFlags, Kind, ModifierFlags, NodeFlags, SymbolFlags};
use tsrs_core::TextRange;

// Before publication all graph edges are NonNull values, never references to uninitialized objects.
// Each pair has identical repr(C) field order and layout; publication changes only the interpretation
// of initialized pointers. Keep the checks beside each pair, including every field offset.
macro_rules! record {
    ($name:ident, $raw:ident { $($field:ident: $ty:ty => $raw_ty:ty),* $(,)? }) => {
        #[repr(C)]
        #[derive(Clone, Copy)]
        pub struct $name<'ast> { $(pub $field: $ty,)* }
        #[repr(C)]
        #[derive(Clone, Copy)]
        pub(crate) struct $raw<'ast> { $(pub $field: $raw_ty,)* }
        const _: () = {
            assert!(std::mem::size_of::<$name<'static>>() == std::mem::size_of::<$raw<'static>>());
            assert!(std::mem::align_of::<$name<'static>>() == std::mem::align_of::<$raw<'static>>());
            $(assert!(std::mem::offset_of!($name<'static>, $field) == std::mem::offset_of!($raw<'static>, $field));)*
        };
    };
}
pub(crate) use record;

record!(Node, RawNode {
    kind: Kind => Kind,
    loc: TextRange => TextRange,
    flags: NodeFlags => NodeFlags,
    parent: Option<&'ast Node<'ast>> => Option<NonNull<Node<'ast>>>,
    data: NodeData<'ast> => RawNodeData<'ast>,
    jsdoc: &'ast [&'ast Node<'ast>] => &'ast [NonNull<Node<'ast>>],
});

record!(NodeList, RawNodeList {
    loc: TextRange => TextRange,
    nodes: &'ast [&'ast Node<'ast>] => &'ast [NonNull<Node<'ast>>],
    has_trailing_comma: bool => bool,
    is_missing: bool => bool,
});
record!(ModifierList, RawModifierList {
    loc: TextRange => TextRange,
    nodes: &'ast [&'ast Node<'ast>] => &'ast [NonNull<Node<'ast>>],
    has_trailing_comma: bool => bool,
    flags: ModifierFlags => ModifierFlags,
});
record!(Symbol, RawSymbol {
    flags: SymbolFlags => SymbolFlags,
    check_flags: CheckFlags => CheckFlags,
    name: Text<'ast> => Text<'ast>,
    declarations: &'ast [&'ast Node<'ast>] => &'ast [NonNull<Node<'ast>>],
    value_declaration: Option<&'ast Node<'ast>> => Option<NonNull<Node<'ast>>>,
    parent: Option<&'ast Symbol<'ast>> => Option<NonNull<Symbol<'ast>>>,
    members: Option<&'ast SymbolTable<'ast>> => Option<NonNull<SymbolTable<'ast>>>,
    exports: Option<&'ast SymbolTable<'ast>> => Option<NonNull<SymbolTable<'ast>>>,
    export_symbol: Option<&'ast Symbol<'ast>> => Option<NonNull<Symbol<'ast>>>,
});
record!(SymbolEntry, RawSymbolEntry {
    name: Text<'ast> => Text<'ast>,
    symbol: &'ast Symbol<'ast> => NonNull<Symbol<'ast>>,
});
record!(SymbolTable, RawSymbolTable {
    entries: &'ast [SymbolEntry<'ast>] => &'ast [RawSymbolEntry<'ast>],
});
record!(FlowNode, RawFlowNode {
    flags: FlowFlags => FlowFlags,
    node: Option<&'ast Node<'ast>> => Option<NonNull<Node<'ast>>>,
    antecedent: Option<&'ast FlowNode<'ast>> => Option<NonNull<FlowNode<'ast>>>,
    antecedents: Option<&'ast FlowList<'ast>> => Option<NonNull<FlowList<'ast>>>,
});
record!(FlowList, RawFlowList {
    flow: &'ast FlowNode<'ast> => NonNull<FlowNode<'ast>>,
    next: Option<&'ast FlowList<'ast>> => Option<NonNull<FlowList<'ast>>>,
});
record!(FlowSwitchClauseData, RawFlowSwitchClauseData {
    switch_statement: &'ast Node<'ast> => NonNull<Node<'ast>>,
    clause_start: usize => usize,
    clause_end: usize => usize,
});
record!(FlowReduceLabelData, RawFlowReduceLabelData {
    target: &'ast FlowNode<'ast> => NonNull<FlowNode<'ast>>,
    antecedents: Option<&'ast FlowList<'ast>> => Option<NonNull<FlowList<'ast>>>,
});

impl<'ast> SymbolTable<'ast> {
    pub fn get(&self, name: &str) -> Option<&'ast Symbol<'ast>> {
        self.get_bytes(name.as_bytes())
    }

    pub fn get_bytes(&self, name: &[u8]) -> Option<&'ast Symbol<'ast>> {
        self.entries.iter().find(|e| e.name.as_bytes() == name).map(|e| e.symbol)
    }
}

impl<'ast> Node<'ast> {
    pub fn source_file(&self) -> Option<&'ast crate::generated::SourceFile<'ast>> {
        if let NodeData::SourceFile(file) = self.data { Some(file) } else { None }
    }
}
