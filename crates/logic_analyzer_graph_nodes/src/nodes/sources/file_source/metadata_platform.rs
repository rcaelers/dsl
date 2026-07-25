use std::path::Path;

use logic_analyzer_processing::nodes::sources::dsl_file::DslFileSource;

pub(crate) fn channel_names(path: &Path) -> Result<Option<Vec<String>>, String> {
    DslFileSource::new(path)
        .map(|source| Some(source.header().probe_names.clone()))
        .map_err(|error| error.to_string())
}
