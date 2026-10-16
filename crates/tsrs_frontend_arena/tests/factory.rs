use tsrs_ast::flow::FlowFlags;
use tsrs_ast::{CheckFlags, Kind, ModifierFlags, NodeFlags, SymbolFlags, TokenFlags};
use tsrs_core::{LanguageVariant, ScriptKind, TextRange, arena::Region};
use tsrs_frontend_arena::{FileArena, NodeData, Text};

#[test]
fn direct_construction_preserves_addresses_and_cycles() {
    let arena = FileArena::new();
    let mut addresses = Vec::new();
    let root = arena.build(|f| {
        let name = f.new_identifier(f.text(b"value"));
        let value = f.new_numeric_literal(f.text(b"42"), TokenFlags::None);
        let declaration = f.new_variable_declaration(name, None, None, Some(value));
        declaration.set_loc(TextRange::new(0, 16));
        declaration.set_flags(NodeFlags::ExportContext);
        let symbol = f.alloc_symbol(
            SymbolFlags::FunctionScopedVariable,
            CheckFlags::None,
            name.text(),
            f.nodes(&[declaration]),
            Some(declaration),
            None,
            None,
            None,
            None,
        );
        let table = f.alloc_symbol_table(f.symbol_entries(&[f.symbol_entry(name.text(), symbol)]));
        symbol.set_members(Some(table));
        symbol.set_parent(Some(symbol));
        declaration
            .as_variable_declaration()
            .set_symbol(Some(symbol));

        let flow = f.alloc_flow_node(FlowFlags::Start, Some(declaration), None, None);
        flow.set_antecedent(Some(flow));
        let list = f.alloc_flow_list(flow, None);
        list.set_next(Some(list));
        flow.set_antecedents(Some(list));
        name.as_identifier().set_flow_node(Some(flow));
        name.set_parent(Some(declaration));
        value.set_parent(Some(declaration));
        let doc = f.new_jsdoc(f.new_node_list(&[]), None);
        doc.set_parent(Some(declaration));
        declaration.set_jsdoc(f.nodes(&[doc]));
        let statements = f.new_node_list(&[declaration]);
        statements.set_loc(TextRange::new(0, 16));
        statements.set_has_trailing_comma(true);
        let metadata = f.new_file_metadata(
            f.source("/direct.ts"),
            ScriptKind::TS,
            LanguageVariant::Standard,
        );
        metadata.set_imports(f.nodes(&[name]));
        metadata.set_node_count(5);
        metadata.set_global_exports(Some(table));
        let eof = f.new_token(Kind::EndOfFile);
        let file = f.new_source_file(
            f.source("var value = 42;"),
            f.source("/direct.ts"),
            statements,
            eof,
            metadata,
        );
        file.as_source_file().set_locals(Some(table));
        file.for_each_child(&mut |child| {
            child.set_parent(Some(file));
            false
        });
        addresses.extend([
            std::ptr::from_ref(file).addr(),
            std::ptr::from_ref(file.as_source_file()).addr(),
            std::ptr::from_ref(declaration).addr(),
            std::ptr::from_ref(symbol).addr(),
            std::ptr::from_ref(flow).addr(),
            std::ptr::from_ref(statements).addr(),
            statements.nodes().as_ptr().addr(),
            std::ptr::from_ref(metadata).addr(),
        ]);
        file
    });
    let source = root.source_file().unwrap();
    let declaration = source.statements.nodes[0];
    let symbol = declaration.symbol().unwrap();
    let NodeData::VariableDeclaration(data) = declaration.data else {
        panic!("declaration payload")
    };
    let flow = data.name.flow_node().unwrap();
    assert_eq!(
        addresses,
        [
            std::ptr::from_ref(root).addr(),
            std::ptr::from_ref(source).addr(),
            std::ptr::from_ref(declaration).addr(),
            std::ptr::from_ref(symbol).addr(),
            std::ptr::from_ref(flow).addr(),
            std::ptr::from_ref(source.statements).addr(),
            source.statements.nodes.as_ptr().addr(),
            std::ptr::from_ref(source.metadata).addr(),
        ]
    );
    assert!(std::ptr::eq(declaration.parent.unwrap(), root));
    assert!(std::ptr::eq(data.name.parent.unwrap(), declaration));
    assert!(std::ptr::eq(symbol.declarations[0], declaration));
    assert!(std::ptr::eq(symbol.parent.unwrap(), symbol));
    assert!(std::ptr::eq(
        symbol.members.unwrap().get("value").unwrap(),
        symbol
    ));
    assert!(std::ptr::eq(flow.antecedent.unwrap(), flow));
    let list = flow.antecedents.unwrap();
    assert!(std::ptr::eq(list.next.unwrap(), list));
    assert!(std::ptr::eq(list.flow, flow));
    assert!(std::ptr::eq(
        declaration.jsdoc[0].parent.unwrap(),
        declaration
    ));
    assert!(std::ptr::eq(source.metadata.imports[0], data.name));
    assert!(source.statements.has_trailing_comma);
    assert_eq!(declaration.loc, TextRange::new(0, 16));
    assert_eq!(declaration.flags, NodeFlags::ExportContext);
    std::thread::scope(|s| {
        let first = s.spawn(|| root.locals().unwrap().get("value").unwrap().name);
        let second = s.spawn(|| declaration.symbol().unwrap().name);
        assert_eq!(first.join().unwrap(), second.join().unwrap());
    });
}

