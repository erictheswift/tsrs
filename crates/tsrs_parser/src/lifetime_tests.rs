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
