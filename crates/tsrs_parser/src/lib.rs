mod jsdoc;
mod parser_1;
mod parser_2;
mod parser_3;
mod references;
mod reparser;
mod types;
mod utilities;

pub use parser_1::{enable_lazy_dts, parse_isolated_entity_name, parse_source_file, parse_source_file_embedded, parse_source_file_keep_text, parse_source_file_owned, JSDocInfo, Parser, ParserState, ParsingContext, ParsingContexts};
pub use types::ParseFlags;
pub use utilities::get_jsdoc_comment_ranges;

#[cfg(feature = "lifetime-prototype")]
pub use parser_1::parse_source_file_for_publication;

pub mod name_parser;
pub mod token_parser;

pub mod context_parser;
pub mod lookahead_parser;

pub mod list_parser;

pub mod import_parser;

pub mod jsx_parser;

pub mod jsdoc_parser;