#[test]
fn scanner_values_go_directly_into_native_nodes_without_tls() {
    let arena = FileArena::new();
    let region = Region::new(4096);
    let scope = region.enter();
    let mut scanner = arena.scanner(r"'\ud800' '🚀' 1_000");
    let mut text_addresses = Vec::new();
    let root = arena.build(|f| {
        let mut nodes = Vec::new();
        loop {
            let kind = scanner.scan();
            let bytes = scanner.token_value();
            let node = match kind {
                Kind::StringLiteral => {
                    f.new_string_literal(Text::from_bytes(bytes), scanner.token_flags())
                }
                Kind::NumericLiteral => {
                    f.new_numeric_literal(Text::from_bytes(bytes), scanner.token_flags())
                }
                Kind::EndOfFile => break,
                _ => panic!("literal fixture"),
            };
            node.set_loc(scanner.token_range());
            text_addresses.push(bytes.as_ptr().addr());
            nodes.push(node);
        }
        f.new_array_literal_expression(f.new_node_list(&nodes), false)
    });
    assert_eq!(region.used_bytes(), 0);
    drop(scope);
    drop(region);
    drop(scanner);
    let NodeData::ArrayLiteralExpression(array) = root.data else {
        panic!("array payload")
    };
    let nodes = array.elements.nodes;
    assert_eq!(nodes[0].text().as_bytes(), [0xed, 0xa0, 0x80]);
    assert!(nodes[0].text().as_str().is_err());
    assert_eq!(nodes[1].text(), "🚀");
    assert_eq!(nodes[2].text(), "1000");
    assert_eq!(
        text_addresses,
        nodes
            .iter()
            .map(|n| n.text().as_bytes().as_ptr().addr())
            .collect::<Vec<_>>()
    );
}

#[test]
fn setters_lists_and_enum_replacement_freeze_the_final_values() {
    let arena = FileArena::new();
    let root = arena.build(|f| {
        let old = f.new_identifier(f.text(b"old"));
        let replacement = f.new_identifier(f.text(b"new"));
        let expression = f.new_parenthesized_expression(old);
        expression
            .as_parenthesized_expression()
            .set_expression(replacement);
        old.set_data(replacement.data());
        let list = f.new_node_list(&[expression]);
        list.set_nodes(f.nodes(&[old, expression]));
        let modifiers = f.alloc_modifier_list(
            TextRange::new(0, 6),
            f.nodes(&[f.new_token(Kind::ExportKeyword)]),
            false,
            ModifierFlags::Export,
        );
        modifiers.set_has_trailing_comma(true);
        let declaration = f.new_variable_declaration_list(list, NodeFlags::Const);
        let root = f.new_variable_statement(Some(modifiers), declaration);
        root.for_each_child(&mut |child| {
            child.set_parent(Some(root));
            false
        });
        // Force chunk growth while every earlier construction reference remains live.
        for _ in 0..3 {
            f.values(&vec![7u64; 8192]);
        }
        root
    });
    let NodeData::VariableStatement(data) = root.data else {
        panic!("variable statement")
    };
    assert!(data.modifiers.unwrap().has_trailing_comma);
    assert_eq!(data.modifiers.unwrap().flags, ModifierFlags::Export);
    assert_eq!(data.declaration_list.flags, NodeFlags::Const);
    let NodeData::VariableDeclarationList(data) = data.declaration_list.data else {
        panic!("list")
    };
    assert_eq!(data.declarations.nodes[0].text(), "new");
    let NodeData::ParenthesizedExpression(data) = data.declarations.nodes[1].data else {
        panic!("parentheses")
    };
    assert_eq!(data.expression.text(), "new");
    assert!(arena.allocated_bytes() >= 3 * 65536);
}

