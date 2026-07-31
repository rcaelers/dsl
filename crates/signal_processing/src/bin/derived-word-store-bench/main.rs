//! Generated-workload benchmark for the indexed derived-word store.

std::cfg_select! {
    target_arch = "wasm32" => {
        fn main() {}
    }
    _ => {
        fn main() -> Result<(), Box<dyn std::error::Error>> {
            native::main()
        }

        mod native {
            use std::time::Instant;

            use signal_processing::{IndexedAnnotationWriter, LiveStoreConfig, Word};

            const BATCH_WORDS: usize = 1_048_576;
            const DEFAULT_BATCH_COUNT: usize = 128;
            const MINIMUM_WORDS_PER_SECOND: f64 = 60_000_000.0;
            const TIMESTAMP_CYCLE_NS: u64 = 360;
            const TIMESTAMP_PREFIX_NS: [u64; 4] = [0, 60, 160, 240];

            fn generated_timestamp_ns(position: usize) -> u64 {
                let cycles = position / TIMESTAMP_PREFIX_NS.len();
                let remainder = position % TIMESTAMP_PREFIX_NS.len();
                cycles as u64 * TIMESTAMP_CYCLE_NS + TIMESTAMP_PREFIX_NS[remainder]
            }

            pub(crate) fn main() -> Result<(), Box<dyn std::error::Error>> {
                let directory = tempfile::tempdir()?;
                let batch_count = std::env::var("DERIVED_WORD_STORE_BATCH_COUNT")
                    .ok()
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(DEFAULT_BATCH_COUNT);
                let total_words = BATCH_WORDS * batch_count;
                let config = LiveStoreConfig {
                    directory: directory.path().to_owned(),
                    ..LiveStoreConfig::default()
                };
                let mut words = (0..BATCH_WORDS)
                    .map(|index| Word::new((index & 0xff) as u64, generated_timestamp_ns(index)))
                    .collect::<Vec<_>>();
                let (mut writer, store) = IndexedAnnotationWriter::create(config)?;

                let started = Instant::now();
                for batch in 0..batch_count {
                    for (index, word) in words.iter_mut().enumerate() {
                        word.timestamp_ns =
                            generated_timestamp_ns(batch * BATCH_WORDS + index);
                    }
                    writer.append_batch(&words)?;
                }
                writer.finish()?;
                let elapsed = started.elapsed();
                let throughput = total_words as f64 / elapsed.as_secs_f64();
                let metadata = store.snapshot().metadata;

                println!(
                    "derived-word-store words={total_words} blocks={} bytes={} elapsed_s={:.3} words_s={throughput:.1}",
                    metadata.committed_block_count,
                    metadata.committed_data_len,
                    elapsed.as_secs_f64(),
                );
                assert_eq!(metadata.committed_word_count, total_words as u64);
                assert!(
                    throughput >= MINIMUM_WORDS_PER_SECOND,
                    "derived-word store encoded {throughput:.1} words/s; expected at least \
                     {MINIMUM_WORDS_PER_SECOND:.1}"
                );
                Ok(())
            }
        }
    }
}
