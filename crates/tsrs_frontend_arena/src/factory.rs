use crate::{FileArena, Node, Text, construction as build};
use std::marker::PhantomData;
use tsrs_ast::{Kind, NodeFlags};

// Construction records use actual, invariant borrowed references. Only this factory can create
// them, and only inside build's higher-ranked callback. Cell<T> is transparent; the generated
// assertions cover every field before the publication boundary reinterprets the complete graph.
macro_rules! construction_record {
    ($name:ident, $native:ty, $allocate:ident { $($field:ident, $set:ident: $ty:ty),* $(,)? }) => {
        #[repr(C)]
        pub struct $name<'build, 'ast: 'build> {
            $($field: std::cell::Cell<$ty>,)*
            brand: std::marker::PhantomData<fn(&'build ()) -> &'build ()>,
            arena: std::marker::PhantomData<&'ast ()>,
        }
        impl<'build, 'ast> $name<'build, 'ast> {
            $(pub fn $field(&self) -> $ty { self.$field.get() }
              pub fn $set(&self, value: $ty) { self.$field.set(value); })*
        }
        impl<'build, 'ast> crate::NodeFactory<'build, 'ast> {
            pub fn $allocate(&self, $($field: $ty),*) -> &'build $name<'build, 'ast> {
                self.alloc($name {
                    $($field: std::cell::Cell::new($field),)*
                    brand: std::marker::PhantomData, arena: std::marker::PhantomData,
                })
            }
        }
        const _: () = {
            assert!(std::mem::size_of::<$name<'static, 'static>>() == std::mem::size_of::<$native>());
            assert!(std::mem::align_of::<$name<'static, 'static>>() == std::mem::align_of::<$native>());
            $(assert!(std::mem::offset_of!($name<'static, 'static>, $field) == std::mem::offset_of!($native, $field));)*
        };
    };
}
pub(crate) use construction_record;

// Table entries are immutable values copied into an arena slice. Their referenced symbols remain
// mutable during construction; a table edit replaces the entries slice rather than moving records.
macro_rules! construction_entry {
    ($name:ident, $native:ty, $create:ident { $($field:ident: $ty:ty),* $(,)? }) => {
        #[repr(C)]
        #[derive(Clone, Copy)]
        pub struct $name<'build, 'ast: 'build> {
            $($field: $ty,)*
            brand: std::marker::PhantomData<fn(&'build ()) -> &'build ()>,
            arena: std::marker::PhantomData<&'ast ()>,
        }
        impl<'build, 'ast> $name<'build, 'ast> {
            $(pub fn $field(&self) -> $ty { self.$field })*
        }
        impl<'build, 'ast> crate::NodeFactory<'build, 'ast> {
            pub fn $create(&self, $($field: $ty),*) -> $name<'build, 'ast> {
                $name { $($field,)* brand: std::marker::PhantomData, arena: std::marker::PhantomData }
            }
        }
        const _: () = {
            assert!(std::mem::size_of::<$name<'static, 'static>>() == std::mem::size_of::<$native>());
            assert!(std::mem::align_of::<$name<'static, 'static>>() == std::mem::align_of::<$native>());
            $(assert!(std::mem::offset_of!($name<'static, 'static>, $field) == std::mem::offset_of!($native, $field));)*
        };
    };
}
pub(crate) use construction_entry;

/// A single construction scope. Records borrow the invariant `'build` brand, and text borrows `'ast`.
/// Obtain a factory through `FileArena::build`; it cannot be stored beyond that callback.
pub struct NodeFactory<'build, 'ast: 'build> {
    arena: &'ast FileArena,
    brand: PhantomData<fn(&'build ()) -> &'build ()>,
}

