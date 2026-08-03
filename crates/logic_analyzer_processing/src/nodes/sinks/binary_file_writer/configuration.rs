/// How a numeric word's value is written to the file. Byte and text words are
/// written in full and do not use this width.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WriteWidth {
    /// Low byte only (`value as u8`).
    #[default]
    U8,
    /// Little-endian 16-bit.
    U16Le,
    /// Little-endian 32-bit.
    U32Le,
}

/// Platform-neutral binary-writer configuration.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BinaryFileWriterConfig {
    width: WriteWidth,
    index_csv: bool,
    static_filename: Option<String>,
}

impl BinaryFileWriterConfig {
    /// Creates binary writer configuration.
    ///
    /// # Parameters
    /// - `width`: Byte order and width used for numeric words.
    /// - `index_csv`: Whether the writer also records completed files in an index CSV.
    /// - `static_filename`: Optional filename used when no filename input is connected.
    pub fn new(width: WriteWidth, index_csv: bool, static_filename: Option<String>) -> Self {
        Self {
            width,
            index_csv,
            static_filename,
        }
    }

    /// Returns the numeric-word byte width and byte order.
    pub const fn width(&self) -> WriteWidth {
        self.width
    }

    /// Returns whether completed output files are appended to an index CSV.
    pub const fn index_csv(&self) -> bool {
        self.index_csv
    }

    /// Returns the optional filename used without a filename input.
    pub fn static_filename(&self) -> Option<&str> {
        self.static_filename.as_deref()
    }
}
