use std::path::Path;

use logic_analyzer_processing::nodes::sources::sigrok_file::SigrokFileSource;

pub(crate) fn channel_names(path: &Path) -> Result<Option<Vec<String>>, String> {
    SigrokFileSource::new(path)
        .map(|source| Some(source.header().probe_names.clone()))
        .map_err(|error| error.to_string())
}
