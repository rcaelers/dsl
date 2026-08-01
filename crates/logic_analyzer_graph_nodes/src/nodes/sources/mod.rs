//! Concrete capture source graph nodes.

pub(crate) mod dslogic_u3pro16;
mod file_source;
mod metadata;
mod sigrok_file_source;
#[cfg(test)]
mod test_capture_source;
#[cfg(test)]
mod test_uart_source;
