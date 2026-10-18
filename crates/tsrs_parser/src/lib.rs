pub mod raw;
mod scoped;
pub use scoped::{NodeList, NodeRef, ParseDiagnostic, ParsedFile, parse_source_file_in};
mod jsdoc;
mod parser_1;
mod parser_2;
mod parser_3;
mod references;
mod reparser;
mod types;
mod utilities;

pub use parser_1::{
    JSDocInfo, Parser, ParserState, ParsingContext, ParsingContexts, enable_lazy_dts,
};
pub use types::ParseFlags;
pub use utilities::get_jsdoc_comment_ranges;
