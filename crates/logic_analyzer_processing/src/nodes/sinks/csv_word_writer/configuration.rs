/// How a word's value is rendered in the CSV `value` column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CsvValueFormat {
    #[default]
    Decimal,
    /// Uppercase hex, zero-padded to `width` digits.
    Hex { width: usize },
}

/// Platform-neutral CSV-writer configuration.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CsvWordWriterConfig {
    value_format: CsvValueFormat,
    header: Option<String>,
    static_filename: Option<String>,
}

impl CsvWordWriterConfig {
    /// Creates CSV writer configuration.
    ///
    /// # Parameters
    /// - `value_format`: Representation used for numeric words in the `value` column.
    /// - `header`: Optional literal header written to each created file.
    /// - `static_filename`: Optional filename used when no filename input is connected.
    pub fn new(
        value_format: CsvValueFormat,
        header: Option<String>,
        static_filename: Option<String>,
    ) -> Self {
        Self {
            value_format,
            header,
            static_filename,
        }
    }

    /// Returns the numeric-word representation used by the writer.
    pub const fn value_format(&self) -> CsvValueFormat {
        self.value_format
    }

    /// Returns the optional literal CSV header.
    pub fn header(&self) -> Option<&str> {
        self.header.as_deref()
    }

    /// Returns the optional filename used without a filename input.
    pub fn static_filename(&self) -> Option<&str> {
        self.static_filename.as_deref()
    }
}