#[test]
fn abandoned_scopes_and_independent_publications_do_not_leave_mutators() {
    let arena = FileArena::new();
    let first = arena.build(|f| f.new_identifier(f.text(b"first")));
    let canceled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        arena.build(|f| {
            let n = f.new_identifier(f.text(b"canceled"));
            n.set_parent(Some(n));
            panic!("cancel construction")
        });
    }));
    assert!(canceled.is_err());
    let second = arena.build(|f| f.new_identifier(f.text(b"second")));
    assert_eq!(first.text(), "first");
    assert_eq!(second.text(), "second");
    let other = FileArena::new();
    let third = other.build(|f| f.new_identifier(f.text(b"third")));
    let all = [first, second, third];
    assert_eq!(
        all.map(|n| n.text().as_str().unwrap()),
        ["first", "second", "third"]
    );
}

#[test]
fn auxiliary_flow_payloads_and_metadata_entries_keep_their_links() {
    use tsrs_frontend_arena::construction;
    let arena = FileArena::new();
    let root = arena.build(|f| {
        let switch = f.new_switch_statement(
            f.new_identifier(f.text(b"x")),
            f.new_case_block(f.new_node_list(&[])),
        );
        let clause = f.new_node(
            Kind::Unknown,
            construction::NodeData::FlowSwitchClauseData(
                f.alloc_flow_switch_clause_data(switch, 2, 5),
            ),
        );
        let flow = f.alloc_flow_node(FlowFlags::SwitchClause, Some(clause), None, None);
        let antecedents = f.alloc_flow_list(flow, None);
        let reduce = f.new_node(
            Kind::Unknown,
            construction::NodeData::FlowReduceLabelData(
                f.alloc_flow_reduce_label_data(flow, Some(antecedents)),
            ),
        );
        let root_flow = f.alloc_flow_node(FlowFlags::ReduceLabel, Some(reduce), Some(flow), None);
        switch.as_switch_statement().set_flow_node(Some(root_flow));
        let metadata = f.new_file_metadata(
            f.source("/flow.ts"),
            ScriptKind::TS,
            LanguageVariant::Standard,
        );
        let name = f.text(b"package/*");
        let symbol = f.alloc_symbol(
            SymbolFlags::ValueModule,
            CheckFlags::None,
            name,
            f.nodes(&[switch]),
            None,
            None,
            None,
            None,
            None,
        );
        metadata.set_pattern_ambient_modules(f.pattern_modules(&[f.pattern_ambient_module(
            name,
            tsrs_core::PatternKind::Wildcard(8),
            symbol,
        )]));
        metadata.set_ambient_module_names(f.values(&[name]));
        metadata.set_external_module_indicator(Some(switch));
        f.new_source_file(
            f.source(""),
            f.source("/flow.ts"),
            f.new_node_list(&[switch]),
            f.new_token(Kind::EndOfFile),
            metadata,
        )
    });
    let source = root.source_file().unwrap();
    let switch = source.statements.nodes[0];
    let root_flow = switch.flow_node().unwrap();
    let NodeData::FlowReduceLabelData(reduce) = root_flow.node.unwrap().data else {
        panic!("reduce payload")
    };
    assert!(std::ptr::eq(reduce.target, root_flow.antecedent.unwrap()));
    assert!(std::ptr::eq(
        reduce.antecedents.unwrap().flow,
        reduce.target
    ));
    let NodeData::FlowSwitchClauseData(clause) = reduce.target.node.unwrap().data else {
        panic!("clause payload")
    };
    assert!(std::ptr::eq(clause.switch_statement, switch));
    assert_eq!((clause.clause_start, clause.clause_end), (2, 5));
    let pattern = source.metadata.pattern_ambient_modules[0];
    assert_eq!(pattern.text, "package/*");
    assert_eq!(pattern.kind, tsrs_core::PatternKind::Wildcard(8));
    assert!(std::ptr::eq(pattern.symbol.declarations[0], switch));
    assert_eq!(source.metadata.ambient_module_names[0], "package/*");
    assert!(std::ptr::eq(
        source.metadata.external_module_indicator.unwrap(),
        switch
    ));
}
