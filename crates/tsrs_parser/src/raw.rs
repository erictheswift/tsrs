//! Compiler entry points for the lifetime-erased AST. Prefer [`crate::parse_source_file_in`]
//! when the result does not need to enter the compiler's manually owned object graph.

pub use crate::parser_1::{
    parse_isolated_entity_name, parse_source_file, parse_source_file_embedded,
    parse_source_file_keep_text, parse_source_file_owned,
};
