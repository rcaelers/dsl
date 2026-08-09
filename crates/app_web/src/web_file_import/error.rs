use platform_artifacts::SourceReadError;

#[derive(Debug, thiserror::Error)]
pub(crate) enum BrowserFileRegistryError {
    #[error("'{display_name}' is too large for the current browser importer ({max_mib} MiB limit)")]
    FileTooLarge {
        display_name: String,
        max_mib: usize,
    },
    #[error("'{display_name}' exceeds the browser importer address space")]
    AddressSpaceOverflow { display_name: String },
    #[error("could not prepare browser import: {0}")]
    Source(#[source] SourceReadError),
    #[error("the browser capture import budget is full ({max_mib} MiB limit)")]
    SessionBudgetFull { max_mib: usize },
    #[error("browser capture reference space is exhausted")]
    ReferenceExhausted,
    #[error("browser capture '{reference}' is already registered")]
    DuplicateReference { reference: String },
    #[error(
        "browser capture '{reference}' is not available in this session; select the file again"
    )]
    UnavailableReference { reference: String },
}

impl From<SourceReadError> for BrowserFileRegistryError {
    fn from(error: SourceReadError) -> Self {
        Self::Source(error)
    }
}
