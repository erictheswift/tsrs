//! The only bridge to the legacy construction graph. No construction handle survives `finish()`.
use crate::model::*;
use crate::{FileArena, Text};
use rustc_hash::FxHashMap;
use std::ptr::NonNull;
use tsrs_ast as old;
use tsrs_core::P;

#[derive(Clone, Copy)]
enum Work<'ast> {
    Node(P<old::Node>, NonNull<Node<'ast>>),
    List(P<old::NodeList>, NonNull<NodeList<'ast>>),
    Modifiers(P<old::ModifierList>, NonNull<ModifierList<'ast>>),
    Symbol(P<old::Symbol>, NonNull<Symbol<'ast>>),
    Table(P<old::SymbolTable>, NonNull<SymbolTable<'ast>>),
    Flow(P<old::flow::FlowNode>, NonNull<FlowNode<'ast>>),
    FlowList(P<old::flow::FlowList>, NonNull<FlowList<'ast>>),
}

pub(crate) struct Publisher<'ast> {
    pub(crate) arena: &'ast FileArena,
    pub(crate) source_text: &'ast str,
    source: P<old::SourceFile>,
    work: Vec<Work<'ast>>,
    nodes: FxHashMap<usize, NonNull<Node<'ast>>>,
    lists: FxHashMap<usize, NonNull<NodeList<'ast>>>,
    modifiers: FxHashMap<usize, NonNull<ModifierList<'ast>>>,
    symbols: FxHashMap<usize, NonNull<Symbol<'ast>>>,
    tables: FxHashMap<usize, NonNull<SymbolTable<'ast>>>,
    flows: FxHashMap<usize, NonNull<FlowNode<'ast>>>,
    flow_lists: FxHashMap<usize, NonNull<FlowList<'ast>>>,
    strings: FxHashMap<Vec<u8>, Text<'ast>>,
}

macro_rules! enqueue {
    ($method:ident, $map:ident, $old:ty, $new:ty, $work:ident) => {
        pub(crate) fn $method(&mut self, source: P<$old>) -> NonNull<$new> {
            if let Some(&slot) = self.$map.get(&source.addr()) {
                return slot;
            }
            let slot = self.arena.slot::<$new>();
            self.$map.insert(source.addr(), slot);
            self.work.push(Work::$work(source, slot));
            slot
        }
    };
}

// Both representations are repr(C), have matching field offsets, and use NonNull for each native
// reference (including Option's null niche). Graph references are formed only after the queue drains.
fn write_record<T, R: Copy>(slot: NonNull<T>, raw: R) {
    assert_eq!(std::mem::size_of::<T>(), std::mem::size_of::<R>());
    assert_eq!(std::mem::align_of::<T>(), std::mem::align_of::<R>());
    // SAFETY: all callers use a fresh slot and its paired Raw record, whose field offsets are asserted
    // by record!. Writing the raw representation creates no references to unfinished graph objects.
    unsafe { slot.cast::<R>().as_ptr().write(raw) };
}

impl<'ast> Publisher<'ast> {
    pub(crate) fn new(arena: &'ast FileArena, source: P<old::SourceFile>) -> Self {
        Self {
            arena,
            source,
            source_text: arena.text(source.text()),
            work: Vec::new(),
            nodes: FxHashMap::default(),
            lists: FxHashMap::default(),
            modifiers: FxHashMap::default(),
            symbols: FxHashMap::default(),
            tables: FxHashMap::default(),
            flows: FxHashMap::default(),
            flow_lists: FxHashMap::default(),
            strings: FxHashMap::default(),
        }
    }
    enqueue!(node, nodes, old::Node, Node<'ast>, Node);
    enqueue!(list, lists, old::NodeList, NodeList<'ast>, List);
    enqueue!(modifiers, modifiers, old::ModifierList, ModifierList<'ast>, Modifiers);
    enqueue!(symbol, symbols, old::Symbol, Symbol<'ast>, Symbol);
    enqueue!(table, tables, old::SymbolTable, SymbolTable<'ast>, Table);
    enqueue!(flow, flows, old::flow::FlowNode, FlowNode<'ast>, Flow);
    enqueue!(flow_list, flow_lists, old::flow::FlowList, FlowList<'ast>, FlowList);

    pub(crate) fn nodes(&mut self, source: &[P<old::Node>]) -> &'ast [NonNull<Node<'ast>>] {
        let nodes: Vec<_> = source.iter().map(|&n| self.node(n)).collect();
        self.arena.slice(&nodes)
    }
    pub(crate) fn text(&mut self, source: &str) -> Text<'ast> {
        let offset = source.as_ptr().addr().wrapping_sub(self.source.text().as_ptr().addr());
        if offset.checked_add(source.len()).is_some_and(|end| end <= self.source_text.len()) {
            return Text(&self.source_text.as_bytes()[offset..offset + source.len()]);
        }
        if let Some(&text) = self.strings.get(source.as_bytes()) {
            return text;
        }
        // Legacy cooked literals can contain CESU-8. Copy bytes without forming a new invalid str.
        let text = Text(self.arena.slice(source.as_bytes()));
        self.strings.insert(source.as_bytes().to_vec(), text);
        text
    }
    pub(crate) fn utf8(&mut self, source: &str) -> &'ast str {
        self.text(source).as_str().expect("file metadata is UTF-8 source text")
    }
    pub(crate) fn strings(&mut self, source: &[&str]) -> &'ast [Text<'ast>] {
        let texts: Vec<_> = source.iter().map(|s| self.text(s)).collect();
        self.arena.slice(&texts)
    }

    pub(crate) fn finish(mut self) -> (&'ast Node<'ast>, crate::GraphCounts) {
        let root = self.node(self.source.as_node());
        #[cfg(test)]
        let mut initialized = Vec::new();
        while let Some(work) = self.work.pop() {
            #[cfg(test)]
            initialized.push(work);
            match work {
                Work::Node(source, slot) => {
                    let jsdoc = source.jsdoc(Some(self.source.get()));
                    let raw = RawNode {
                        kind: source.kind(),
                        loc: source.loc(),
                        flags: source.flags(),
                        parent: source.parent().map(|p| self.node(p)),
                        data: self.data(source.data()),
                        jsdoc: self.nodes(jsdoc),
                    };
                    write_record(slot, raw);
                }
                Work::List(source, slot) => {
                    let raw = RawNodeList {
                        loc: source.loc(),
                        nodes: self.nodes(source.nodes()),
                        has_trailing_comma: source.has_trailing_comma(),
                    };
                    write_record(slot, raw);
                }
                Work::Modifiers(source, slot) => {
                    let raw = RawModifierList {
                        loc: source.loc(),
                        nodes: self.nodes(source.nodes()),
                        has_trailing_comma: source.has_trailing_comma(),
                        flags: source.modifier_flags,
                    };
                    write_record(slot, raw);
                }
                Work::Symbol(source, slot) => {
                    let raw = RawSymbol {
                        flags: source.flags(),
                        check_flags: source.check_flags(),
                        name: self.text(source.name()),
                        declarations: self.nodes(source.declarations()),
                        value_declaration: source.value_declaration().map(|n| self.node(n)),
                        parent: source.parent().map(|s| self.symbol(s)),
                        members: source.members().map(|t| self.table(t)),
                        exports: source.exports().map(|t| self.table(t)),
                        export_symbol: source.export_symbol().map(|s| self.symbol(s)),
                    };
                    write_record(slot, raw);
                }
                Work::Table(source, slot) => {
                    let entries: Vec<_> = source
                        .entries()
                        .iter()
                        .map(|(name, symbol)| RawSymbolEntry { name: self.text(name), symbol: self.symbol(*symbol) })
                        .collect();
                    write_record(slot, RawSymbolTable { entries: self.arena.slice(&entries) });
                }
                Work::Flow(source, slot) => {
                    let raw = RawFlowNode {
                        flags: source.flags(),
                        node: source.node().map(|n| self.node(n)),
                        antecedent: source.antecedent().map(|f| self.flow(f)),
                        antecedents: source.antecedents().map(|l| self.flow_list(l)),
                    };
                    write_record(slot, raw);
                }
                Work::FlowList(source, slot) => {
                    let raw = RawFlowList { flow: self.flow(source.flow), next: source.next.get().map(|l| self.flow_list(l)) };
                    write_record(slot, raw);
                }
            }
        }
        #[cfg(test)]
        for work in initialized {
            self.verify_work(work);
        }
        let counts = crate::GraphCounts { nodes: self.nodes.len(), symbols: self.symbols.len(), flows: self.flows.len() };
        // SAFETY: every queued object was initialized, including all transitively reachable edges.
        // Raw/native records have identical repr(C) layouts. All NonNull edges now point at fully
        // initialized native records; all slices/text live in arena. No further writes are possible
        // through the returned API, and its lifetime borrows arena. The legacy region can now die.
        (unsafe { root.as_ref() }, counts)
    }
}

#[cfg(test)]
#[path = "publication_tests.rs"]
mod tests;
