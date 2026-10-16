use crate::model::*;
use crate::publish::Publisher;
use std::ptr::NonNull;
use tsrs_core::{P, TextRange};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileReference<'ast> {
    pub loc: TextRange,
    pub file_name: &'ast str,
    pub resolution_mode: tsrs_core::ResolutionMode,
    pub preserve: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PragmaArgument<'ast> {
    pub key: &'ast str,
    pub loc: TextRange,
    pub name: &'ast str,
    pub value: &'ast str,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pragma<'ast> {
    pub range: tsrs_ast::CommentRange,
    pub name: &'ast str,
    pub arguments: &'ast [PragmaArgument<'ast>],
}
record!(PatternAmbientModule, RawPatternAmbientModule {
    text: crate::Text<'ast> => crate::Text<'ast>,
    kind: tsrs_core::PatternKind => tsrs_core::PatternKind,
    symbol: &'ast Symbol<'ast> => NonNull<Symbol<'ast>>,
});
record!(FileMetadata, RawFileMetadata {
    path: &'ast str => &'ast str,
    external_module_options: tsrs_ast::ExternalModuleIndicatorOptions => tsrs_ast::ExternalModuleIndicatorOptions,
    language_variant: tsrs_core::LanguageVariant => tsrs_core::LanguageVariant,
    script_kind: tsrs_core::ScriptKind => tsrs_core::ScriptKind,
    is_declaration_file: bool => bool,
    uses_uri_style_node_core_modules: tsrs_core::Tristate => tsrs_core::Tristate,
    identifier_count: usize => usize,
    node_count: usize => usize,
    text_count: usize => usize,
    symbol_count: usize => usize,
    imports: &'ast [&'ast Node<'ast>] => &'ast [NonNull<Node<'ast>>],
    module_augmentations: &'ast [&'ast Node<'ast>] => &'ast [NonNull<Node<'ast>>],
    ambient_module_names: &'ast [crate::Text<'ast>] => &'ast [crate::Text<'ast>],
    comment_directives: &'ast [tsrs_ast::CommentDirective] => &'ast [tsrs_ast::CommentDirective],
    pragmas: &'ast [Pragma<'ast>] => &'ast [Pragma<'ast>],
    referenced_files: &'ast [FileReference<'ast>] => &'ast [FileReference<'ast>],
    type_reference_directives: &'ast [FileReference<'ast>] => &'ast [FileReference<'ast>],
    lib_reference_directives: &'ast [FileReference<'ast>] => &'ast [FileReference<'ast>],
    check_js_directive: Option<tsrs_ast::CheckJsDirective> => Option<tsrs_ast::CheckJsDirective>,
    reparsed_clones: &'ast [&'ast Node<'ast>] => &'ast [NonNull<Node<'ast>>],
    common_js_module_indicator: Option<&'ast Node<'ast>> => Option<NonNull<Node<'ast>>>,
    external_module_indicator: Option<&'ast Node<'ast>> => Option<NonNull<Node<'ast>>>,
    pattern_ambient_modules: &'ast [PatternAmbientModule<'ast>] => &'ast [RawPatternAmbientModule<'ast>],
    global_exports: Option<&'ast SymbolTable<'ast>> => Option<NonNull<SymbolTable<'ast>>>,
});

impl<'ast> Publisher<'ast> {
    fn references(&mut self, items: &[P<tsrs_ast::FileReference>]) -> &'ast [FileReference<'ast>] {
        let items: Vec<_> = items
            .iter()
            .map(|r| FileReference {
                loc: r.text_range,
                file_name: self.utf8(&r.file_name),
                resolution_mode: r.resolution_mode,
                preserve: r.preserve,
            })
            .collect();
        self.arena.slice(&items)
    }

    pub(crate) fn metadata(&mut self, file: &tsrs_ast::SourceFile) -> NonNull<FileMetadata<'ast>> {
        let pragmas: Vec<_> = file
            .pragmas
            .get()
            .iter()
            .map(|p| {
                let args: Vec<_> = p
                    .args
                    .iter()
                    .map(|(key, a)| PragmaArgument {
                        key: self.utf8(key),
                        loc: a.text_range,
                        name: self.utf8(&a.name),
                        value: self.utf8(&a.value),
                    })
                    .collect();
                Pragma { range: p.comment_range, name: self.utf8(&p.name), arguments: self.arena.slice(&args) }
            })
            .collect();
        let patterns: Vec<_> = file
            .pattern_ambient_modules
            .get()
            .iter()
            .map(|p| RawPatternAmbientModule { text: self.text(&p.pattern.text), kind: p.pattern.kind, symbol: self.symbol(p.symbol) })
            .collect();
        let data = RawFileMetadata {
            path: self.utf8(file.parse_options().path.as_str()),
            external_module_options: file.parse_options().external_module_indicator_options,
            language_variant: file.language_variant.get(),
            script_kind: file.script_kind.get(),
            is_declaration_file: file.is_declaration_file.get(),
            uses_uri_style_node_core_modules: file.uses_uri_style_node_core_modules.get(),
            identifier_count: file.identifier_count.get(),
            node_count: file.node_count.get(),
            text_count: file.text_count.get(),
            symbol_count: file.symbol_count.get(),
            imports: self.nodes(file.imports.get()),
            module_augmentations: self.nodes(file.module_augmentations.get()),
            ambient_module_names: self.strings(file.ambient_module_names.get()),
            comment_directives: self.arena.slice(file.comment_directives.get()),
            pragmas: self.arena.slice(&pragmas),
            referenced_files: self.references(file.referenced_files.get()),
            type_reference_directives: self.references(file.type_reference_directives.get()),
            lib_reference_directives: self.references(file.lib_reference_directives.get()),
            check_js_directive: file.check_js_directive.get().map(|d| *d),
            reparsed_clones: self.nodes(file.reparsed_clones.get()),
            common_js_module_indicator: file.common_js_module_indicator.get().map(|n| self.node(n)),
            external_module_indicator: file.external_module_indicator.get().map(|n| self.node(n)),
            pattern_ambient_modules: self.arena.slice(&patterns),
            global_exports: file.global_exports.get().map(|t| self.table(t)),
        };
        NonNull::from(self.arena.alloc(data)).cast()
    }
}
