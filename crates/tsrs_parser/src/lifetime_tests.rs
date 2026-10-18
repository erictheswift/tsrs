//! Compile-fail checks for the parser ownership boundary.
//!
//! Parsed files cannot outlive the allocator.
//!
//! ```compile_fail,E0597
//! use tsrs_core::{Allocator, ScriptKind};
//! use tsrs_ast::SourceFileParseOptions;
//! use tsrs_parser::parse_source_file_in;
//! let file = {
//!     let allocator = Allocator::current();
//!     parse_source_file_in(&allocator, SourceFileParseOptions {
//!         file_name: "/a.ts".into(), ..Default::default()
//!     }, "let value = 1", ScriptKind::TS)
//! };
//! println!("{}", file.text());
//! ```
//!
//! Borrowed source text cannot outlive the allocator.
//!
//! ```compile_fail,E0597
//! use tsrs_core::{Allocator, ScriptKind};
//! use tsrs_ast::SourceFileParseOptions;
//! use tsrs_parser::parse_source_file_in;
//! let text = {
//!     let allocator = Allocator::current();
//!     parse_source_file_in(&allocator, SourceFileParseOptions {
//!         file_name: "/a.ts".into(), ..Default::default()
//!     }, "let value = 1", ScriptKind::TS).text()
//! };
//! println!("{text}");
//! ```
//!
//! Node lists cannot outlive the allocator.
//!
//! ```compile_fail,E0597
//! use tsrs_core::{Allocator, ScriptKind};
//! use tsrs_ast::SourceFileParseOptions;
//! use tsrs_parser::parse_source_file_in;
//! let statements = {
//!     let allocator = Allocator::current();
//!     parse_source_file_in(&allocator, SourceFileParseOptions {
//!         file_name: "/a.ts".into(), ..Default::default()
//!     }, "let value = 1", ScriptKind::TS).statements()
//! };
//! println!("{}", statements.len());
//! ```
//!
//! Scoped nodes cannot expose raw compiler handles.
//!
//! ```compile_fail,E0599
//! use tsrs_core::{Allocator, ScriptKind};
//! use tsrs_ast::SourceFileParseOptions;
//! use tsrs_parser::parse_source_file_in;
//! let allocator = Allocator::current();
//! let file = parse_source_file_in(&allocator, SourceFileParseOptions {
//!     file_name: "/a.ts".into(), ..Default::default()
//! }, "let value = 1", ScriptKind::TS);
//! let raw = file.root().get();
//! ```
//!
//! Unbound factories cannot accept temporary text.
//!
//! ```compile_fail,E0597
//! let factory = tsrs_ast::NodeFactory::default();
//! let node = {
//!     let text = String::from("temporary");
//!     factory.new_identifier(&text)
//! };
//! println!("{}", node.text());
//! ```
//!
//! Individual node views cannot outlive the allocator.
//!
//! ```compile_fail,E0597
//! use tsrs_core::{Allocator, ScriptKind};
//! use tsrs_ast::SourceFileParseOptions;
//! use tsrs_parser::parse_source_file_in;
//! let node = {
//!     let allocator = Allocator::current();
//!     parse_source_file_in(&allocator, SourceFileParseOptions {
//!         file_name: "/a.ts".into(), ..Default::default()
//!     }, "let value = 1", ScriptKind::TS).root()
//! };
//! println!("{:?}", node.kind());
//! ```
//!
//! Factory text allocation cannot return a static reference.
//!
//! ```compile_fail,E0597
//! let text = {
//!     let allocator = tsrs_core::Allocator::current();
//!     // No raw AST handles are created or retained.
//!     let factory = unsafe { tsrs_ast::NodeFactory::new_in(&allocator) };
//!     factory.alloc_text("temporary")
//! };
//! println!("{text}");
//! ```
//!
//! Packed text and generated payloads retain their input borrow.
//!
//! ```compile_fail,E0597
//! let allocator = tsrs_core::Allocator::current();
//! let payload = {
//!     let text = String::from("temporary");
//!     tsrs_ast::LiteralLikeNodeBaseStorage {
//!         text: tsrs_core::PackedStr::new_in(&allocator, &text),
//!         token_flags: tsrs_core::OwnedCell::new(tsrs_ast::TokenFlags::None),
//!     }
//! };
//! println!("{}", payload.text());
//! ```
//!
//! Packed text cannot outlive its allocator, even if its input is static.
//!
//! ```compile_fail,E0597
//! let packed = {
//!     let allocator = tsrs_core::Allocator::current();
//!     tsrs_core::PackedStr::new_in(&allocator, "text")
//! };
//! println!("{}", packed.as_str());
//! ```
//!
//! Packed lists retain both the allocator and borrowed elements.
//!
//! ```compile_fail,E0597
//! let packed = {
//!     let allocator = tsrs_core::Allocator::current();
//!     tsrs_core::ThinSlice::new_in(&allocator, &[1_u16, 2])
//! };
//! println!("{:?}", packed.get());
//! ```
//!
//! ```compile_fail,E0597
//! let allocator = tsrs_core::Allocator::current();
//! let values;
//! let packed = {
//!     let text = String::from("temporary");
//!     values = [text.as_str()];
//!     tsrs_core::ThinSlice::new_in(&allocator, &values)
//! };
//! println!("{:?}", packed.get());
//! ```
//!
//! Node-list storage cannot outlive its backing slice.
//!
//! ```compile_fail,E0597
//! let allocator = tsrs_core::Allocator::current();
//! let list = {
//!     let nodes = Vec::new();
//!     tsrs_ast::NodeListStorage::new_in(
//!         &allocator, tsrs_core::undefined_text_range(), &nodes,
//!     )
//! };
//! println!("{}", list.nodes().len());
//! ```