impl FileArena {
    /// Construct a graph in its final arena slots, then publish it without copying. The callback may
    /// mutate construction records; its references cannot escape or mix with a different scope.
    ///
    /// ```
    /// use tsrs_frontend_arena::{FileArena, TextRange};
    /// let arena = FileArena::new();
    /// let node = arena.build(|f| {
    ///     let node = f.new_identifier(f.text(b"name"));
    ///     node.set_loc(TextRange::new(0, 4));
    ///     node
    /// });
    /// assert_eq!(node.text(), "name");
    /// ```
    ///
    /// Construction nodes, payloads, lists, and factories cannot escape the callback.
    ///
    /// ```compile_fail
    /// use tsrs_frontend_arena::{FileArena, TextRange};
    /// let arena = FileArena::new();
    /// let mut saved = None;
    /// let root = arena.build(|f| { let n = f.new_identifier(f.text(b"x")); saved = Some(n); n });
    /// saved.unwrap().set_loc(TextRange::new(0, 1));
    /// println!("{:?}", root.loc);
    /// ```
    ///
    /// ```compile_fail
    /// use tsrs_frontend_arena::FileArena;
    /// let arena = FileArena::new();
    /// let mut saved = None;
    /// arena.build(|f| { let n = f.new_identifier(f.text(b"x")); saved = Some(n.as_identifier()); n });
    /// saved.unwrap().set_text(Default::default());
    /// ```
    ///
    /// ```compile_fail
    /// use tsrs_frontend_arena::FileArena;
    /// let arena = FileArena::new();
    /// let mut saved = None;
    /// arena.build(|f| { let n = f.new_identifier(f.text(b"x")); saved = Some(f.new_node_list(&[n])); n });
    /// saved.unwrap().set_nodes(&[]);
    /// ```
    ///
    /// ```compile_fail
    /// use tsrs_frontend_arena::FileArena;
    /// let arena = FileArena::new();
    /// let mut saved = None;
    /// arena.build(|f| { let n = f.new_identifier(f.text(b"x")); saved = Some(f); n });
    /// saved.unwrap().new_identifier(Default::default());
    /// ```
    ///
    /// Distinct construction scopes cannot share mutable graph edges, even in the same arena.
    ///
    /// ```compile_fail
    /// use tsrs_frontend_arena::FileArena;
    /// let arena = FileArena::new();
    /// arena.build(|outer| {
    ///     let node = outer.new_identifier(outer.text(b"x"));
    ///     arena.build(|inner| inner.new_parenthesized_expression(node));
    ///     node
    /// });
    /// ```
    ///
    /// The long-lived value allocator cannot be used to smuggle construction references out.
    ///
    /// ```compile_fail
    /// use tsrs_frontend_arena::FileArena;
    /// let arena = FileArena::new();
    /// let mut saved = None;
    /// arena.build(|f| { let n = f.new_identifier(f.text(b"x")); saved = Some(f.values(&[n])); n });
    /// saved.unwrap()[0].set_parent(None);
    /// ```
    ///
    /// Copyable table entries carry the same scope restriction.
    ///
    /// ```compile_fail
    /// use tsrs_ast::{CheckFlags, SymbolFlags};
    /// use tsrs_frontend_arena::FileArena;
    /// let arena = FileArena::new();
    /// let mut saved = None;
    /// arena.build(|f| {
    ///     let n = f.new_identifier(f.text(b"x"));
    ///     let symbol = f.alloc_symbol(SymbolFlags::None, CheckFlags::None, n.text(), &[], None, None, None, None, None);
    ///     saved = Some(f.symbol_entry(n.text(), symbol));
    ///     n
    /// });
    /// saved.unwrap().symbol().set_declarations(&[]);
    /// ```
    ///
    /// A graph slice borrowed from another allocator must also survive the published graph.
    ///
    /// ```compile_fail
    /// use tsrs_frontend_arena::FileArena;
    /// struct OtherStorage;
    /// impl OtherStorage {
    ///     fn alloc<T>(&self, value: T) -> &T { Box::leak(Box::new(value)) }
    /// }
    /// let arena = FileArena::new();
    /// let root;
    /// {
    ///     let other = OtherStorage;
    ///     root = arena.build(|f| {
    ///         let node = f.new_identifier(f.text(b"x"));
    ///         node.set_jsdoc(other.alloc([node]));
    ///         node
    ///     });
    /// }
    /// println!("{}", root.jsdoc.len());
    /// ```
    ///
    /// Construction cells cannot be shared with another thread.
    ///
    /// ```compile_fail
    /// use tsrs_frontend_arena::FileArena;
    /// let arena = FileArena::new();
    /// arena.build(|f| {
    ///     let n = f.new_identifier(f.text(b"x"));
    ///     std::thread::scope(|s| { s.spawn(|| n.set_parent(None)).join().unwrap(); });
    ///     n
    /// });
    /// ```
    ///
    /// Published references still borrow their arena and permit no mutation.
    ///
    /// ```compile_fail
    /// use tsrs_frontend_arena::FileArena;
    /// let arena = FileArena::new();
    /// let root = arena.build(|f| f.new_identifier(f.text(b"x")));
    /// drop(arena);
    /// println!("{:?}", root.text());
    /// ```
    ///
    /// ```compile_fail
    /// use tsrs_frontend_arena::{FileArena, TextRange};
    /// let arena = FileArena::new();
    /// let root = arena.build(|f| f.new_identifier(f.text(b"x")));
    /// root.loc = TextRange::new(0, 1);
    /// ```
    pub fn build<'ast>(
        &'ast self,
        construct: impl for<'build> FnOnce(
            NodeFactory<'build, 'ast>,
        ) -> &'build build::Node<'build, 'ast>,
    ) -> &'ast Node<'ast> {
        match self.try_build(|factory| Ok::<_, std::convert::Infallible>(construct(factory))) {
            Ok(root) => root,
            Err(never) => match never {},
        }
    }

    /// Publish only successful construction. Errors may own diagnostics but cannot retain scoped
    /// graph references. Failed allocations are reclaimed when the arena drops, without rewinding.
    ///
    /// ```compile_fail
    /// use tsrs_frontend_arena::FileArena;
    /// let arena = FileArena::new();
    /// let result = arena.try_build(|f| Err(f.new_identifier(f.text(b"escaped"))));
    /// result.err().unwrap().set_parent(None);
    /// ```
    pub fn try_build<'ast, E>(
        &'ast self,
        construct: impl for<'build> FnOnce(
            NodeFactory<'build, 'ast>,
        ) -> Result<&'build build::Node<'build, 'ast>, E>,
    ) -> Result<&'ast Node<'ast>, E> {
        let root = construct(NodeFactory {
            arena: self,
            brand: PhantomData,
        })?;
        // SAFETY: only this scope's factory can create records bearing its invariant brand. All
        // branded record references therefore point to fully initialized slots in this arena.
        // Slices are either factory-allocated or borrowed from a captured owner which the higher-
        // ranked callback requires to live for 'ast; non-branded data explicitly lives for 'ast.
        // Record/enum layouts and field offsets are asserted beside
        // their definitions. The callback has ended and cannot retain construction references or
        // mutation cells. Reinterpreting its root now freezes the reachable graph for 'ast.
        Ok(unsafe { &*std::ptr::from_ref(root).cast::<Node<'ast>>() })
    }
}

impl<'build, 'ast> NodeFactory<'build, 'ast> {
    /// Create parser state in this construction scope. Source and cooked values belong to this arena.
    /// Import `TokenParser`/`NameParser` for migrated grammar, and `ContextParser`/`LookaheadParser`
    /// for parser checkpoints, context flags and syntax decisions.
    ///
    /// ```
    /// use tsrs_frontend_arena::{FileArena, ScriptKind, TokenParser};
    /// let arena = FileArena::new();
    /// let literal = arena.build(|factory| {
    ///     let mut parser = factory.parser("42", ScriptKind::TS);
    ///     parser.next_token();
    ///     parser.parse_literal_expression()
    /// });
    /// assert_eq!(literal.text(), "42");
    /// ```
    ///
    /// ```compile_fail
    /// use tsrs_frontend_arena::{FileArena, ScriptKind, TokenParser};
    /// let arena = FileArena::new();
    /// let mut saved = None;
    /// arena.build(|factory| {
    ///     let mut parser = factory.parser("42", ScriptKind::TS);
    ///     parser.next_token();
    ///     let literal = parser.parse_literal_expression();
    ///     saved = Some(parser);
    ///     literal
    /// });
    /// saved.unwrap().next_token();
    /// ```
    pub fn parser(
        self,
        text: &str,
        script_kind: tsrs_core::ScriptKind,
    ) -> crate::Parser<'build, 'ast> {
        let scanner = self.arena.scanner(text);
        crate::Parser::new(scanner, self, script_kind)
    }

    pub(crate) fn alloc<T>(&self, value: T) -> &'build T {
        self.arena.alloc(value)
    }

    /// Copy non-graph data whose elements already live as long as the published graph.
    pub fn values<T: Copy + 'ast>(&self, values: &[T]) -> &'ast [T] {
        self.arena.slice(values)
    }

    pub fn text(&self, bytes: &[u8]) -> Text<'ast> {
        Text(self.values(bytes))
    }

    pub fn source(&self, text: &str) -> &'ast str {
        self.arena.text(text)
    }

    pub fn nodes(
        &self,
        nodes: &[&'build build::Node<'build, 'ast>],
    ) -> &'build [&'build build::Node<'build, 'ast>] {
        self.arena.slice(nodes)
    }

    pub fn symbol_entries(
        &self,
        entries: &[build::SymbolEntry<'build, 'ast>],
    ) -> &'build [build::SymbolEntry<'build, 'ast>] {
        self.arena.slice(entries)
    }

    pub fn pattern_modules(
        &self,
        entries: &[build::PatternAmbientModule<'build, 'ast>],
    ) -> &'build [build::PatternAmbientModule<'build, 'ast>] {
        self.arena.slice(entries)
    }

    pub fn new_node(
        &self,
        kind: Kind,
        data: build::NodeData<'build, 'ast>,
    ) -> &'build build::Node<'build, 'ast> {
        self.alloc_node(
            kind,
            tsrs_core::undefined_text_range(),
            NodeFlags::None,
            None,
            data,
            &[],
        )
    }

    pub fn new_node_list(
        &self,
        nodes: &[&'build build::Node<'build, 'ast>],
    ) -> &'build build::NodeList<'build, 'ast> {
        self.alloc_node_list(tsrs_core::undefined_text_range(), self.nodes(nodes), false)
    }

    pub fn new_source_file(
        &self,
        text: &'ast str,
        file_name: &'ast str,
        statements: &'build build::NodeList<'build, 'ast>,
        end_of_file_token: &'build build::Node<'build, 'ast>,
        metadata: &'build build::FileMetadata<'build, 'ast>,
    ) -> &'build build::Node<'build, 'ast> {
        self.new_node(
            Kind::SourceFile,
            build::NodeData::SourceFile(self.alloc_source_file(
                statements,
                end_of_file_token,
                None,
                None,
                None,
                text,
                file_name,
                metadata,
            )),
        )
    }

    /// Empty metadata for a new source file. The native parser/binder will fill its fields.
    pub fn new_file_metadata(
        &self,
        path: &'ast str,
        script_kind: tsrs_core::ScriptKind,
        language_variant: tsrs_core::LanguageVariant,
    ) -> &'build build::FileMetadata<'build, 'ast> {
        self.alloc_file_metadata(
            path,
            tsrs_ast::ExternalModuleIndicatorOptions::default(),
            language_variant,
            script_kind,
            false,
            tsrs_core::Tristate::Unknown,
            0,
            0,
            0,
            0,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            None,
            &[],
            None,
            None,
            &[],
            None,
        )
    }
}
